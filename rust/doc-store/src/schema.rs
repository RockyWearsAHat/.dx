//! The SQLite schema and its migration.
//!
//! # Shape
//! Content lives in `chunks`, addressed by the SHA-256 of the chunk's canonical block text
//! and stored as a `dxz` frame ([`doc_core::compress`] — the codec is named in each frame's
//! magic, and old `DXZ1` frames are decoded forever). A **manifest** is a whole document
//! version: it is keyed by
//! the digest of that version's canonical source, and `manifest_chunks` lists the chunks it is
//! made of, in order. `documents` then just names a path and points at the manifest that is
//! current for it.
//!
//! # Why versions are addressed, not paths
//! Two things fall out of keying content by digest rather than by path, and both are needed:
//!
//! - **Git's diff driver works.** `textconv` is handed a *temporary copy* of a blob, not the
//!   file in the workspace, so a path tells it nothing. The pointer's digest is the only
//!   usable key — and looking a version up by digest works for any revision, which is what
//!   makes `git log -p` and `git show` render real documents.
//! - **History is nearly free.** An edited document shares every unchanged block with its
//!   previous version, so keeping the old manifest costs a row per block, not a copy of the
//!   document.
//!
//! `sections` and `tokens` are derived read models over the current version: they let an agent
//! ask for one section or search the corpus without decompressing whole documents. They are
//! rebuilt on every save, so they cannot disagree with the content.
//!
//! # Why no refcount column
//! An unreferenced chunk is found by asking, not by bookkeeping: `collect_garbage` deletes the
//! chunks no `manifest_chunks` row points at. A counter would be a second source of truth that
//! could drift from the join table; a query cannot.

use rusqlite::Connection;

use crate::StoreError;

/// Current schema version. A database at an older version is rebuilt (see [`apply`]).
pub const VERSION: i64 = 2;

/// Drop every table this schema owns, in dependency order.
///
/// Used when an older database is opened. The index is a derived artifact — the content lives
/// in the committed packs — so discarding and rebuilding it is safe where migrating an
/// unreleased schema would be pointless ceremony. `Store::sync` repopulates it.
const DROP_ALL: &str = "
DROP TABLE IF EXISTS tokens;
DROP TABLE IF EXISTS sections;
DROP TABLE IF EXISTS documents;
DROP TABLE IF EXISTS manifest_chunks;
DROP TABLE IF EXISTS manifests;
DROP TABLE IF EXISTS document_chunks;
DROP TABLE IF EXISTS chunks;
";

/// Statements that create the current schema.
const CURRENT: &str = "
CREATE TABLE IF NOT EXISTS chunks (
    hash         TEXT PRIMARY KEY,
    bytes        BLOB NOT NULL,
    plain_bytes  INTEGER NOT NULL,
    compressed   INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS manifests (
    digest        TEXT PRIMARY KEY,
    source_bytes  INTEGER NOT NULL,
    created_at    TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS manifest_chunks (
    digest      TEXT NOT NULL,
    position    INTEGER NOT NULL,
    chunk_hash  TEXT NOT NULL,
    PRIMARY KEY (digest, position),
    FOREIGN KEY (digest) REFERENCES manifests(digest) ON DELETE CASCADE,
    FOREIGN KEY (chunk_hash) REFERENCES chunks(hash)
);

CREATE TABLE IF NOT EXISTS documents (
    id             INTEGER PRIMARY KEY,
    path           TEXT NOT NULL UNIQUE,
    title          TEXT NOT NULL,
    summary        TEXT NOT NULL,
    source_digest  TEXT NOT NULL,
    local_only     INTEGER NOT NULL DEFAULT 0,
    updated_at     TEXT NOT NULL,
    FOREIGN KEY (source_digest) REFERENCES manifests(digest)
);

CREATE TABLE IF NOT EXISTS sections (
    id           INTEGER PRIMARY KEY,
    document_id  INTEGER NOT NULL,
    position     INTEGER NOT NULL,
    slug         TEXT NOT NULL,
    heading      TEXT NOT NULL,
    depth        INTEGER NOT NULL,
    FOREIGN KEY (document_id) REFERENCES documents(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS tokens (
    document_id  INTEGER NOT NULL,
    token        TEXT NOT NULL,
    hits         INTEGER NOT NULL,
    PRIMARY KEY (document_id, token),
    FOREIGN KEY (document_id) REFERENCES documents(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_manifest_chunks_hash ON manifest_chunks(chunk_hash);
CREATE INDEX IF NOT EXISTS idx_documents_digest ON documents(source_digest);
CREATE INDEX IF NOT EXISTS idx_sections_document ON sections(document_id, position);
CREATE INDEX IF NOT EXISTS idx_tokens_token ON tokens(token);
";

/// Open `connection` for use as a document store: enforce foreign keys, pick a durable but
/// fast journal, and migrate the schema up to [`VERSION`].
///
/// `PRAGMA foreign_keys` must be set per connection, not once per database, which is why it
/// belongs here rather than in the DDL.
pub fn apply(connection: &Connection) -> Result<(), StoreError> {
    connection
        .execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;",
        )
        .map_err(StoreError::backend)?;

    let found: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(StoreError::backend)?;

    if found > VERSION {
        return Err(StoreError::Backend(format!(
            "this document store was written by a newer dx (schema {found}, this build \
             understands {VERSION}); upgrade dx to open it"
        )));
    }

    if found > 0 && found < VERSION {
        // An index written by an older dx. Rebuild rather than migrate: it is derived data,
        // and `Store::sync` restores every document from the packs.
        connection
            .execute_batch(DROP_ALL)
            .map_err(StoreError::backend)?;
    }

    connection
        .execute_batch(CURRENT)
        .map_err(StoreError::backend)?;
    connection
        .pragma_update(None, "user_version", VERSION)
        .map_err(StoreError::backend)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_database_gets_the_current_schema() {
        let connection = Connection::open_in_memory().expect("memory db");
        apply(&connection).expect("migrate");

        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("version");
        assert_eq!(version, VERSION);

        let tables: i64 = connection
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name IN \
                 ('chunks', 'manifests', 'manifest_chunks', 'documents', 'sections', 'tokens')",
                [],
                |row| row.get(0),
            )
            .expect("count");
        assert_eq!(tables, 6);
    }

    #[test]
    fn migrating_twice_is_harmless() {
        let connection = Connection::open_in_memory().expect("memory db");
        apply(&connection).expect("first");
        apply(&connection).expect("second");
    }

    #[test]
    fn a_future_schema_is_refused_with_advice_rather_than_corrupted() {
        let connection = Connection::open_in_memory().expect("memory db");
        connection
            .pragma_update(None, "user_version", VERSION + 5)
            .expect("bump");
        let error = apply(&connection).expect_err("should refuse");
        assert!(error.to_string().contains("upgrade dx"), "{error}");
    }

    #[test]
    fn an_older_index_is_rebuilt_rather_than_left_broken() {
        let connection = Connection::open_in_memory().expect("memory db");
        // An index shaped like an earlier release, marked with its version.
        connection
            .execute_batch(
                "CREATE TABLE documents (id INTEGER PRIMARY KEY, path TEXT, source_bytes INTEGER);
                 CREATE TABLE document_chunks (document_id INTEGER, position INTEGER);",
            )
            .expect("old schema");
        connection
            .pragma_update(None, "user_version", 1)
            .expect("mark old");

        apply(&connection).expect("rebuild");

        // The superseded table is gone and the current one is in place.
        let stale: i64 = connection
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name = 'document_chunks'",
                [],
                |row| row.get(0),
            )
            .expect("count");
        assert_eq!(stale, 0, "the old table survived the rebuild");

        // And a current-shaped insert works, which the old `documents` table would have refused.
        connection
            .execute_batch(
                "INSERT INTO manifests (digest, source_bytes, created_at) VALUES ('d', 1, 'now');
                 INSERT INTO documents (path, title, summary, source_digest, updated_at)
                   VALUES ('a.dx', 't', '', 'd', 'now');",
            )
            .expect("current schema is usable");
    }

    #[test]
    fn foreign_keys_are_enforced_on_the_connection() {
        let connection = Connection::open_in_memory().expect("memory db");
        apply(&connection).expect("migrate");
        // A chunk reference with no chunk row must be refused.
        connection
            .execute(
                "INSERT INTO manifests (digest, source_bytes, created_at) \
                 VALUES ('d', 1, 'now')",
                [],
            )
            .expect("manifest");
        let orphan = connection.execute(
            "INSERT INTO manifest_chunks (digest, position, chunk_hash) VALUES ('d', 0, 'nope')",
            [],
        );
        assert!(orphan.is_err(), "foreign keys were not enforced");

        // And a document may not point at a manifest that does not exist.
        let dangling = connection.execute(
            "INSERT INTO documents (path, title, summary, source_digest, updated_at) \
             VALUES ('a.dx', 't', '', 'missing', 'now')",
            [],
        );
        assert!(dangling.is_err(), "a document kept a dangling manifest");
    }
}
