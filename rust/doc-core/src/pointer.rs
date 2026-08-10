//! The one-line pointer a `.dx` file on disk actually holds.
//!
//! ```text
//! ~ dx1 3f786850e387550fdab836ed7e6dc881de23001b3f786850e387550fdab836ed
//! ```
//!
//! The content lives in the workspace store; the file is a pointer into it. The grammar is
//! here, in the format crate, because every surface has to agree on it: the store writes it,
//! the CLI expands it, an editor asks whether the file it opened is one, and a browser
//! extension decides from it whether the page it is looking at should be replaced by a
//! document. A second recognizer that answered differently would be a surface showing a
//! pointer where a document belongs — the one thing this project is built not to do.
//!
//! # Why the digest is in the pointer
//! It is the digest of the document's **canonical source**, so the file changes exactly when
//! the content changes. That is what lets git do its job on a pointer repository: `git status`
//! notices an edited document, each document keeps its own history, and the `dx` diff driver
//! can expand the pointer into the real text. A constant marker like a bare `~` would make
//! every content change invisible to git.
//!
//! The pointer is also self-verifying: resolving it checks the recovered source against the
//! digest, so a stale or mismatched store is reported rather than served.

use crate::digest::sha256_hex;

/// Marker that opens every pointer line, followed by the format tag and the digest.
pub const MARKER: &str = "~ dx1 ";

/// The digest of a canonical source, in the form a pointer records.
#[must_use]
pub fn digest_of(source: &str) -> String {
    sha256_hex(source.as_bytes())
}

/// Render the pointer text for a document whose canonical source is `source`.
///
/// The result is a single line with a trailing newline, so the file is a well-formed text
/// file that any editor will leave alone.
#[must_use]
pub fn render(source: &str) -> String {
    format!("{MARKER}{}\n", digest_of(source))
}

/// The digest recorded by a pointer, or `None` when `text` is not one.
///
/// Recognition is deliberately strict: a file is a pointer only when its first line is the
/// marker followed by 64 hex digits. Anything else — including a document that merely starts
/// with `~` — is real content to be ingested, never silently discarded.
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

/// Whether `text` is a pointer rather than document content.
#[must_use]
pub fn is_pointer(text: &str) -> bool {
    digest_in(text).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = "::paragraph id=p\nhello\n::end\n";

    #[test]
    fn a_pointer_is_one_short_line_carrying_the_content_digest() {
        let pointer = render(SOURCE);
        assert_eq!(pointer.lines().count(), 1);
        assert!(pointer.ends_with('\n'));
        assert!(
            pointer.len() < 80,
            "a pointer should stay tiny: {} bytes",
            pointer.len()
        );
        assert_eq!(digest_in(&pointer).expect("digest"), digest_of(SOURCE));
    }

    #[test]
    fn the_pointer_changes_when_the_content_changes() {
        // Without this, git could not see that a document was edited.
        let other = "::paragraph id=p\nhello there\n::end\n";
        assert_ne!(render(SOURCE), render(other));
    }

    #[test]
    fn real_content_is_never_mistaken_for_a_pointer() {
        assert!(!is_pointer(SOURCE));
        assert!(!is_pointer(""));
        assert!(!is_pointer("~"));
        assert!(!is_pointer("~ dx1 nothex"));
        // A document that happens to open with the marker text but a bad digest is content.
        assert!(!is_pointer("~ dx1 abc\n::paragraph id=p\nx\n::end\n"));
    }

    #[test]
    fn a_pointer_is_recognized_with_trailing_whitespace_or_upper_case_hex() {
        let digest = digest_of(SOURCE);
        let upper = format!("~ dx1 {}  \n", digest.to_uppercase());
        assert_eq!(digest_in(&upper).expect("digest"), digest);
    }
}
