//! The [`DocStore`] trait implementation for [`FsDocStore`].
//!
//! These methods are the public store surface (list/search/get/create/save/ingest/
//! render). Each delegates to the private bundle/persistence helpers on [`FsDocStore`]
//! (in the parent module) and the path/identity/render helpers in the sibling modules,
//! keeping the trait impl a thin, readable orchestration layer.

use doc_core::format::parse;
use doc_core::model::Document as CoreDocument;
use doc_core::search::build_index;

use crate::store::{CreateSpec, DocStore, DocSummary, Document, StoreError};

use super::render::render_document_html;
use super::stub::{derive_title, is_stub, normalize_relative_path, slug, stable_document_id};
use super::FsDocStore;

impl DocStore for FsDocStore {
    fn list(&self, query: &str, limit: usize) -> Result<Vec<DocSummary>, StoreError> {
        let loaded = self.load_all()?;
        let needle = query.trim();
        if needle.is_empty() {
            // No query: list every document, sorted by path for deterministic output.
            let mut summaries: Vec<DocSummary> = loaded
                .iter()
                .map(|doc| self.to_full(doc).summary())
                .collect();
            summaries.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
            summaries.truncate(limit);
            return Ok(summaries);
        }
        self.search(needle, limit)
    }

    fn search(&self, query: &str, limit: usize) -> Result<Vec<DocSummary>, StoreError> {
        let loaded = self.load_all()?;
        let indexed: Vec<(String, CoreDocument)> = loaded
            .iter()
            .map(|doc| (doc.relative_path.clone(), doc.document.clone()))
            .collect();
        let index = build_index(&indexed);

        let mut summaries = Vec::new();
        for hit in index.search(query) {
            if let Some(doc) = loaded.iter().find(|doc| doc.relative_path == hit.path) {
                summaries.push(self.to_full(doc).summary());
            }
            if summaries.len() >= limit {
                break;
            }
        }
        Ok(summaries)
    }

    fn get_by_path(&self, path: &str) -> Result<Document, StoreError> {
        let path = path.trim();
        if path.is_empty() {
            return Err(StoreError::InvalidArgument("path is required".to_string()));
        }
        self.find_full(path)
    }

    fn get_by_id(&self, id: i64) -> Result<Document, StoreError> {
        let loaded = self.load_all()?;
        loaded
            .iter()
            .find(|doc| stable_document_id(&doc.relative_path) == id)
            .map(|doc| self.to_full(doc))
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

        let relative_path = if spec.path.trim().is_empty() {
            format!("documents/{}.dx", slug(&title))
        } else {
            normalize_relative_path(spec.path.trim())
        };
        if relative_path.is_empty() {
            return Err(StoreError::InvalidArgument(
                "path must stay inside the workspace".to_string(),
            ));
        }

        // Build the document from supplied content, or seed a heading from the title.
        let document = match spec.content {
            Some(content) if !content.trim().is_empty() => parse(&content),
            _ => parse(&format!("::heading level=1\n{title}\n::end\n")),
        };
        self.persist(&relative_path, &document)
    }

    fn save(&mut self, path: &str, source: &str) -> Result<Document, StoreError> {
        let path = path.trim();
        if path.is_empty() {
            return Err(StoreError::InvalidArgument("path is required".to_string()));
        }
        let relative_path = normalize_relative_path(path);
        if relative_path.is_empty() {
            return Err(StoreError::InvalidArgument(
                "path must stay inside the workspace".to_string(),
            ));
        }
        let document = parse(source);
        self.persist(&relative_path, &document)
    }

    fn ingest(&mut self) -> Result<serde_json::Value, StoreError> {
        let files = self.collect_dx_files()?;
        let mut ingested = 0usize;
        for relative_path in &files {
            let absolute = self.root.join(relative_path);
            let text = std::fs::read_to_string(&absolute).map_err(|err| {
                StoreError::Backend(format!("failed to read {}: {err}", absolute.display()))
            })?;

            let document = if is_stub(&text) {
                // A stub points back into the bundle; reuse the stored content if present,
                // otherwise treat the (empty) stub as an empty document.
                match self.find_core(relative_path)? {
                    Some(document) => document,
                    None => parse(""),
                }
            } else {
                // A full DOCSRC / legacy source file: parse it into a document.
                parse(&text)
            };
            self.persist(relative_path, &document)?;
            ingested += 1;
        }

        Ok(serde_json::json!({
            "ingested": ingested,
            "engine": "fs-bundle-dxlite",
        }))
    }

    fn render_html(&self, path: &str) -> Result<String, StoreError> {
        let loaded = self.load_all()?;
        let doc = loaded
            .iter()
            .find(|doc| doc.relative_path == path)
            .ok_or(StoreError::NotFound)?;
        Ok(render_document_html(
            &doc.document,
            &derive_title(&doc.document, &doc.relative_path),
        ))
    }
}
