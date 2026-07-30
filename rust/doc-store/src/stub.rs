//! The on-disk stub pointer that stands in for a document's content.
//!
//! A `.dx` file on disk is not the document — it is a one-line pointer into the store:
//!
//! ```text
//! ~ dx1 3f786850e387550fdab836ed7e6dc881de23001b3f786850e387550fdab836ed
//! ```
//!
//! # Why the hash is in the pointer
//! The digest is of the document's **canonical source**, so the stub changes exactly when
//! the content changes. That is what lets git do its job on a stub repository: `git status`
//! notices an edited document, each document gets its own history, and the `dx` diff driver
//! can expand the pointer into the real text (see the `textconv` command). A constant
//! marker like a bare `~` would make every content change invisible to git.
//!
//! The pointer is also self-verifying: resolving it checks the recovered source against the
//! digest, so a stale or mismatched store is reported rather than served.

use doc_core::digest::sha256_hex;

/// Marker that opens every stub line, followed by the format tag and the digest.
const MARKER: &str = "~ dx1 ";

/// The digest of a canonical source, in the form a stub records.
#[must_use]
pub fn digest_of(source: &str) -> String {
    sha256_hex(source.as_bytes())
}

/// Render the stub text for a document whose canonical source is `source`.
///
/// The result is a single line with a trailing newline, so the file is a well-formed text
/// file that any editor will leave alone.
#[must_use]
pub fn render(source: &str) -> String {
    format!("{MARKER}{}\n", digest_of(source))
}

/// The digest recorded by a stub, or `None` when `text` is not a stub.
///
/// Recognition is deliberately strict: a file is a stub only when its first line is the
/// marker followed by 64 hex digits. Anything else — including a document that merely
/// starts with `~` — is treated as real content to be ingested, never silently discarded.
#[must_use]
pub fn digest_in(text: &str) -> Option<String> {
    let first = text.lines().next()?.trim_end();
    let digest = first.strip_prefix(MARKER)?.trim();
    let is_hex = digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit());
    if is_hex {
        Some(digest.to_ascii_lowercase())
    } else {
        None
    }
}

/// Whether `text` is a stub pointer rather than document content.
#[must_use]
pub fn is_stub(text: &str) -> bool {
    digest_in(text).is_some()
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
    fn a_stub_is_one_short_line_carrying_the_content_digest() {
        let stub = render(SOURCE);
        assert_eq!(stub.lines().count(), 1);
        assert!(stub.ends_with('\n'));
        assert!(
            stub.len() < 80,
            "stub should stay tiny: {} bytes",
            stub.len()
        );
        assert_eq!(digest_in(&stub).expect("digest"), digest_of(SOURCE));
    }

    #[test]
    fn the_stub_changes_when_the_content_changes() {
        // Without this, git could not see that a document was edited.
        let other = "::paragraph id=p\nhello there\n::end\n";
        assert_ne!(render(SOURCE), render(other));
    }

    #[test]
    fn real_content_is_never_mistaken_for_a_stub() {
        assert!(!is_stub(SOURCE));
        assert!(!is_stub(""));
        assert!(!is_stub("~"));
        assert!(!is_stub("~ dx1 nothex"));
        // A document that happens to open with the marker text but a bad digest is content.
        assert!(!is_stub("~ dx1 abc\n::paragraph id=p\nx\n::end\n"));
    }

    #[test]
    fn a_stub_is_recognized_with_trailing_whitespace_or_upper_case_hex() {
        let digest = digest_of(SOURCE);
        let upper = format!("~ dx1 {}  \n", digest.to_uppercase());
        assert_eq!(digest_in(&upper).expect("digest"), digest);
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
