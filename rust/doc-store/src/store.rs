//! [`Store`] — the document authority and the resolver over it.
//!
//! Every read in the platform lands in one of three methods here: [`Store::source`] for
//! canonical text, [`Store::document`] for the parsed model, and [`Store::search`] /
//! [`Store::list`] for finding things. Each one reads from SQLite, never from the `.dx` file,
//! because the `.dx` file is a pointer.
//!
//! Writes go the other way: [`Store::save`] stores the chunks, refreshes the derived read
//! models, exports the pack to make it durable, and then rewrites the stub on disk — in that order,
//! so a crash leaves the pack durable before the pointer that names it is written. A pointer that
//! names content the packs do not yet hold is a signal to re-resolve, never a reason to serve nothing.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use doc_core::chunk::{self, Chunk};
use doc_core::compress::{compress, decompress};
use doc_core::format::{parse, stringify};
use doc_core::model::Document;
use doc_core::render::outline;
use doc_core::search::{
    build_index, distinct_tokens, document_tokens, vocabulary_kin, MAX_LOOSE_VARIANTS,
    MIN_LOOSE_LEN,
};
use rusqlite::{params, Connection, OptionalExtension};

use crate::git::{self, Route};
use crate::{pack, schema, stub, StoreError};

/// Directory holding the store and its packs, relative to the workspace root.
pub(crate) const STORE_DIR: &str = ".doc";
/// The SQLite database file, relative to the workspace root.
const DB_RELATIVE: &str = ".doc/index.db";

/// Directories never walked when discovering documents.
///
/// `fixture`/`fixtures` are here because test fixtures are files whose exact bytes *are*
/// the test — adopting one into the store replaces it with a pointer and silently turns
/// the suite that reads it into a suite that reads pointers.
const SKIPPED_DIRECTORIES: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "build",
    "dist",
    ".venv",
    "__pycache__",
    "fixture",
    "fixtures",
    STORE_DIR,
];

/// Lightweight metadata for a stored document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    /// Workspace-relative `.dx` path.
    pub path: String,
    /// Display title.
    pub title: String,
    /// Summary line, empty when the document has none.
    pub summary: String,
    /// Byte length of the canonical source.
    pub source_bytes: usize,
    /// Whether the document is git-ignored or untracked scratch work.
    pub local_only: bool,
    /// When the document was last written, ISO-8601.
    pub updated_at: String,
}

/// A [`Summary`] paired with its relevance score from [`Store::search`]; higher is a better
/// match. Mirrors [`doc_core::search::ScoredHit`] one layer up, once the store has resolved
/// the path back to metadata — so a caller can tell a strong match from a weak one instead of
/// treating every hit as equally confident.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoredSummary {
    /// The matching document's metadata.
    pub summary: Summary,
    /// Relevance score from the ranking index; higher is a better match.
    pub score: f64,
}

/// What a [`Store::save`] actually changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Saved {
    /// Workspace-relative path written.
    pub path: String,
    /// Digest of the stored canonical source, as recorded in the stub.
    pub digest: String,
    /// Canonical source byte length.
    pub source_bytes: usize,
    /// Chunks the document is made of.
    pub chunks: usize,
    /// Chunks that were new to the store; the rest were already shared with something else.
    pub chunks_added: usize,
    /// Bytes those new chunks occupy compressed.
    pub bytes_added: usize,
}

/// Storage totals, for `dx doctor` and the compaction report.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Stats {
    /// Documents stored.
    pub documents: usize,
    /// Distinct chunks stored.
    pub chunks: usize,
    /// Chunk references across all documents; the excess over `chunks` is what sharing saved.
    pub chunk_references: usize,
    /// Total canonical source bytes across every document.
    pub source_bytes: usize,
    /// Bytes the chunks occupy in the database, compressed.
    pub stored_bytes: usize,
}

impl Stats {
    /// Stored bytes as a percentage of canonical source bytes, or `None` when nothing is
    /// stored. Lower is better.
    #[must_use]
    pub fn compaction_percent(&self) -> Option<f64> {
        if self.source_bytes == 0 {
            return None;
        }
        Some(100.0 * self.stored_bytes as f64 / self.source_bytes as f64)
    }
}

/// Metadata about an unresolved pointer for diagnostic purposes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedPointer {
    /// Path to the `.dx` file.
    pub path: String,
    /// The digest the pointer names (if it could be extracted).
    pub digest: Option<String>,
}

/// What [`Store::sync`] reconciled.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SyncReport {
    /// Plain-text `.dx` files adopted into the store.
    pub ingested: Vec<String>,
    /// Documents whose stub had drifted from the stored content and has been rewritten.
    pub stubs_written: Vec<String>,
    /// Documents recovered from a pack because the database did not have them.
    pub restored: Vec<String>,
    /// Documents that changed path, as `(from, to)`: a pointer arrived somewhere new naming
    /// content the packs already held under another name. `git mv` and a plain `mv` both
    /// leave exactly this, and following it is what keeps the move from being a deletion.
    pub moved: Vec<(String, String)>,
    /// Stubs that could not be resolved from any source, with diagnostic metadata.
    pub unresolved: Vec<String>,
    /// Detailed metadata about each unresolved pointer.
    pub unresolved_details: Vec<UnresolvedPointer>,
    /// Unresolved pointers whose path is force-added but git-ignored — their content stays
    /// in the local pack and cannot reach other machines.
    pub tracked_but_ignored: Vec<String>,
    /// Index rows dropped because their `.dx` file was deleted from the tree.
    pub pruned: Vec<String>,
    /// Pack files whose bytes on disk changed, `.doc/`-relative. A pack can be rewritten with
    /// no document changing at all — an encoding policy that moved, or a file whose git status
    /// did — which is exactly the case a sync has to notice rather than report as clean.
    pub packs_rewritten: Vec<String>,
    /// Chunks deleted because nothing referenced them.
    pub chunks_collected: usize,
    /// `.dx` files still carrying git conflict markers, which a sync refuses to adopt.
    ///
    /// A merge that could not be resolved leaves the document as plain text with `<<<<<<<`
    /// in it ([`crate::merge`]). That text parses — the markers are ordinary body lines —
    /// so adopting it would store both branches' words as one document and lose the fact
    /// that a person still has to choose. It is named and left alone instead.
    pub conflicted: Vec<String>,
}

impl SyncReport {
    /// Whether the sync changed nothing.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.ingested.is_empty()
            && self.stubs_written.is_empty()
            && self.restored.is_empty()
            && self.moved.is_empty()
            && self.unresolved.is_empty()
            && self.tracked_but_ignored.is_empty()
            && self.pruned.is_empty()
            && self.packs_rewritten.is_empty()
            && self.conflicted.is_empty()
            && self.chunks_collected == 0
    }
}

/// Whether the index at `root` was written by an older dx and would be rebuilt on open.
///
/// A reader asks this before opening: [`Store::open`] migrates, and migrating discards the
/// derived tables, so opening a stale index turns a read into a write. `false` when there is
/// no index at all — there is nothing stale about a store that does not exist yet.
///
/// A store from a *newer* dx is not stale, it is unreadable, and [`Store::open`] says so.
pub fn stale_index(root: &Path) -> Result<bool, StoreError> {
    Ok(schema::version_at(&root.join(DB_RELATIVE))?.is_some_and(|found| found < schema::VERSION))
}

/// The document store rooted at a workspace directory.
pub struct Store {
    root: PathBuf,
    connection: Connection,
    /// Set only for the duration of [`Store::sync`]. Every write exports the packs, and the
    /// export refuses to drop a document the tree still points at — which is the right answer
    /// everywhere except inside the repair that is putting those documents back.
    repairing: bool,
    /// During sync/reconcile, stubs that need writing but must be deferred until after
    /// export_packs to ensure durability: pointers are written only after their content
    /// is safely in the durable pack files.
    deferred_stubs: Vec<(String, String)>,
}

impl Store {
    /// An open store over `connection`, in normal (non-repairing) operation.
    #[cfg(test)]
    pub fn connection_mut(&mut self) -> &mut Connection {
        &mut self.connection
    }
    fn over(root: PathBuf, connection: Connection) -> Self {
        Self {
            root,
            connection,
            repairing: false,
            deferred_stubs: Vec::new(),
        }
    }

    /// Open (creating if needed) the store for the workspace rooted at `root`.
    ///
    /// Creates `.doc/` and migrates the schema. Opening does not touch any `.dx` file and
    /// does not execute anything — reading is always free of side effects.
    pub fn open(root: &Path) -> Result<Self, StoreError> {
        let root = root.to_path_buf();
        let directory = root.join(STORE_DIR);
        fs::create_dir_all(&directory).map_err(|error| {
            StoreError::Backend(format!("could not create {}: {error}", directory.display()))
        })?;

        let connection = Connection::open(root.join(DB_RELATIVE)).map_err(StoreError::backend)?;
        schema::apply(&connection)?;
        Ok(Self::over(root, connection))
    }

    /// Open an existing store without the ability to write.
    ///
    /// This is the read path for a store on a medium this process cannot write — a
    /// read-only checkout, a build sandbox, mounted media. Nothing is created and no
    /// schema is applied; the database must already exist. The database is in WAL mode
    /// ([`schema::apply`]), and a read-only filesystem also refuses the `-shm` file WAL
    /// readers need — so a plain read-only open is probed with a real read and, when the
    /// medium refuses even that, the store is opened `immutable`. Immutable is sound
    /// exactly here: on a medium nothing can write, there is no writer to race.
    pub fn open_read_only(root: &Path) -> Result<Self, StoreError> {
        let path = root.join(DB_RELATIVE);
        let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY;

        if let Ok(connection) = Connection::open_with_flags(&path, flags) {
            let probe: Result<i64, _> =
                connection.query_row("SELECT count(*) FROM documents", [], |row| row.get(0));
            if probe.is_ok() {
                return Ok(Self::over(root.to_path_buf(), connection));
            }
        }

        let uri = format!(
            "file:{}?immutable=1",
            path.to_string_lossy()
                .replace('?', "%3F")
                .replace('#', "%23")
        );
        let connection =
            Connection::open_with_flags(uri, flags | rusqlite::OpenFlags::SQLITE_OPEN_URI)
                .map_err(StoreError::backend)?;
        Ok(Self::over(root.to_path_buf(), connection))
    }

    /// Open a store held entirely in memory, for tests and one-shot rendering.
    pub fn open_in_memory(root: &Path) -> Result<Self, StoreError> {
        let connection = Connection::open_in_memory().map_err(StoreError::backend)?;
        schema::apply(&connection)?;
        Ok(Self::over(root.to_path_buf(), connection))
    }

    /// The workspace root this store belongs to.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The absolute path of the `.dx` stub for `relative`.
    #[must_use]
    pub fn stub_path(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }

    /// Normalize a caller-supplied path, or explain why it is unusable.
    fn require_path(path: &str) -> Result<String, StoreError> {
        stub::normalize_path(path).ok_or_else(|| StoreError::InvalidPath(path.to_string()))
    }

    /// The stored document id for `relative`, or `None`.
    fn document_id(&self, relative: &str) -> Result<Option<i64>, StoreError> {
        self.connection
            .query_row(
                "SELECT id FROM documents WHERE path = ?1",
                params![relative],
                |row| row.get(0),
            )
            .optional()
            .map_err(StoreError::backend)
    }

    /// Whether a document is stored at `path`.
    pub fn contains(&self, path: &str) -> Result<bool, StoreError> {
        let relative = Self::require_path(path)?;
        Ok(self.document_id(&relative)?.is_some())
    }

    /// The canonical source of the document at `path`.
    ///
    /// This is the resolver: it reassembles the document from its chunks and verifies the
    /// result against the digest recorded when it was stored. A mismatch is reported as
    /// [`StoreError::Corrupt`] rather than returned, so a damaged store can never be mistaken
    /// for an edited document.
    pub fn source(&self, path: &str) -> Result<String, StoreError> {
        let relative = Self::require_path(path)?;
        let id = self
            .document_id(&relative)?
            .ok_or_else(|| StoreError::NotFound(relative.clone()))?;

        let expected: String = self
            .connection
            .query_row(
                "SELECT source_digest FROM documents WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .map_err(StoreError::backend)?;

        self.source_of_version(&expected)?
            .ok_or_else(|| StoreError::Corrupt(format!("{relative} has no stored content")))
    }

    /// The canonical source of the version whose digest is `digest`, or `None` when this store
    /// has never held it.
    ///
    /// This is the digest-keyed read path, and it is what makes the git diff driver work: git
    /// hands `textconv` a temporary copy of a blob, so the only usable key is the digest written
    /// inside the pointer itself. Because every version ever saved keeps its manifest, this
    /// resolves historical revisions too.
    ///
    /// The result is verified against `digest` before it is returned, so a damaged store is
    /// reported rather than served.
    pub fn source_of_version(&self, digest: &str) -> Result<Option<String>, StoreError> {
        let texts = self.chunk_texts_of(digest)?;
        if texts.is_empty() {
            return Ok(None);
        }
        let source = chunk::join(texts.iter().map(String::as_str));
        let found = stub::digest_of(&source);
        if found != digest {
            return Err(StoreError::Corrupt(format!(
                "version {digest} reassembled to {} bytes with digest {found} instead",
                source.len()
            )));
        }
        Ok(Some(source))
    }

    /// The chunk texts of the version `digest`, in document order, decompressed.
    fn chunk_texts_of(&self, digest: &str) -> Result<Vec<String>, StoreError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT c.bytes, c.compressed, m.chunk_hash
                 FROM manifest_chunks m JOIN chunks c ON c.hash = m.chunk_hash
                 WHERE m.digest = ?1 ORDER BY m.position",
            )
            .map_err(StoreError::backend)?;

        let rows = statement
            .query_map(params![digest], |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, i64>(1)? != 0,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(StoreError::backend)?;

        let mut texts = Vec::new();
        for row in rows {
            let (bytes, compressed, hash) = row.map_err(StoreError::backend)?;
            let plain = if compressed {
                decompress(&bytes).map_err(|error| {
                    StoreError::Corrupt(format!("chunk {hash} would not decompress ({error})"))
                })?
            } else {
                bytes
            };
            texts.push(String::from_utf8(plain).map_err(|_| {
                StoreError::Corrupt(format!("chunk {hash} is not valid UTF-8 text"))
            })?);
        }
        Ok(texts)
    }

    /// The parsed document at `path`.
    pub fn document(&self, path: &str) -> Result<Document, StoreError> {
        Ok(parse(&self.source(path)?))
    }

    /// Store `document` at `path`, rewrite its stub, and export the packs.
    ///
    /// The canonical source is derived from `document`, so a save always lands the document
    /// in canonical form — the property that stops two editors from fighting over formatting.
    ///
    /// A path under a directory [`walk`] never enters is refused
    /// ([`StoreError::Unlistable`]): the row would be stored but no listing could ever
    /// re-find it, which is the ghost-document symptom by another door.
    pub fn save(&mut self, path: &str, document: &Document) -> Result<Saved, StoreError> {
        let relative = Self::require_path(path)?;
        if let Some(directory) = unwalked_directory(&relative) {
            let directory = directory.to_string();
            return Err(StoreError::Unlistable {
                path: relative,
                directory,
            });
        }
        let source = stringify(document);
        let chunks = chunk::split(document);
        let route = git::route(&self.root, &relative);

        let saved = self.write_document(&relative, document, &source, &chunks, route)?;
        // Export packs first to ensure content is durable before the pointer is written.
        // This prevents a crash between pointer write and pack write from leaving a
        // dangling pointer naming content not yet in any pack.
        self.export_packs()?;
        self.export_source_index()?;
        self.write_stub(&relative, &source)?;
        Ok(saved)
    }

    /// Persist a document's rows and chunks inside one transaction.
    fn write_document(
        &mut self,
        relative: &str,
        document: &Document,
        source: &str,
        chunks: &[Chunk],
        route: Route,
    ) -> Result<Saved, StoreError> {
        let digest = stub::digest_of(source);
        let title = document.display_title(relative);
        let now = timestamp();

        let transaction = self.connection.transaction().map_err(StoreError::backend)?;

        // Chunk bodies first, so every reference below already has a target.
        let mut chunks_added = 0;
        let mut bytes_added = 0;
        for chunk in chunks {
            let (bytes, compressed) = encode_chunk(&chunk.text);
            let inserted = transaction
                .execute(
                    "INSERT INTO chunks (hash, bytes, plain_bytes, compressed)
                     VALUES (?1, ?2, ?3, ?4) ON CONFLICT(hash) DO NOTHING",
                    params![
                        chunk.hash,
                        bytes,
                        chunk.text.len() as i64,
                        i64::from(compressed)
                    ],
                )
                .map_err(StoreError::backend)?;
            if inserted > 0 {
                chunks_added += 1;
                bytes_added += bytes.len();
            }
        }

        // The manifest for this exact content. Identical content is the same digest, so
        // re-saving an unchanged document — or reverting to an earlier one — adds nothing.
        let fresh_manifest = transaction
            .execute(
                "INSERT INTO manifests (digest, source_bytes, created_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(digest) DO NOTHING",
                params![digest, source.len() as i64, now],
            )
            .map_err(StoreError::backend)?
            > 0;
        if fresh_manifest {
            for (position, chunk) in chunks.iter().enumerate() {
                transaction
                    .execute(
                        "INSERT INTO manifest_chunks (digest, position, chunk_hash)
                         VALUES (?1, ?2, ?3)",
                        params![digest, position as i64, chunk.hash],
                    )
                    .map_err(StoreError::backend)?;
            }
        }

        transaction
            .execute(
                "INSERT INTO documents
                    (path, title, summary, source_digest, local_only, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(path) DO UPDATE SET
                    title = excluded.title, summary = excluded.summary,
                    source_digest = excluded.source_digest,
                    local_only = excluded.local_only, updated_at = excluded.updated_at",
                params![
                    relative,
                    title,
                    document.summary,
                    digest,
                    i64::from(route.is_local_only()),
                    now
                ],
            )
            .map_err(StoreError::backend)?;

        let id: i64 = transaction
            .query_row(
                "SELECT id FROM documents WHERE path = ?1",
                params![relative],
                |row| row.get(0),
            )
            .map_err(StoreError::backend)?;

        write_read_models(&transaction, id, document)?;
        transaction.commit().map_err(StoreError::backend)?;

        Ok(Saved {
            path: relative.to_string(),
            digest,
            source_bytes: source.len(),
            chunks: chunks.len(),
            chunks_added,
            bytes_added,
        })
    }

    /// Write the stub for `relative` unless the file already says exactly this.
    ///
    /// Skipping an identical write keeps file mtimes — and therefore editor reload storms and
    /// git's idea of what changed — quiet when a save did not alter the content.
    fn write_stub(&self, relative: &str, source: &str) -> Result<bool, StoreError> {
        let path = self.stub_path(relative);
        let wanted = stub::render(source);
        if fs::read_to_string(&path).is_ok_and(|found| found == wanted) {
            return Ok(false);
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                StoreError::Backend(format!("could not create {}: {error}", parent.display()))
            })?;
        }
        fs::write(&path, &wanted).map_err(|error| {
            StoreError::Backend(format!("could not write {}: {error}", path.display()))
        })?;
        Ok(true)
    }

    /// Adopt plain-text `.dx` content at `path` into the store.
    ///
    /// This is how anything that is not `dx` — a text editor, `git checkout`, an agent that
    /// wrote the file directly — gets its work preserved rather than overwritten.
    pub fn ingest(&mut self, path: &str, text: &str) -> Result<Saved, StoreError> {
        self.save(path, &parse(text))
    }

    /// Forget the document at `path` and delete its stub.
    ///
    /// Chunks it no longer shares with anything are collected.
    pub fn delete(&mut self, path: &str) -> Result<(), StoreError> {
        let relative = Self::require_path(path)?;
        let id = self
            .document_id(&relative)?
            .ok_or_else(|| StoreError::NotFound(relative.clone()))?;
        self.connection
            .execute("DELETE FROM documents WHERE id = ?1", params![id])
            .map_err(StoreError::backend)?;
        self.collect_garbage()?;

        let path = self.stub_path(&relative);
        if fs::read_to_string(&path).is_ok_and(|found| stub::is_stub(&found)) {
            fs::remove_file(&path).map_err(|error| {
                StoreError::Backend(format!("could not remove {}: {error}", path.display()))
            })?;
        }
        self.export_packs()?;
        self.export_source_index()
    }

    /// Every stored document, ordered by path.
    pub fn list(&self) -> Result<Vec<Summary>, StoreError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT d.path, d.title, d.summary, m.source_bytes, d.local_only, d.updated_at
                 FROM documents d JOIN manifests m ON m.digest = d.source_digest
                 ORDER BY d.path",
            )
            .map_err(StoreError::backend)?;
        let rows = statement
            .query_map([], |row| {
                Ok(Summary {
                    path: row.get(0)?,
                    title: row.get(1)?,
                    summary: row.get(2)?,
                    source_bytes: row.get::<_, i64>(3)?.max(0) as usize,
                    local_only: row.get::<_, i64>(4)? != 0,
                    updated_at: row.get(5)?,
                })
            })
            .map_err(StoreError::backend)?;
        rows.collect::<Result<Vec<Summary>, _>>()
            .map_err(StoreError::backend)
    }

    /// The tokens to narrow the corpus by: each query token, plus — for a token the store
    /// holds in no document of its own — the stored tokens that contain it and the other
    /// names for the same thing ([`doc_core::search::vocabulary_kin`]).
    ///
    /// This is the SQL half of the ranker's loose tier ([`doc_core::search`]): the ranker can
    /// only score documents it was handed, so a narrowing that knows nothing of containing
    /// words or of *graphics card* meaning `gpu` would decide the question before the scorer
    /// ever saw it.
    ///
    /// Complexity: one indexed lookup per token, plus one `LIKE` scan of the token table for
    /// each token that has no holder — the case that would otherwise contribute nothing.
    fn narrowing_tokens(&self, query_tokens: &[String]) -> Result<Vec<String>, StoreError> {
        let mut narrowing = Vec::new();
        for token in query_tokens {
            narrowing.push(token.clone());
            // Kin by meaning, always — the ranker scores them whether or not the word itself
            // is held, so narrowing must hand it those documents either way.
            narrowing.extend(vocabulary_kin(token));

            let held: Option<i64> = self
                .connection
                .query_row(
                    "SELECT 1 FROM tokens WHERE token = ?1 LIMIT 1",
                    params![token],
                    |row| row.get(0),
                )
                .optional()
                .map_err(StoreError::backend)?;
            if held.is_some() || token.chars().count() < MIN_LOOSE_LEN {
                continue;
            }
            let mut statement = self
                .connection
                .prepare(
                    "SELECT DISTINCT token FROM tokens WHERE token LIKE ?1
                     ORDER BY length(token), token LIMIT ?2",
                )
                .map_err(StoreError::backend)?;
            let containers = statement
                .query_map(params![format!("%{token}%"), MAX_LOOSE_VARIANTS], |row| {
                    row.get::<_, String>(0)
                })
                .map_err(StoreError::backend)?
                .collect::<Result<Vec<String>, _>>()
                .map_err(StoreError::backend)?;
            narrowing.extend(containers);
        }
        Ok(narrowing)
    }

    /// Search stored documents for `query`, best matches first, each with its relevance score.
    ///
    /// The token table narrows the corpus to documents that contain at least one query token;
    /// only those are reassembled and ranked, through the same [`build_index`] the rest of the
    /// platform uses. Narrowing in SQL keeps the cost proportional to the matches rather than
    /// to the whole store, and reusing `build_index` keeps one ranking implementation — the
    /// score returned here is that implementation's own, never a placeholder standing in for it.
    /// The full document count is stated to the index, so IDF ranks the survivors exactly as
    /// the whole store would — narrowing changes what is reassembled, never what wins.
    ///
    /// A word the store holds inside longer words but never on its own narrows to those
    /// words ([`Store::narrowing_tokens`]), so the ranker's loose tier still has the
    /// documents it would have scored. Narrowing must never be the reason a hit is missing.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<ScoredSummary>, StoreError> {
        let tokens = self.narrowing_tokens(&distinct_tokens(query))?;
        if tokens.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }

        let placeholders = vec!["?"; tokens.len()].join(", ");
        let sql = format!(
            "SELECT DISTINCT d.path FROM documents d JOIN tokens t ON t.document_id = d.id
             WHERE t.token IN ({placeholders}) ORDER BY d.path"
        );
        let mut statement = self.connection.prepare(&sql).map_err(StoreError::backend)?;
        let candidates = statement
            .query_map(rusqlite::params_from_iter(tokens.iter()), |row| {
                row.get::<_, String>(0)
            })
            .map_err(StoreError::backend)?
            .collect::<Result<Vec<String>, _>>()
            .map_err(StoreError::backend)?;

        let mut documents = Vec::new();
        for path in candidates {
            documents.push((path.clone(), self.document(&path)?));
        }

        let summaries: HashMap<String, Summary> = self
            .list()?
            .into_iter()
            .map(|summary| (summary.path.clone(), summary))
            .collect();
        // Token narrowing kept every holder of every query token, so per-term counts are
        // exact; stating the store's true size keeps IDF's n exact too.
        let index = build_index(
            documents
                .iter()
                .map(|(path, document)| (path.as_str(), document)),
        )
        .with_corpus_size(summaries.len());

        Ok(index
            .search(query)
            .into_iter()
            .take(limit)
            .filter_map(|hit| {
                summaries
                    .get(&hit.path)
                    .cloned()
                    .map(|summary| ScoredSummary {
                        summary,
                        score: hit.score,
                    })
            })
            .collect())
    }

    /// Storage totals across the whole store.
    pub fn stats(&self) -> Result<Stats, StoreError> {
        let row = self
            .connection
            .query_row(
                "SELECT
                    (SELECT count(*) FROM documents),
                    (SELECT count(*) FROM chunks),
                    (SELECT count(*) FROM documents d
                       JOIN manifest_chunks mc ON mc.digest = d.source_digest),
                    (SELECT coalesce(sum(m.source_bytes), 0) FROM documents d
                       JOIN manifests m ON m.digest = d.source_digest),
                    (SELECT coalesce(sum(length(bytes)), 0) FROM chunks)",
                [],
                |row| {
                    Ok(Stats {
                        documents: row.get::<_, i64>(0)?.max(0) as usize,
                        chunks: row.get::<_, i64>(1)?.max(0) as usize,
                        chunk_references: row.get::<_, i64>(2)?.max(0) as usize,
                        source_bytes: row.get::<_, i64>(3)?.max(0) as usize,
                        stored_bytes: row.get::<_, i64>(4)?.max(0) as usize,
                    })
                },
            )
            .map_err(StoreError::backend)?;
        Ok(row)
    }

    /// Delete every chunk no manifest references, returning how many went.
    ///
    /// Manifests are deliberately kept even when no current document references them:
    /// they are the version history [`Store::source_of_version`] answers from, which is
    /// what lets `git log -p` and `git show` still render a document at an old revision
    /// after an edit — or after the document itself is deleted.
    pub fn collect_garbage(&self) -> Result<usize, StoreError> {
        let removed = self
            .connection
            .execute(
                "DELETE FROM chunks WHERE hash NOT IN (SELECT chunk_hash FROM manifest_chunks)",
                [],
            )
            .map_err(StoreError::backend)?;
        Ok(removed)
    }

    /// Write the repo and local packs from what is stored.
    ///
    /// Outside a repair this refuses to drop a document the tree still points at
    /// ([`StoreError::WouldLose`]): an export trusts the index completely, so an index that
    /// has lost documents would otherwise make the loss durable in the packs.
    fn export_packs(&self) -> Result<(), StoreError> {
        pack::export(
            self,
            if self.repairing {
                pack::Loss::Expected
            } else {
                pack::Loss::Refuse
            },
        )
    }

    fn export_source_index(&self) -> Result<(), StoreError> {
        let source_index_path = self.root.join(pack::SOURCE_INDEX);
        let mut lines = String::new();

        let mut stmt = self
            .connection
            .prepare("SELECT DISTINCT path FROM source_files ORDER BY path")
            .map_err(StoreError::backend)?;

        let file_paths: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .map_err(StoreError::backend)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::backend)?;

        for path in file_paths {
            let mut tokens_stmt = self
                .connection
                .prepare("SELECT token FROM source_tokens WHERE path = ?1 ORDER BY token")
                .map_err(StoreError::backend)?;

            let tokens: Vec<String> = tokens_stmt
                .query_map([&path], |row| row.get(0))
                .map_err(StoreError::backend)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(StoreError::backend)?;

            if !tokens.is_empty() {
                lines.push_str(&path);
                for token in tokens {
                    lines.push('\t');
                    lines.push_str(&token);
                }
                lines.push('\n');
            }
        }

        // Write the file, or remove it if there's nothing to index
        if lines.is_empty() {
            if source_index_path.exists() {
                fs::remove_file(&source_index_path).map_err(|error| {
                    StoreError::Backend(format!(
                        "could not remove {}: {error}",
                        source_index_path.display()
                    ))
                })?;
            }
        } else {
            fs::write(&source_index_path, &lines).map_err(|error| {
                StoreError::Backend(format!(
                    "could not write {}: {error}",
                    source_index_path.display()
                ))
            })?;
        }

        Ok(())
    }

    /// Reconcile the workspace with the store so every `.dx` file resolves correctly.
    ///
    /// This is the repair path, and it is deliberately conservative — it never discards
    /// content:
    /// 1. A `.dx` file holding **plain text** is adopted into the store. Anything that wrote
    ///    a document without going through `dx` keeps its work.
    /// 2. A **stub the store knows** is left alone, and rewritten only if its digest drifted.
    /// 3. A **stub the store does not know** is restored from the packs — the fresh-clone
    ///    case, where the database does not exist yet.
    /// 4. A stub nothing can resolve is reported in [`SyncReport::unresolved`] rather than
    ///    deleted or blanked.
    /// 5. An index row whose `.dx` file is **gone** is pruned and the packs rewritten
    ///    without it. The file is how a document exists in a workspace and the index is a
    ///    rebuildable cache over the tree, so the tree wins — a row that outlives its file
    ///    is a ghost that keeps resolving a document nobody can see. This never fires on a
    ///    fresh clone: a new database has no rows, so every pack document is restored, not
    ///    pruned, and the deleted document's bytes stay reachable in the pack's git history.
    ///
    /// Finally the packs are written whether or not any of that fired, because a pack can be
    /// out of date with **nothing to reconcile**: the encoding policy it was written under may
    /// have moved, or the file's git status may have. The writer compares the bytes it would
    /// write against the bytes that are there, so the common case still touches
    /// nothing, and [`SyncReport::packs_rewritten`] names what actually changed.
    pub fn sync(&mut self) -> Result<SyncReport, StoreError> {
        let before = pack::snapshot(&self.root);

        self.repairing = true;
        let reconciled = self.reconcile();
        self.repairing = false;
        let mut report = reconciled?;

        self.export_packs()?;
        self.export_source_index()?;

        // Write deferred stubs now that packs are durable. These are stubs that need
        // rewriting after recovery from the packs, ensuring they are written only after
        // their content is safely on disk.
        let deferred = std::mem::take(&mut self.deferred_stubs);
        for (relative, source) in deferred {
            if self.write_stub(&relative, &source)? {
                report.stubs_written.push(relative);
            }
        }

        let after = pack::snapshot(&self.root);
        report.packs_rewritten = pack::NAMES
            .iter()
            .zip(before.iter().zip(after.iter()))
            .filter(|(_, (was, now))| was != now)
            .map(|(name, _)| (*name).to_string())
            .collect();
        Ok(report)
    }

    /// Steps 1–5 of [`Store::sync`], with [`Store::repairing`] set: put the tree and the index
    /// back in agreement. Kept separate so the flag is cleared on every exit path, including
    /// an error.
    fn reconcile(&mut self) -> Result<SyncReport, StoreError> {
        let mut report = SyncReport::default();
        let available = pack::load_all(&self.root)?;
        // Paths a document has moved away from: their rows go, and neither the prune nor the
        // materialize pass below may treat them as a deletion or as a document to bring
        // back — one would report a move as a loss, the other would undo it.
        let mut moved_from: BTreeSet<String> = BTreeSet::new();

        for absolute in discover(&self.root) {
            let Some(relative) = self.relative_of(&absolute) else {
                continue;
            };
            let Ok(text) = fs::read_to_string(&absolute) else {
                continue;
            };

            match stub::digest_in(&text) {
                None => {
                    // Real content on disk: adopt it, then replace it with its pointer —
                    // unless a merge left it half-resolved, which is a decision only a
                    // person can finish and must not be stored as if it were a document.
                    if doc_core::merge::has_conflict_markers(&text) {
                        report.conflicted.push(relative);
                        continue;
                    }
                    self.ingest(&relative, &text)?;
                    report.ingested.push(relative);
                }
                Some(digest) => {
                    let known = self.document_id(&relative)?.is_some();
                    if known {
                        let source = self.source(&relative)?;
                        if stub::digest_of(&source) == digest {
                            continue;
                        }
                        // Document is in the index but stub doesn't match. Check if the stub's
                        // version exists in the packs. If yes, this might be a merge where packs
                        // were updated but index wasn't. If no, this is a normal mismatch where
                        // the index is authoritative and the stub must be rewritten.
                        match packed_as(&available, &digest) {
                            Some((_, packed)) => {
                                // Stub's version IS in packs. This could be:
                                // 1. A merge: packs have new version from merge, index has old
                                // 2. A partial write: index has new version, packs still have old
                                // We trust packs as the source of truth and restore from them.
                                self.ingest(&relative, packed.as_str())?;
                                report.restored.push(relative);
                            }
                            None => {
                                // Stub's version is NOT in packs. The index has the only copy
                                // and is the authoritative version. The stub must be rewritten.
                                // Defer until after export makes the content durable on disk.
                                self.deferred_stubs.push((relative, source));
                            }
                        }
                        continue;
                    }
                    // A pointer the index has never seen. Its own path first — the
                    // fresh-clone case — and then the same digest under *any* name, because
                    // a document that moved keeps its content and changes only its key. Not
                    // following that was a way to destroy a document with two ordinary
                    // commands: `git mv` left the new path unresolved and the old path a row
                    // whose file was gone, and rule 5 pruned the only copy.
                    let held = available
                        .get_key_value(&relative)
                        .or_else(|| packed_as(&available, &digest));
                    match held {
                        Some((was, source)) => {
                            let from = was.clone();
                            self.ingest(&relative, source)?;
                            if from == relative {
                                report.restored.push(relative);
                            } else {
                                moved_from.insert(from.clone());
                                report.moved.push((from, relative));
                            }
                        }
                        None => {
                            if crate::git::is_tracked_but_ignored(&self.root, &relative) {
                                report.tracked_but_ignored.push(relative);
                            } else {
                                let digest = stub::digest_in(&text).map(|d| d.to_string());
                                report.unresolved.push(relative.clone());
                                report.unresolved_details.push(UnresolvedPointer {
                                    path: relative,
                                    digest,
                                });
                            }
                        }
                    }
                }
            }
        }

        // A row whose file is gone records a deletion made in the tree (rule 5 above):
        // prune it, then rewrite the packs so the deletion sticks instead of being
        // restored by the next sync. A path the document moved *away* from is one of these
        // and is not a deletion — the row goes, and the report says "moved", because
        // "pruned" beside a restore is how a move used to read like two accidents.
        let mut pruned_any = false;
        for summary in self.list()? {
            if self.stub_path(&summary.path).exists() {
                continue;
            }
            self.connection
                .execute(
                    "DELETE FROM documents WHERE path = ?1",
                    params![summary.path],
                )
                .map_err(StoreError::backend)?;
            pruned_any = true;
            if !moved_from.contains(&summary.path) {
                report.pruned.push(summary.path);
            }
        }
        if pruned_any {
            self.export_packs()?;
        }

        // Anything left in the packs that the database does not know is materialized —
        // row, stub file and all — so a pack document is always reachable. Skipping what
        // was just pruned matters: the pack copy loaded before the prune is the ghost
        // itself, and materializing it would resurrect the deleted file.
        for (relative, source) in &available {
            if report.pruned.contains(relative) || moved_from.contains(relative) {
                continue;
            }
            if self.document_id(relative)?.is_none() {
                self.ingest(relative, source)?;
                report.restored.push(relative.clone());
            }
        }

        report.chunks_collected = self.collect_garbage()?;
        Ok(report)
    }

    /// The workspace-relative path of `absolute`, or `None` when it is outside the root.
    fn relative_of(&self, absolute: &Path) -> Option<String> {
        let relative = absolute.strip_prefix(&self.root).ok()?;
        stub::normalize_path(&relative.to_string_lossy())
    }
}

/// The pack entry whose content is exactly `digest`, under whatever name it is filed.
///
/// The packs are keyed by path and a document's identity is its content, so a pointer is
/// answerable by any entry carrying its digest. That is what makes a move followable: the new
/// path has never been in a pack, and the old one holds precisely what the new pointer names.
fn packed_as<'a>(
    available: &'a BTreeMap<String, String>,
    digest: &str,
) -> Option<(&'a String, &'a String)> {
    available
        .iter()
        .find(|(_, source)| stub::digest_of(source) == digest)
}

/// Compress `text` when that actually saves space, reporting which form was chosen.
///
/// Short blocks — a heading, a one-line paragraph — usually grow under any compressor, so
/// storing them raw is both smaller and cheaper to read back.
fn encode_chunk(text: &str) -> (Vec<u8>, bool) {
    let plain = text.as_bytes();
    let squeezed = compress(plain);
    if squeezed.len() < plain.len() {
        (squeezed, true)
    } else {
        (plain.to_vec(), false)
    }
}

/// Rebuild the derived section and token rows for document `id`.
///
/// Both are caches over the chunks, so they are deleted and rewritten wholesale rather than
/// patched — a partial update is how a read model starts disagreeing with its source.
fn write_read_models(
    transaction: &rusqlite::Transaction<'_>,
    id: i64,
    document: &Document,
) -> Result<(), StoreError> {
    transaction
        .execute("DELETE FROM sections WHERE document_id = ?1", params![id])
        .map_err(StoreError::backend)?;
    transaction
        .execute("DELETE FROM tokens WHERE document_id = ?1", params![id])
        .map_err(StoreError::backend)?;

    for entry in outline(document) {
        if entry.level == 0 {
            continue;
        }
        transaction
            .execute(
                "INSERT INTO sections (document_id, position, slug, heading, depth)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    id,
                    entry.index as i64,
                    entry.id,
                    entry.preview,
                    i64::from(entry.level)
                ],
            )
            .map_err(StoreError::backend)?;
    }

    let mut counts: HashMap<String, i64> = HashMap::new();
    for token in document_tokens(document) {
        *counts.entry(token).or_insert(0) += 1;
    }
    for (token, hits) in counts {
        transaction
            .execute(
                "INSERT INTO tokens (document_id, token, hits) VALUES (?1, ?2, ?3)",
                params![id, token, hits],
            )
            .map_err(StoreError::backend)?;
    }
    Ok(())
}

/// An ISO-8601 timestamp for "now", to second precision and UTC.
///
/// Hand-rolled from the epoch seconds so the crate needs no date dependency. A clock that
/// predates the epoch yields the epoch itself rather than an error, since a timestamp is
/// metadata and must never be the reason a save fails.
///
/// Public because everything dx stamps — a saved manifest, a filed report — must be
/// stamped the same way; a second spelling of "now" is a second answer to when.
#[must_use]
pub fn timestamp() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());

    let days = seconds / 86_400;
    let time_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days as i64);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        time_of_day / 3600,
        (time_of_day % 3600) / 60,
        time_of_day % 60
    )
}

/// Convert days since 1970-01-01 into a `(year, month, day)` civil date.
///
/// Howard Hinnant's `civil_from_days`, which is exact for the proleptic Gregorian calendar
/// and needs no lookup tables.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    } as u32;
    (year + i64::from(month <= 2), month, day)
}

/// Find every `.dx` file under `root`, sorted by path.
#[must_use]
pub fn discover(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    walk(root, &mut found);
    found.sort();
    found
}

/// The first directory segment of `relative` that [`walk`] would never enter — a dotted
/// name or one of [`SKIPPED_DIRECTORIES`] — or `None` when every segment is walkable.
/// The file's own name is exempt: discovery matches files by their `.dx` extension alone.
fn unwalked_directory(relative: &str) -> Option<&str> {
    let mut segments = relative.split('/');
    let _file = segments.next_back();
    segments.find(|segment| segment.starts_with('.') || SKIPPED_DIRECTORIES.contains(segment))
}

/// Recursively collect `.dx` files, skipping build output and the store itself.
fn walk(directory: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if !(name.starts_with('.') || SKIPPED_DIRECTORIES.contains(&name.as_str())) {
                walk(&path, found);
            }
            continue;
        }
        if path.extension().is_some_and(|extension| extension == "dx") {
            found.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::load_all;

    /// A scratch workspace directory, emptied first so runs do not leak into each other.
    fn scratch(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dx-store-tests-{label}"));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("scratch root");
        root
    }

    const NOTES: &str = "::heading level=1 id=notes\nNotes\n::end\n\n::paragraph id=p\nA line of prose about kubernetes.\n::end\n";

    #[test]
    fn a_save_into_a_directory_discovery_never_walks_is_refused_with_the_rule() {
        // Storing under `build/` or `fixtures/` creates a row no listing can re-find —
        // the ghost-document symptom by another door — so the save itself refuses.
        let root = scratch("unlistable-save");
        let mut store = Store::open(&root).expect("open");
        for path in ["build/notes.dx", "fixtures/deep/case.dx", ".cache/notes.dx"] {
            let error = store.ingest(path, NOTES).expect_err(path);
            let sentence = error.to_string();
            assert!(sentence.contains("never appear in a listing"), "{sentence}");
        }
        // The rule names directories, never the file itself: a dotted or oddly named
        // *file* is still discovered by its extension.
        store.ingest("build-notes.dx", NOTES).expect("plain save");
        assert_eq!(store.list().expect("list").len(), 1);
    }

    #[test]
    fn a_saved_document_leaves_a_stub_on_disk_and_reads_back_whole() {
        let root = scratch("stub-and-read");
        let mut store = Store::open(&root).expect("open");
        store.ingest("notes.dx", NOTES).expect("ingest");

        // On disk: a pointer, not content.
        let on_disk = fs::read_to_string(root.join("notes.dx")).expect("read stub");
        assert!(stub::is_stub(&on_disk), "expected a stub, got {on_disk:?}");
        assert!(!on_disk.contains("kubernetes"));
        assert!(on_disk.len() < 80);

        // Through the resolver: the true document, byte-for-byte canonical.
        assert_eq!(store.source("notes.dx").expect("source"), NOTES);
        assert_eq!(store.document("notes.dx").expect("doc").blocks.len(), 2);
    }

    #[test]
    fn every_authored_attribute_survives_a_store_round_trip() {
        let root = scratch("lossless");
        let mut store = Store::open(&root).expect("open");
        let source = "::code id=stats lang=python run deps=\"numpy pandas\" timeout=45 format=svg\nprint(1)\n::end\n\n::output id=o for=stats status=ok hash=abc123 format=svg\n<svg></svg>\n::end\n\n::checklist id=c\n[x] done\n[ ] todo\n::end\n";
        store.ingest("rich.dx", source).expect("ingest");
        assert_eq!(store.source("rich.dx").expect("source"), source);
    }

    #[test]
    fn identical_blocks_across_documents_are_stored_once() {
        let root = scratch("sharing");
        let mut store = Store::open(&root).expect("open");
        let shared = "::paragraph id=p\nExactly the same block.\n::end\n";
        store.ingest("a.dx", shared).expect("a");
        let second = store.ingest("b.dx", shared).expect("b");

        assert_eq!(second.chunks, 1);
        assert_eq!(second.chunks_added, 0, "the block should already be stored");
        let stats = store.stats().expect("stats");
        assert_eq!(stats.documents, 2);
        assert_eq!(stats.chunks, 1);
        assert_eq!(stats.chunk_references, 2);
    }

    #[test]
    fn editing_one_block_leaves_the_others_shared() {
        let root = scratch("incremental");
        let mut store = Store::open(&root).expect("open");
        store
            .ingest(
                "doc.dx",
                "::paragraph id=a\nkeep\n::end\n\n::paragraph id=b\nchange me\n::end\n",
            )
            .expect("first");
        let after = store
            .ingest(
                "doc.dx",
                "::paragraph id=a\nkeep\n::end\n\n::paragraph id=b\nchanged\n::end\n",
            )
            .expect("second");

        assert_eq!(after.chunks, 2);
        assert_eq!(after.chunks_added, 1, "only the edited block is new");
    }

    #[test]
    fn storage_is_smaller_than_the_source_it_holds() {
        let root = scratch("compaction");
        let mut store = Store::open(&root).expect("open");
        let big = include_str!("../../doc-core/tests/fixtures/showcase.input.dx");
        store.ingest("showcase.dx", big).expect("ingest");

        let stats = store.stats().expect("stats");
        let percent = stats.compaction_percent().expect("percent");
        assert!(
            percent < 100.0,
            "stored {} bytes for {} bytes of source ({percent:.1}%)",
            stats.stored_bytes,
            stats.source_bytes
        );
    }

    #[test]
    fn a_missing_document_says_so_instead_of_returning_nothing() {
        let root = scratch("missing");
        let store = Store::open(&root).expect("open");
        let error = store.source("nope.dx").expect_err("should fail");
        assert!(matches!(error, StoreError::NotFound(_)), "{error}");
        assert!(error.to_string().contains("dx ls"));
    }

    #[test]
    fn a_path_escaping_the_workspace_is_refused() {
        let root = scratch("traversal");
        let mut store = Store::open(&root).expect("open");
        assert!(matches!(
            store.source("../../etc/passwd"),
            Err(StoreError::InvalidPath(_))
        ));
        assert!(matches!(
            store.ingest("../escape.dx", NOTES),
            Err(StoreError::InvalidPath(_))
        ));
    }

    #[test]
    fn deleting_a_document_removes_its_stub_and_stops_resolving_the_path() {
        let root = scratch("delete");
        let mut store = Store::open(&root).expect("open");
        let saved = store.ingest("gone.dx", NOTES).expect("ingest");
        store.delete("gone.dx").expect("delete");

        assert!(!root.join("gone.dx").exists(), "stub should be gone");
        assert!(matches!(
            store.source("gone.dx"),
            Err(StoreError::NotFound(_))
        ));
        assert_eq!(store.stats().expect("stats").documents, 0);
        // The version itself is still addressable, so an old commit still diffs.
        assert_eq!(
            store
                .source_of_version(&saved.digest)
                .expect("version lookup"),
            Some(NOTES.to_string())
        );
    }

    #[test]
    fn an_earlier_version_stays_addressable_by_its_digest() {
        // This is what lets `git log -p` and `git show` render a document at an old revision.
        let root = scratch("history");
        let mut store = Store::open(&root).expect("open");
        let first = store.ingest("doc.dx", NOTES).expect("first");
        let changed =
            "::heading level=1 id=notes\nNotes\n::end\n\n::paragraph id=p\nrewritten\n::end\n";
        let second = store.ingest("doc.dx", changed).expect("second");

        assert_ne!(first.digest, second.digest);
        assert_eq!(
            store.source_of_version(&first.digest).expect("old"),
            Some(NOTES.to_string())
        );
        assert_eq!(
            store.source_of_version(&second.digest).expect("new"),
            Some(changed.to_string())
        );
        // The current read still gives the newest content.
        assert_eq!(store.source("doc.dx").expect("current"), changed);
    }

    #[test]
    fn keeping_history_costs_only_the_blocks_that_changed() {
        let root = scratch("history-cost");
        let mut store = Store::open(&root).expect("open");
        store
            .ingest(
                "doc.dx",
                "::paragraph id=a\nkeep\n::end\n\n::paragraph id=b\nfirst\n::end\n",
            )
            .expect("first");
        let after = store
            .ingest(
                "doc.dx",
                "::paragraph id=a\nkeep\n::end\n\n::paragraph id=b\nsecond\n::end\n",
            )
            .expect("second");

        assert_eq!(after.chunks_added, 1, "only the edited block is new");
        // Three distinct blocks stored across two versions of a two-block document.
        assert_eq!(store.stats().expect("stats").chunks, 3);
    }

    #[test]
    fn an_unknown_version_is_absent_rather_than_an_error() {
        let root = scratch("unknown-version");
        let store = Store::open(&root).expect("open");
        assert_eq!(
            store.source_of_version(&"0".repeat(64)).expect("lookup"),
            None
        );
    }

    #[test]
    fn deleting_one_document_keeps_chunks_another_still_shares() {
        let root = scratch("shared-delete");
        let mut store = Store::open(&root).expect("open");
        let shared = "::paragraph id=p\nshared body\n::end\n";
        store.ingest("keep.dx", shared).expect("keep");
        store.ingest("drop.dx", shared).expect("drop");
        store.delete("drop.dx").expect("delete");

        assert_eq!(store.source("keep.dx").expect("still there"), shared);
        assert_eq!(store.stats().expect("stats").documents, 1);
        assert_eq!(store.stats().expect("stats").chunks, 1);
    }

    #[test]
    fn listing_and_search_find_documents_through_the_database() {
        let root = scratch("search");
        let mut store = Store::open(&root).expect("open");
        store.ingest("kubernetes.dx", NOTES).expect("k");
        store
            .ingest(
                "recipes.dx",
                "::paragraph id=p\nbread and soup and butter\n::end\n",
            )
            .expect("r");

        let listed = store.list().expect("list");
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].path, "kubernetes.dx");
        assert_eq!(listed[0].title, "Notes");

        let hits = store.search("kubernetes", 10).expect("search");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].summary.path, "kubernetes.dx");
        assert!(hits[0].score > 0.0);

        assert!(store
            .search("nothingmatches", 10)
            .expect("empty")
            .is_empty());
        assert!(store.search("", 10).expect("blank").is_empty());
    }

    #[test]
    fn narrowing_keeps_the_document_only_the_vocabulary_bridge_reaches() {
        // Narrowing must never be the reason a hit is missing: a document that says `gpu`
        // holds no token of "graphics card" at all, so nothing but the bridge puts it in
        // front of the ranker.
        let root = scratch("bridge-narrowing");
        let mut store = Store::open(&root).expect("open");
        store
            .ingest(
                "backend.dx",
                "::paragraph id=p\nBACKEND_AUTO tries the GPU and falls back to the CPU\n::end\n",
            )
            .expect("ingest");
        store
            .ingest(
                "notes.dx",
                "::paragraph id=p\nwhere the values come from, and where they come from\n::end\n",
            )
            .expect("ingest");

        let hits = store
            .search(
                "how does it pick between the graphics card and the processor",
                3,
            )
            .expect("search");
        assert_eq!(
            hits.first().map(|hit| hit.summary.path.as_str()),
            Some("backend.dx"),
            "{hits:?}"
        );
    }

    #[test]
    fn search_through_the_database_ranks_the_stronger_match_first_with_a_distinct_score() {
        // Regression test: `Store::search` once discarded the ranking index's own score and
        // let the caller stand in a placeholder, so every hit looked equally confident no
        // matter how well (or poorly) it actually matched. The score returned here must be
        // the real one, and it must actually separate a strong match from a weak one.
        let root = scratch("search-scores");
        let mut store = Store::open(&root).expect("open");
        store
            .ingest(
                "strong.dx",
                "::paragraph id=p\ncompression compression compression\n::end\n",
            )
            .expect("strong");
        store
            .ingest(
                "weak.dx",
                "::paragraph id=p\na passing compression mention\n::end\n",
            )
            .expect("weak");

        let hits = store.search("compression", 10).expect("search");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].summary.path, "strong.dx");
        assert!(
            hits[0].score > hits[1].score,
            "strong.dx ({}) should outscore weak.dx ({})",
            hits[0].score,
            hits[1].score
        );
    }

    /// Narrowing runs in SQL and ranking runs in memory, so the two have to agree about
    /// what a query token can match. A word the store holds only inside longer words must
    /// still reach the ranker, or narrowing answers a question the scorer never saw.
    #[test]
    fn narrowing_reaches_a_document_that_only_holds_the_word_inside_a_longer_one() {
        let root = scratch("search-loose");
        let mut store = Store::open(&root).expect("open");
        store
            .ingest(
                "bitpack.dx",
                "::paragraph id=p\nbitpack_getu reads a bitpacked weight\n::end\n",
            )
            .expect("bitpack");
        store
            .ingest(
                "audio.dx",
                "::paragraph id=p\nthe mixer renders samples\n::end\n",
            )
            .expect("audio");

        let hits = store.search("how are weights packed", 10).expect("search");
        assert_eq!(
            hits.first().map(|hit| hit.summary.path.as_str()),
            Some("bitpack.dx")
        );
    }

    /// Opening a store migrates it, and migrating discards the derived tables — so a read
    /// has to be able to see that an index is old without rewriting it. This is the guard
    /// that keeps `dx search` on a just-upgraded install from silently rebuilding the index.
    #[test]
    fn an_index_from_an_older_dx_reads_as_stale_and_is_left_exactly_as_it_was() {
        let root = scratch("stale-index");
        {
            let mut store = Store::open(&root).expect("open");
            store.ingest("notes.dx", NOTES).expect("ingest");
        }
        let database = root.join(DB_RELATIVE);
        let connection = Connection::open(&database).expect("reopen");
        connection
            .pragma_update(None, "user_version", schema::VERSION - 1)
            .expect("stamp an older version");
        drop(connection);

        assert!(stale_index(&root).expect("ask"));
        assert_eq!(
            schema::version_at(&database).expect("version"),
            Some(schema::VERSION - 1),
            "asking must not upgrade what it asked about"
        );

        // The write path is where the upgrade belongs, and it happens there.
        let _ = Store::open(&root).expect("open");
        assert!(!stale_index(&root).expect("ask again"));
    }

    #[test]
    fn sync_adopts_plain_text_written_by_anything_else() {
        let root = scratch("ingest-plain");
        let mut store = Store::open(&root).expect("open");
        // Something that is not dx writes a document directly.
        fs::write(root.join("outside.dx"), NOTES).expect("write");

        let report = store.sync().expect("sync");
        assert_eq!(report.ingested, vec!["outside.dx".to_string()]);
        assert_eq!(store.source("outside.dx").expect("resolves"), NOTES);
        assert!(stub::is_stub(
            &fs::read_to_string(root.join("outside.dx")).expect("read")
        ));
    }

    #[test]
    fn sync_restores_documents_from_the_pack_when_the_database_is_gone() {
        // The fresh-clone case: stubs and packs are committed, index.db is not.
        let root = scratch("fresh-clone");
        {
            let mut store = Store::open(&root).expect("open");
            store.ingest("notes.dx", NOTES).expect("ingest");
        }
        fs::remove_file(root.join(DB_RELATIVE)).expect("drop database");

        let mut store = Store::open(&root).expect("reopen");
        assert!(
            store.source("notes.dx").is_err(),
            "database really is empty"
        );

        let report = store.sync().expect("sync");
        assert!(report.restored.contains(&"notes.dx".to_string()));
        assert_eq!(store.source("notes.dx").expect("restored"), NOTES);
    }

    #[test]
    fn sync_reports_a_stub_it_cannot_resolve_instead_of_serving_nothing() {
        let root = scratch("unresolved");
        let mut store = Store::open(&root).expect("open");
        fs::write(root.join("orphan.dx"), stub::render(NOTES)).expect("write stub");

        let report = store.sync().expect("sync");
        assert_eq!(report.unresolved, vec!["orphan.dx".to_string()]);
        // And the file is left exactly as found — never blanked.
        assert!(stub::is_stub(
            &fs::read_to_string(root.join("orphan.dx")).expect("read")
        ));
    }

    #[test]
    fn sync_rewrites_a_pack_whose_encoding_no_longer_matches_the_policy() {
        // The gap this closes: with nothing to reconcile, sync used to return early and a
        // pack written under a policy that has since moved was never rewritten. Harmless
        // while every policy decodes, and wrong the day one does not.
        let root = scratch("policy-drift");
        let mut store = Store::open(&root).expect("open");
        store.ingest("notes.dx", NOTES).expect("ingest");
        assert!(store.sync().expect("settle").is_clean());

        // Rewrite the pack under the other policy, without changing a single document.
        let (repo_path, _) = pack::paths(&root);
        let source = store.source("notes.dx").expect("source");
        let pack = doc_core::chunk::Pack::build([("notes.dx", &parse(&source))]);
        let other =
            if doc_core::chunk::encode_pack_for(&pack, doc_core::chunk::PackStorage::Compressed)
                == fs::read(&repo_path).expect("read")
            {
                doc_core::chunk::PackStorage::ForVersionControl
            } else {
                doc_core::chunk::PackStorage::Compressed
            };
        fs::write(&repo_path, doc_core::chunk::encode_pack_for(&pack, other)).expect("write");

        let report = store.sync().expect("sync");
        assert_eq!(
            report.packs_rewritten,
            vec![pack::REPO_PACK.to_string()],
            "a pack whose encoding drifted must be rewritten: {report:?}"
        );
        assert!(!report.is_clean(), "and reported, not called clean");
        assert!(store.sync().expect("settle").is_clean(), "then it settles");
    }

    #[test]
    fn sync_restores_documents_the_index_lost_without_tripping_the_export_guard() {
        // The two changes have to compose: exporting refuses to drop a pointed-at document,
        // and sync is the repair that puts those documents back — so the guard must stand
        // down for exactly the length of the repair, and be back up after it.
        let root = scratch("repair-guard");
        let mut store = Store::open(&root).expect("open");
        for name in ["a.dx", "b.dx", "c.dx"] {
            store.ingest(name, NOTES).expect("ingest");
        }
        drop(store);
        fs::remove_file(root.join(".doc/index.db")).expect("lose the index");

        let mut store = Store::open(&root).expect("reopen");
        let report = store.sync().expect("sync repairs rather than refusing");
        assert_eq!(report.restored.len(), 3, "{report:?}");
        for name in ["a.dx", "b.dx", "c.dx"] {
            assert!(store.source(name).is_ok(), "{name} must resolve again");
        }
        assert!(store.sync().expect("settle").is_clean());
    }

    #[test]
    fn sync_after_a_merge_keeps_the_merged_document_instead_of_rewinding_the_pointer() {
        // What a merge leaves: git has written the merged pointer and the merge driver has
        // written the merged pack, and neither wrote the store — the driver is a read. The
        // index therefore still holds this branch's version. Sync used to treat the index as
        // the authority and rewrite the pointer from it, which threw the merge away.
        let root = scratch("merged-pointer");
        let mut store = Store::open(&root).expect("open");
        store.ingest("notes.dx", NOTES).expect("ours");

        // The pack the merge driver would have written, built the way any pack is.
        let merged = NOTES.replace(
            "A line of prose about kubernetes.",
            "What the merge produced.",
        );
        let elsewhere = scratch("merged-pointer-incoming");
        let mut theirs = Store::open(&elsewhere).expect("open");
        theirs.ingest("notes.dx", &merged).expect("their version");
        drop(theirs);

        let (repo_path, _) = pack::paths(&root);
        let (their_pack, _) = pack::paths(&elsewhere);
        fs::copy(&their_pack, &repo_path).expect("the merge driver's pack");
        fs::write(root.join("notes.dx"), stub::render(&merged)).expect("git's merged pointer");

        let report = store.sync().expect("sync");
        assert_eq!(report.restored, vec!["notes.dx".to_string()], "{report:?}");
        assert!(
            report.stubs_written.is_empty(),
            "the pointer git wrote must stand: {report:?}"
        );
        assert_eq!(store.source("notes.dx").expect("resolves"), merged);
    }

    #[test]
    fn a_save_refuses_when_the_index_has_lost_a_document_the_tree_still_points_at() {
        let root = scratch("save-guard");
        let mut store = Store::open(&root).expect("open");
        store.ingest("keep.dx", NOTES).expect("keep");
        store.ingest("lost.dx", NOTES).expect("lost");
        store
            .connection
            .execute("DELETE FROM documents WHERE path = 'lost.dx'", [])
            .expect("forget");

        let error = store
            .ingest("keep.dx", "::heading level=1 id=n\nNotes again\n::end\n")
            .expect_err("a save must not make the loss durable");
        assert!(
            matches!(&error, StoreError::WouldLose(path) if path == "lost.dx"),
            "{error}"
        );
        // And the repair the message names actually works.
        // Note: both "keep.dx" and "lost.dx" are restored from packs because their
        // stubs are out of sync with the index. The packs are trusted as the authoritative
        // source during sync, so any document whose stub names a version in the packs is
        // restored from it.
        let report = store.sync().expect("sync");
        let mut restored = report.restored.clone();
        restored.sort();
        assert_eq!(
            restored,
            vec!["keep.dx".to_string(), "lost.dx".to_string()],
            "{report:?}"
        );
    }

    #[test]
    fn sync_follows_a_moved_document_instead_of_pruning_the_only_copy_of_it() {
        // `git mv thing.dx docs/thing.dx` used to be a way to destroy a document with two
        // ordinary commands: the new path resolved to nothing, the old path was a row whose
        // file was gone, and the prune took the last copy while both commands reported
        // success. The pointer names the content, so the content is findable under any name.
        let root = scratch("follow-move");
        let mut store = Store::open(&root).expect("open");
        store.ingest("thing.dx", NOTES).expect("ingest");
        store.ingest("stays.dx", "# Stays\n").expect("ingest");
        drop(store);

        fs::create_dir_all(root.join("docs")).expect("dirs");
        fs::rename(root.join("thing.dx"), root.join("docs/thing.dx")).expect("move it");

        let mut store = Store::open(&root).expect("reopen");
        let report = store.sync().expect("sync");
        assert_eq!(
            report.moved,
            vec![("thing.dx".to_string(), "docs/thing.dx".to_string())],
            "{report:?}"
        );
        assert!(
            report.pruned.is_empty(),
            "a move is not a deletion: {report:?}"
        );
        assert!(report.unresolved.is_empty(), "{report:?}");
        assert_eq!(store.source("docs/thing.dx").expect("resolves"), NOTES);
        assert!(
            !root.join("thing.dx").exists(),
            "the old path must not be resurrected"
        );
        assert!(
            store.source("stays.dx").is_ok(),
            "the untouched document stays"
        );
        assert!(store.sync().expect("settle").is_clean(), "and it settles");
    }

    #[test]
    fn sync_prunes_a_row_whose_file_was_deleted_and_the_deletion_sticks() {
        // Deleting the .dx file is deleting the document: the tree wins over the index,
        // and the pack rewrite keeps the next sync from resurrecting the ghost.
        let root = scratch("prune-ghost");
        let mut store = Store::open(&root).expect("open");
        let saved = store.ingest("notes.dx", NOTES).expect("ingest");
        store.ingest("kept.dx", "# Kept\n").expect("ingest kept");
        fs::remove_file(root.join("notes.dx")).expect("delete the file");

        let report = store.sync().expect("sync");
        assert_eq!(report.pruned, vec!["notes.dx".to_string()]);
        assert!(store.source("notes.dx").is_err(), "the row is gone");
        assert!(store.source("kept.dx").is_ok(), "other documents stay");
        // History survives the prune, exactly as it survives Store::delete: an old
        // commit still diffs.
        assert_eq!(
            store
                .source_of_version(&saved.digest)
                .expect("version lookup"),
            Some(NOTES.to_string())
        );

        let settled = store.sync().expect("second sync");
        assert!(settled.is_clean(), "the deletion settles: {settled:?}");
        assert!(!root.join("notes.dx").exists(), "the file stays deleted");
    }

    #[test]
    fn a_clean_workspace_syncs_to_no_changes() {
        let root = scratch("idempotent-sync");
        let mut store = Store::open(&root).expect("open");
        store.ingest("notes.dx", NOTES).expect("ingest");

        store.sync().expect("first sync");
        let second = store.sync().expect("second sync");
        assert!(second.is_clean(), "sync should settle: {second:?}");
    }

    #[test]
    fn saving_the_same_content_twice_does_not_touch_the_stub_file() {
        let root = scratch("quiet-writes");
        let mut store = Store::open(&root).expect("open");
        store.ingest("notes.dx", NOTES).expect("first");
        let before = fs::metadata(root.join("notes.dx"))
            .and_then(|meta| meta.modified())
            .expect("mtime");
        store.ingest("notes.dx", NOTES).expect("second");
        let after = fs::metadata(root.join("notes.dx"))
            .and_then(|meta| meta.modified())
            .expect("mtime");
        assert_eq!(
            before, after,
            "an unchanged save should not rewrite the stub"
        );
    }

    #[test]
    fn non_ascii_prose_survives_the_store() {
        let root = scratch("i18n");
        let mut store = Store::open(&root).expect("open");
        let source = "::paragraph id=p\nこんにちは — naïve café ✅ Ω\n::end\n";
        store.ingest("i18n.dx", source).expect("ingest");
        assert_eq!(store.source("i18n.dx").expect("source"), source);
    }

    #[test]
    fn empty_input_is_stored_as_the_canonical_empty_document() {
        let root = scratch("empty");
        let mut store = Store::open(&root).expect("open");
        store.ingest("blank.dx", "").expect("ingest");
        let source = store.source("blank.dx").expect("source");
        assert_eq!(
            source,
            "::paragraph id=paragraph-1\nStart writing here.\n::end\n"
        );
        // And that canonical form is a fixed point.
        store.ingest("blank.dx", &source).expect("re-ingest");
        assert_eq!(store.source("blank.dx").expect("source"), source);
    }

    #[test]
    fn a_document_in_a_subdirectory_round_trips() {
        let root = scratch("nested-path");
        let mut store = Store::open(&root).expect("open");
        store.ingest("notes/deep/thing.dx", NOTES).expect("ingest");
        assert!(root.join("notes/deep/thing.dx").exists());
        assert_eq!(store.source("notes/deep/thing.dx").expect("source"), NOTES);
    }

    #[test]
    fn a_tampered_chunk_is_reported_not_served() {
        let root = scratch("corrupt");
        let mut store = Store::open(&root).expect("open");
        store.ingest("notes.dx", NOTES).expect("ingest");

        // Rewrite one chunk body to something else of the same shape.
        let replacement = "::paragraph id=p\nnot what was stored\n::end";
        store
            .connection
            .execute(
                "UPDATE chunks SET bytes = ?1, compressed = 0, plain_bytes = ?2
                 WHERE hash = (SELECT chunk_hash FROM manifest_chunks
                               ORDER BY position DESC LIMIT 1)",
                params![replacement.as_bytes(), replacement.len() as i64],
            )
            .expect("tamper");

        let error = store.source("notes.dx").expect_err("should refuse");
        assert!(matches!(error, StoreError::Corrupt(_)), "{error}");
        assert!(error.to_string().contains("dx sync"));
    }

    #[test]
    fn timestamps_are_iso_8601_and_decode_a_known_day() {
        // 2026-07-29 is 20663 days after the epoch.
        assert_eq!(civil_from_days(20_663), (2026, 7, 29));
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // Leap-year boundary.
        assert_eq!(civil_from_days(59), (1970, 3, 1));

        let now = timestamp();
        assert_eq!(now.len(), 20, "{now}");
        assert!(now.ends_with('Z'));
    }

    #[test]
    fn short_blocks_are_stored_raw_and_long_ones_compressed() {
        let (bytes, compressed) = encode_chunk("::rule id=r\n\n::end");
        assert!(!compressed, "a tiny block should not be inflated");
        assert_eq!(bytes.len(), "::rule id=r\n\n::end".len());

        let repetitive = format!("::paragraph id=p\n{}\n::end", "the same words ".repeat(40));
        let (bytes, compressed) = encode_chunk(&repetitive);
        assert!(compressed, "a repetitive block should compress");
        assert!(bytes.len() < repetitive.len());
    }

    #[test]
    fn discovery_skips_build_output_and_the_store_directory() {
        let root = scratch("discover");
        for relative in [
            "a.dx",
            "deep/b.dx",
            "node_modules/c.dx",
            "target/d.dx",
            "tests/fixtures/e.dx",
            "test/fixture/f.dx",
        ] {
            let path = root.join(relative);
            fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
            fs::write(&path, NOTES).expect("write");
        }
        fs::create_dir_all(root.join(STORE_DIR)).expect("store dir");
        fs::write(root.join(STORE_DIR).join("e.dx"), NOTES).expect("write");

        let found = discover(&root);
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found.iter().all(|path| {
            let text = path.to_string_lossy();
            !text.contains("node_modules")
                && !text.contains("target")
                && !text.contains("fixture")
                && !text.contains(STORE_DIR)
        }));
    }

    #[test]
    fn summaries_carry_the_shared_title_derivation() {
        // The rule itself lives (and is pinned) in `Document::display_title`; this holds
        // the store's summaries to it.
        let bare = parse("::paragraph id=p\nbody\n::end\n");
        assert_eq!(bare.display_title("a/plain-name.dx"), "plain-name");
    }

    #[test]
    fn save_writes_pack_before_stub_for_durability() {
        // Verify that packs are exported (made durable) before stubs are written.
        // This ensures a pointer never names content not yet in any pack.
        let root = scratch("pack-before-stub");
        let mut store = Store::open(&root).expect("open");

        // Save a document
        store.ingest("notes.dx", NOTES).expect("save");

        // Verify the pack exists
        let pack_path = root.join(".doc/repo.dxcp");
        assert!(pack_path.exists(), "pack should be written during save");

        // Verify the stub exists
        let stub_path = root.join("notes.dx");
        assert!(stub_path.exists(), "stub should be written during save");

        // Read the stub and verify it's a pointer
        let stub_contents = fs::read_to_string(&stub_path).expect("read stub");
        assert_eq!(
            stub_contents.lines().count(),
            1,
            "stub should be a single-line pointer"
        );
        assert!(
            stub_contents.starts_with("~ dx1 "),
            "stub should start with dx marker"
        );

        // Verify the pack contains data (it's binary/compressed, so just check it's not empty)
        let pack_bytes = fs::read(&pack_path).expect("read pack");
        assert!(!pack_bytes.is_empty(), "pack should not be empty");

        // Verify the pack decodes correctly
        let loaded = load_all(&root).expect("load");
        assert!(
            loaded.contains_key("notes.dx"),
            "pack should contain notes.dx"
        );
    }

    #[test]
    fn sync_provides_unresolved_pointer_diagnostics() {
        // Verify that unresolved pointers include diagnostic metadata.
        let root = scratch("unresolved-diagnostics");
        let mut store = Store::open(&root).expect("open");

        // Create a pointer to non-existent content
        let fake_digest = "0".repeat(64);
        let pointer_content = format!("~ dx1 {}\n", fake_digest);
        let notes_path = root.join("notes.dx");
        fs::write(&notes_path, &pointer_content).expect("write pointer");

        // Sync should report it as unresolved with diagnostic info
        let report = store.sync().expect("sync");
        assert!(
            report.unresolved.contains(&"notes.dx".to_string()),
            "unresolved should contain notes.dx: {report:?}"
        );
        assert!(
            !report.unresolved_details.is_empty(),
            "should have unresolved details: {report:?}"
        );

        let details = &report.unresolved_details;
        assert!(
            details.iter().any(|d| d.path == "notes.dx"),
            "details should include notes.dx"
        );

        let notes_detail = details
            .iter()
            .find(|d| d.path == "notes.dx")
            .expect("find detail");
        assert_eq!(
            notes_detail.digest,
            Some(fake_digest),
            "digest should be extracted from pointer"
        );
    }

    #[test]
    fn pointer_content_matches_pack_content() {
        // Verify that after a save, the pointer's digest matches the pack content.
        let root = scratch("pointer-pack-match");
        let mut store = Store::open(&root).expect("open");

        store.ingest("notes.dx", NOTES).expect("save");

        let stub_path = root.join("notes.dx");
        let stub_text = fs::read_to_string(&stub_path).expect("read stub");

        // Extract digest from stub
        let expected_digest = stub::digest_in(&stub_text).expect("stub should have digest");

        // Verify the source can be resolved and its digest matches
        let source = store.source("notes.dx").expect("source");
        let actual_digest = stub::digest_of(&source);

        assert_eq!(
            expected_digest, actual_digest,
            "pointer digest should match stored content digest"
        );
    }

    #[test]
    fn source_index_is_exported_when_source_files_and_tokens_exist() {
        let root = std::env::temp_dir().join("dx-source-index-test");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("scratch root");

        let mut store = Store::open(&root).expect("open");

        // Manually insert source files and tokens since the indexer doesn't populate them yet
        {
            let tx = store.connection_mut().transaction().expect("transaction");
            tx.execute(
                "INSERT INTO source_files (path, size, mtime) VALUES (?1, ?2, ?3)",
                ["src/main.rs", "100", "2025-01-01T00:00:00Z"],
            )
            .expect("insert file 1");
            tx.execute(
                "INSERT INTO source_files (path, size, mtime) VALUES (?1, ?2, ?3)",
                ["src/lib.rs", "200", "2025-01-01T00:00:00Z"],
            )
            .expect("insert file 2");

            tx.execute(
                "INSERT INTO source_tokens (path, token) VALUES (?1, ?2)",
                ["src/main.rs", "fn"],
            )
            .expect("insert token 1");
            tx.execute(
                "INSERT INTO source_tokens (path, token) VALUES (?1, ?2)",
                ["src/main.rs", "struct"],
            )
            .expect("insert token 2");
            tx.execute(
                "INSERT INTO source_tokens (path, token) VALUES (?1, ?2)",
                ["src/lib.rs", "impl"],
            )
            .expect("insert token 3");

            tx.commit().expect("commit");
        }

        // Export the source index
        store.export_source_index().expect("export");

        // Verify the file exists
        let source_index_path = root.join(pack::SOURCE_INDEX);
        assert!(
            source_index_path.exists(),
            "source_index file should be created at {}",
            source_index_path.display()
        );

        // Verify the content
        let content = fs::read_to_string(&source_index_path).expect("read file");
        assert!(content.contains("src/main.rs"), "should contain first file path");
        assert!(content.contains("src/lib.rs"), "should contain second file path");
        assert!(content.contains("fn"), "should contain first token");
        assert!(content.contains("struct"), "should contain second token");
        assert!(content.contains("impl"), "should contain third token");
    }

    #[test]
    fn source_index_is_removed_when_there_are_no_source_files() {
        let root = std::env::temp_dir().join("dx-source-index-empty-test");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("scratch root");

        let mut store = Store::open(&root).expect("open");

        // Create the file first
        let source_index_path = root.join(pack::SOURCE_INDEX);
        fs::write(&source_index_path, "dummy content").expect("write dummy file");
        assert!(source_index_path.exists());

        // Export with no source files should remove it
        store.export_source_index().expect("export");

        assert!(
            !source_index_path.exists(),
            "source_index file should be removed when empty"
        );
    }
}
