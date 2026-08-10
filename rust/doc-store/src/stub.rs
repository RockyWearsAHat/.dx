//! The on-disk stub pointer that stands in for a document's content, and the path law
//! that decides what a document may be called.
//!
//! The pointer grammar itself is [`doc_core::pointer`] — the format crate owns it, because
//! the CLI, the editors, and the browser extension all have to recognize the same line, and
//! two recognizers that disagreed would be a surface showing a pointer where a document
//! belongs. This module is the store's door onto it: the same three operations, named the
//! way the store's callers speak, plus [`normalize_path`], which is store policy and lives
//! nowhere else.

pub use doc_core::pointer::{digest_in, digest_of, render};

/// Whether `text` is a stub pointer rather than document content.
#[must_use]
pub fn is_stub(text: &str) -> bool {
    doc_core::pointer::is_pointer(text)
}

/// Normalize a caller-supplied path to a clean `/`-separated workspace-relative `.dx` path.
///
/// Strips a leading separator, collapses `.` segments, and rejects any `..` traversal by
/// returning `None` — a document path may never escape the workspace. A path with no `.dx`
/// extension gains one.
#[must_use]
pub fn normalize_path(path: &str) -> Option<String> {
    let cleaned = path.trim().replace('\\', "/");
    let mut segments: Vec<&str> = Vec::new();
    for segment in cleaned.split('/') {
        match segment {
            "" | "." => {}
            ".." => return None,
            other => segments.push(other),
        }
    }
    let joined = segments.join("/");
    if joined.is_empty() {
        return None;
    }
    if joined.ends_with(".dx") {
        Some(joined)
    } else {
        Some(format!("{joined}.dx"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = "::paragraph id=p\nhello\n::end\n";

    #[test]
    fn the_store_writes_and_reads_the_format_crate_s_pointer() {
        // The grammar is tested where it lives; what matters here is that the store's door
        // opens onto that grammar and not a second one.
        let stub = render(SOURCE);
        assert!(is_stub(&stub));
        assert!(!is_stub(SOURCE));
        assert_eq!(digest_in(&stub).expect("digest"), digest_of(SOURCE));
    }

    #[test]
    fn paths_are_normalized_and_traversal_is_refused() {
        assert_eq!(normalize_path("notes").as_deref(), Some("notes.dx"));
        assert_eq!(normalize_path("/a/b/c.dx").as_deref(), Some("a/b/c.dx"));
        assert_eq!(normalize_path("a\\b\\c.dx").as_deref(), Some("a/b/c.dx"));
        assert_eq!(normalize_path("./a/./b.dx").as_deref(), Some("a/b.dx"));
        assert_eq!(normalize_path("../secrets.dx"), None);
        assert_eq!(normalize_path("a/../../etc/passwd"), None);
        assert_eq!(normalize_path("   "), None);
        assert_eq!(normalize_path(""), None);
    }
}
