//! The [`DocStore`] trait and an in-memory stub implementation.
//!
//! [`DocStore`] captures every document operation the MCP layer needs, decoupled from
//! storage. The protocol layer ([`crate::dispatch`]) talks only to this trait, so the
//! current [`MemoryStore`] stub can be swapped for a real `doc-core`-backed store
//! (bundle archives + dxlite) without touching the protocol code. Each method returns
//! [`Result`] so failures become JSON-RPC error responses rather than panics.

use std::collections::HashMap;

/// An error raised by a [`DocStore`] operation.
///
/// Variants distinguish *caller* mistakes (bad arguments, missing documents) from
/// *backend* failures so the dispatcher can map each to the correct JSON-RPC error
/// code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    /// The requested document does not exist.
    NotFound,
    /// A caller-supplied argument was missing or malformed (e.g. empty path).
    InvalidArgument(String),
    /// The backend failed to complete the operation.
    Backend(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "Document not found"),
            Self::InvalidArgument(message) => write!(f, "{message}"),
            Self::Backend(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for StoreError {}

/// Lightweight metadata for a document, returned by list/search operations.
///
/// This is the projection the MCP `list-documents`/`search-documents`/`resources/list`
/// methods need; it deliberately omits the full source body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocSummary {
    /// Stable numeric document id.
    pub id: i64,
    /// Human-readable document title.
    pub title: String,
    /// Workspace-relative `.dx` path that uniquely identifies the document.
    pub relative_path: String,
    /// ISO-8601 timestamp of the last update.
    pub updated_at: String,
}

/// A full document record including its DOCSRC source body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    /// Stable numeric document id.
    pub id: i64,
    /// Human-readable document title.
    pub title: String,
    /// Workspace-relative `.dx` path.
    pub relative_path: String,
    /// ISO-8601 timestamp of the last update.
    pub updated_at: String,
    /// Canonical DOCSRC source text for the document.
    pub source: String,
}

impl Document {
    /// Project this full record down to its [`DocSummary`] metadata.
    #[must_use]
    pub fn summary(&self) -> DocSummary {
        DocSummary {
            id: self.id,
            title: self.title.clone(),
            relative_path: self.relative_path.clone(),
            updated_at: self.updated_at.clone(),
        }
    }
}

/// Specification for creating a document.
///
/// Mirrors the `create-document` tool arguments from the reference server.
#[derive(Debug, Clone, Default)]
pub struct CreateSpec {
    /// Workspace-relative `.dx` path; the store may derive one when empty.
    pub path: String,
    /// Document title.
    pub title: String,
    /// Optional document summary.
    pub summary: Option<String>,
    /// Document tags.
    pub tags: Vec<String>,
    /// Optional DOCSRC source to save immediately after creation.
    pub content: Option<String>,
}

/// The document operations required by the MCP protocol layer.
///
/// Implementations are the seam between the wire protocol and storage. The protocol
/// layer never assumes a concrete backend, so a real `doc-core`-backed store can
/// replace [`MemoryStore`] by implementing this trait alone.
pub trait DocStore {
    /// List documents, optionally filtered by a free-text `query`, capped at `limit`.
    ///
    /// An empty `query` lists everything. The reference server reuses one
    /// list-or-search routine for both `list-documents` and `search-documents`.
    fn list(&self, query: &str, limit: usize) -> Result<Vec<DocSummary>, StoreError>;

    /// Search documents by `query`, capped at `limit`. Equivalent to [`Self::list`]
    /// with a non-empty query; kept distinct so backends may rank differently.
    fn search(&self, query: &str, limit: usize) -> Result<Vec<DocSummary>, StoreError>;

    /// Fetch a full document by workspace-relative path.
    fn get_by_path(&self, path: &str) -> Result<Document, StoreError>;

    /// Fetch a full document by numeric id.
    fn get_by_id(&self, id: i64) -> Result<Document, StoreError>;

    /// Create a document from `spec`, returning the resulting record.
    fn create(&mut self, spec: CreateSpec) -> Result<Document, StoreError>;

    /// Save full `source` for the document at `path`, returning the updated record.
    fn save(&mut self, path: &str, source: &str) -> Result<Document, StoreError>;

    /// Re-scan the workspace and ingest `.dx` files, returning a JSON-friendly report.
    fn ingest(&mut self) -> Result<serde_json::Value, StoreError>;

    /// Render the document at `path` to standalone HTML for the `docview://` resource.
    fn render_html(&self, path: &str) -> Result<String, StoreError>;
}

/// An in-memory [`DocStore`] backing the protocol end-to-end without real storage.
///
/// Documents live in a `HashMap` keyed by relative path. This lets the JSON-RPC layer
/// be fully exercised in tests and interactively before the `doc-core` wiring lands.
#[derive(Debug, Default)]
pub struct MemoryStore {
    documents: HashMap<String, Document>,
    next_id: i64,
}

impl MemoryStore {
    /// Create an empty store. The first created document gets id `1`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            documents: HashMap::new(),
            next_id: 1,
        }
    }

    /// Deterministic placeholder timestamp for stub records.
    ///
    /// A real store records true update times; the stub uses a fixed value so tests
    /// stay reproducible.
    fn stub_timestamp() -> String {
        "1970-01-01T00:00:00.000Z".to_string()
    }

    /// Collect summaries for documents whose title or path contains `query`
    /// (case-insensitive); an empty query matches all. Results are sorted by path for
    /// deterministic ordering, then truncated to `limit`.
    fn collect(&self, query: &str, limit: usize) -> Vec<DocSummary> {
        let needle = query.trim().to_lowercase();
        let mut matches: Vec<DocSummary> = self
            .documents
            .values()
            .filter(|doc| {
                needle.is_empty()
                    || doc.title.to_lowercase().contains(&needle)
                    || doc.relative_path.to_lowercase().contains(&needle)
                    || doc.source.to_lowercase().contains(&needle)
            })
            .map(Document::summary)
            .collect();
        matches.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
        matches.truncate(limit);
        matches
    }
}

impl DocStore for MemoryStore {
    fn list(&self, query: &str, limit: usize) -> Result<Vec<DocSummary>, StoreError> {
        Ok(self.collect(query, limit))
    }

    fn search(&self, query: &str, limit: usize) -> Result<Vec<DocSummary>, StoreError> {
        Ok(self.collect(query, limit))
    }

    fn get_by_path(&self, path: &str) -> Result<Document, StoreError> {
        self.documents
            .get(path)
            .cloned()
            .ok_or(StoreError::NotFound)
    }

    fn get_by_id(&self, id: i64) -> Result<Document, StoreError> {
        self.documents
            .values()
            .find(|doc| doc.id == id)
            .cloned()
            .ok_or(StoreError::NotFound)
    }

    fn create(&mut self, spec: CreateSpec) -> Result<Document, StoreError> {
        let title = {
            let trimmed = spec.title.trim();
            if trimmed.is_empty() {
                "Untitled".to_string()
            } else {
                trimmed.to_string()
            }
        };

        let path = if spec.path.trim().is_empty() {
            format!("documents/{}.dx", slug(&title))
        } else {
            spec.path.trim().to_string()
        };

        let source = spec.content.unwrap_or_default();
        let id = self.next_id;
        self.next_id += 1;

        let document = Document {
            id,
            title,
            relative_path: path.clone(),
            updated_at: Self::stub_timestamp(),
            source,
        };
        self.documents.insert(path, document.clone());
        Ok(document)
    }

    fn save(&mut self, path: &str, source: &str) -> Result<Document, StoreError> {
        let path = path.trim();
        if path.is_empty() {
            return Err(StoreError::InvalidArgument("path is required".to_string()));
        }

        if let Some(existing) = self.documents.get_mut(path) {
            existing.source = source.to_string();
            existing.updated_at = Self::stub_timestamp();
            return Ok(existing.clone());
        }

        // Saving an unknown path creates it, matching the reference save semantics.
        let id = self.next_id;
        self.next_id += 1;
        let document = Document {
            id,
            title: path.to_string(),
            relative_path: path.to_string(),
            updated_at: Self::stub_timestamp(),
            source: source.to_string(),
        };
        self.documents.insert(path.to_string(), document.clone());
        Ok(document)
    }

    fn ingest(&mut self) -> Result<serde_json::Value, StoreError> {
        Ok(serde_json::json!({
            "ingested": self.documents.len(),
            "engine": "memory-stub",
        }))
    }

    fn render_html(&self, path: &str) -> Result<String, StoreError> {
        let doc = self.get_by_path(path)?;
        // A minimal, escaped HTML shell; the real renderer lives in doc-core/doc-view.
        Ok(format!(
            "<!doctype html><html><head><title>{title}</title></head><body><pre>{body}</pre></body></html>",
            title = escape_html(&doc.title),
            body = escape_html(&doc.source),
        ))
    }
}

/// Slugify `value` into a lowercase, hyphen-separated identifier.
///
/// Mirrors `toSlug` in the reference server: non-alphanumerics collapse to single
/// hyphens, leading/trailing hyphens are trimmed, and an empty result becomes
/// `"untitled"`.
fn slug(value: &str) -> String {
    let mut out = String::new();
    let mut prev_hyphen = false;
    for ch in value.trim().to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            prev_hyphen = false;
        } else if !prev_hyphen {
            out.push('-');
            prev_hyphen = true;
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "untitled".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Escape the five XML/HTML special characters for safe inclusion in markup.
fn escape_html(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}
