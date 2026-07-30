//! The SQLite schema and its migration.
//!
//! # Shape
//! Content lives in `chunks`, addressed by the SHA-256 of the chunk's canonical block text
//! and stored `dxz1`-compressed. `documents` names each document; `document_chunks` records
//! which chunks a document is made of, in order. Because the join table holds *positions*
//! into a shared chunk pool, a block that appears in two documents — or survives an edit to
//! its neighbours — is stored exactly once.
//!
//! `sections` and `tokens` are derived read models: they let an agent ask for one section or
//! search the corpus without decompressing whole documents. They are rebuilt from the
//! chunks whenever a document is saved, so they can never disagree with the content.
//!
//! # Why no refcount column
//! An unreferenced chunk is found by asking, not by bookkeeping: `collect_garbage` deletes
//! the chunks no `document_chunks` row points at. A counter would be a second source of
//! truth that could drift from the join table; a query cannot.

use rusqlite::Connection;

use crate::StoreError;

/// Current schema version. `apply` migrates an older database up to it.
pub const VERSION: i64 = 1;

/// Statements that create the version-1 schema.
const V1: &str = "
CREATE TABLE IF NOT EXISTS chunks (
    hash         TEXT PRIMARY KEY,
    bytes        BLOB NOT NULL,
    plain_bytes  INTEGER NOT NULL,
    compressed   INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS documents (
    id            INTEGER PRIMARY KEY,
    path          TEXT NOT NULL UNIQUE,
    title         TEXT NOT NULL,
    summary       TEXT NOT NULL,
    source_digest  TEXT NOT NULL,
    source_bytes  INTEGER NOT NULL,
    local_only    INTEGER NOT NULL DEFAULT 0,
    updated_at    TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS document_chunks (
    document_id  INTEGER NOT NULL,
    position     INTEGER NOT NULL,
    chunk_hash   TEXT NOT NULL,
    PRIMARY KEY (document_id, position),
    FOREIGN KEY (document_id) REFERENCES documents(id) ON DELETE CASCADE,
    FOREIGN KEY (chunk_hash) REFERENCES chunks(hash)
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

CREATE INDEX IF NOT EXISTS idx_document_chunks_hash ON document_chunks(chunk_hash);
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

    connection.execute_batch(V1).map_err(StoreError::backend)?;
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
                 ('chunks', 'documents', 'document_chunks', 'sections', 'tokens')",
                [],
                |row| row.get(0),
            )
            .expect("count");
        assert_eq!(tables, 5);
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
    fn foreign_keys_are_enforced_on_the_connection() {
        let connection = Connection::open_in_memory().expect("memory db");
        apply(&connection).expect("migrate");
        // A chunk reference with no chunk row must be refused.
        connection
            .execute(
                "INSERT INTO documents (path, title, summary, source_digest, source_bytes, \
                 updated_at) VALUES ('a.dx', 't', '', 'd', 1, 'now')",
                [],
            )
            .expect("document");
        let orphan = connection.execute(
            "INSERT INTO document_chunks (document_id, position, chunk_hash) VALUES (1, 0, 'nope')",
            [],
        );
        assert!(orphan.is_err(), "foreign keys were not enforced");
    }
}
