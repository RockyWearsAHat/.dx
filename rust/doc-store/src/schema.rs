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
//! `sections` and `tokens` are derived read models over the current document versions: they
//! let an agent ask for one section or search the corpus without decompressing whole documents.
//! They are rebuilt on every save, so they cannot disagree with the content.
//!
//! `source_files` and `source_tokens` are derived read models over the source code files
//! indexed by the workspace. They mirror the `documents`/`tokens` pattern: `source_files` holds
//! metadata (path, size, mtime) keyed by path, and `source_tokens` maps source paths to tokens
//! for fast search without decompressing source. Both are built by the indexer and rebuilt on
//! every sync, so they cannot disagree with the actual source files. A read operation over
//! source files must never write to the index — the indexer owns all writes, and reads answer
//! only what the indexer has already recorded.
//!
//! # Why no refcount column
//! An unreferenced chunk is found by asking, not by bookkeeping: `collect_garbage` deletes the
//! chunks no `manifest_chunks` row points at. A counter would be a second source of truth that
//! could drift from the join table; a query cannot.

use std::path::Path;

use rusqlite::Connection;

use crate::StoreError;

/// Current schema version. A database at an older version is rebuilt (see [`apply`]).
///
/// It rises for a change of *meaning* as well as of shape: version 3 carries the same tables
/// as 2, but `tokens` now holds stemmed words and identifier parts
/// ([`doc_core::search`]), and a row written by the older tokeniser would narrow a search to
/// the wrong documents. Derived data that no longer matches how it is queried is stale data.
/// Version 4 adds `source_files` and `source_tokens` tables to index source code without
/// writing to the schema on every read.
/// Version 5 introduces `.doc/source_index` as a committed artifact for source code search data.
pub const VERSION: i64 = 5;

/// Drop every table this schema owns, in dependency order.
///
/// Used when an older database is opened. The index is a derived artifact — the content lives
/// in the committed packs — so discarding and rebuilding it is safe where migrating an
/// unreleased schema would be pointless ceremony. `Store::sync` repopulates it.
const DROP_ALL: &str = "
DROP TABLE IF EXISTS source_tokens;
DROP TABLE IF EXISTS tokens;
DROP TABLE IF EXISTS sections;
DROP TABLE IF EXISTS documents;
DROP TABLE IF EXISTS source_files;
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

CREATE TABLE IF NOT EXISTS source_files (
    path         TEXT PRIMARY KEY,
    size         INTEGER NOT NULL,
    mtime        TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS source_tokens (
    path         TEXT NOT NULL,
    token        TEXT NOT NULL,
    PRIMARY KEY (path, token),
    FOREIGN KEY (path) REFERENCES source_files(path) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_manifest_chunks_hash ON manifest_chunks(chunk_hash);
CREATE INDEX IF NOT EXISTS idx_documents_digest ON documents(source_digest);
CREATE INDEX IF NOT EXISTS idx_sections_document ON sections(document_id, position);
CREATE INDEX IF NOT EXISTS idx_tokens_token ON tokens(token);
CREATE INDEX IF NOT EXISTS idx_source_tokens_token ON source_tokens(token);
";

/// The schema version recorded in the database at `path`, or `None` when there is no
/// database there.
///
/// Opening a store *migrates* it, and migrating rebuilds derived data — so a read has to be
/// able to ask what version is on disk without becoming a write. A database this build
/// cannot read is reported by its version, never by rewriting it: reading never writes.
///
/// `None` also covers a database this process cannot open at all — a read-only medium that
/// refuses even the WAL sidecar files, which is every run inside the sandbox. The version is
/// unknown there, not old, and the caller that actually opens the store reports what went
/// wrong; calling it stale would turn a mount problem into a silent fall-through.
pub fn version_at(path: &Path) -> Result<Option<i64>, StoreError> {
    if !path.exists() {
        return Ok(None);
    }
    let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY;
    let ask = |connection: &Connection| -> Option<i64> {
        connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .ok()
    };

    if let Ok(connection) = Connection::open_with_flags(path, flags) {
        if let Some(version) = ask(&connection) {
            return Ok(Some(version));
        }
    }
    // A store on read-only media: WAL wants an `-shm` file it cannot create, so read the
    // database as immutable — sound exactly there, because nothing can be writing it.
    let uri = format!(
        "file:{}?immutable=1",
        path.to_string_lossy()
            .replace('?', "%3F")
            .replace('#', "%23")
    );
    let immutable = Connection::open_with_flags(uri, flags | rusqlite::OpenFlags::SQLITE_OPEN_URI);
    Ok(immutable.ok().and_then(|connection| ask(&connection)))
}

/// Open `connection` for use as a document store: enforce foreign keys, pick a durable but
/// fast journal, and migrate the schema up to [`VERSION`].
///
/// This is the **write** path. It may drop and recreate the derived tables, so a caller that
/// is only reading must check [`version_at`] first rather than opening its way through an
/// upgrade.
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
                 ('chunks', 'manifests', 'manifest_chunks', 'documents', 'sections', 'tokens', 'source_files', 'source_tokens')",
                [],
                |row| row.get(0),
            )
            .expect("count");
        assert_eq!(tables, 8);
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

    #[test]
    fn source_tokens_cascade_deletes_when_source_file_is_removed() {
        let connection = Connection::open_in_memory().expect("memory db");
        apply(&connection).expect("migrate");

        connection
            .execute(
                "INSERT INTO source_files (path, size, mtime) VALUES ('src/main.rs', 1024, '2025-01-01T00:00:00Z')",
                [],
            )
            .expect("insert source file");

        connection
            .execute(
                "INSERT INTO source_tokens (path, token) VALUES ('src/main.rs', 'fn')",
                [],
            )
            .expect("insert token");

        // Verify the token was inserted
        let count: i64 = connection
            .query_row(
                "SELECT count(*) FROM source_tokens WHERE path = 'src/main.rs'",
                [],
                |row| row.get(0),
            )
            .expect("count");
        assert_eq!(count, 1);

        // Delete the source file and verify the token is cascade-deleted
        connection
            .execute("DELETE FROM source_files WHERE path = 'src/main.rs'", [])
            .expect("delete source file");

        let orphaned: i64 = connection
            .query_row(
                "SELECT count(*) FROM source_tokens WHERE path = 'src/main.rs'",
                [],
                |row| row.get(0),
            )
            .expect("count");
        assert_eq!(orphaned, 0, "cascade delete did not remove tokens");
    }

    #[test]
    fn source_tokens_rejects_orphan_paths() {
        let connection = Connection::open_in_memory().expect("memory db");
        apply(&connection).expect("migrate");

        let orphan = connection.execute(
            "INSERT INTO source_tokens (path, token) VALUES ('nonexistent.rs', 'fn')",
            [],
        );
        assert!(orphan.is_err(), "foreign key constraint was not enforced");
    }

    #[test]
    fn mtime_is_iso_8601_text() {
        let connection = Connection::open_in_memory().expect("memory db");
        apply(&connection).expect("migrate");

        // Insert with ISO-8601 format and verify it is stored as-is
        let mtime = "2025-01-15T14:30:00Z";
        connection
            .execute(
                "INSERT INTO source_files (path, size, mtime) VALUES ('test.rs', 100, ?1)",
                [mtime],
            )
            .expect("insert");

        let stored: String = connection
            .query_row(
                "SELECT mtime FROM source_files WHERE path = 'test.rs'",
                [],
                |row| row.get(0),
            )
            .expect("query");
        assert_eq!(stored, mtime, "mtime was not stored as ISO-8601");
    }
}
