//! What a `::nav` block points at — resolved once, for every view.
//!
//! Navigation is one block:
//!
//! ```text
//! ::nav id=guide class=sidebar label="{n}. {name}"
//! - [Install](install.dx)
//! - format.dx#blocks
//! - #results
//! ::end
//! ```
//!
//! Two forms, and the second is the reason this exists. An entry written as
//! `[Name](target)` says exactly what it is called. An entry that is *just a target*
//! borrows its name from the block's `label` template, so a list of destinations needs no
//! hand-written names and stays right when they change.
//!
//! A nav block with an **empty body** is the document's own contents: its headings, in
//! order, nested by level. That is the whole feature for most documents — one block, no
//! maintenance.
//!
//! # Why resolution lives here and not in each renderer
//! HTML, Markdown, and the editor must agree on what a nav block *is* before they disagree
//! about how it looks. [`entries`] is the single answer; a renderer only decides markup.
//! It is also why resolution never reads another file: this crate has no filesystem (it
//! compiles to `wasm32`), and a name that depended on one would differ between the editor
//! and the CLI. A cross-document entry is named from its target text alone.

use crate::model::{Block, Document, Item};

/// The default name template: whatever the target is most specifically about.
const DEFAULT_LABEL: &str = "{name}";

/// One resolved navigation entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavEntry {
    /// Link target exactly as the author wrote it (`setup.dx`, `setup.dx#install`, `#id`).
    pub target: String,
    /// The name to show, after any template expansion.
    pub name: String,
    /// Nesting depth, `0` for a top-level entry.
    pub depth: usize,
}

/// Resolve every entry a `nav` block points at, in reading order.
///
/// Entries come from the block's own body; when it has none, from `document`'s headings —
/// skipping a leading level-1 heading, which is the document's title rather than a place
/// inside it. Depth is the body's indentation, or the heading level, so one renderer
/// handles both.
///
/// Complexity: `O(n)` in the number of entries or headings.
#[must_use]
pub fn entries(block: &Block, document: &Document) -> Vec<NavEntry> {
    let mut out = if block.items.is_empty() {
        contents_of(document)
    } else {
        let template = if block.label.trim().is_empty() {
            DEFAULT_LABEL
        } else {
            block.label.trim()
        };
        let mut entries = Vec::new();
        push_items(&block.items, 0, template, document, &mut entries);
        entries
    };

    flatten_to_top(&mut out);
    out
}

/// Rebase depths so the shallowest entry sits at `0`.
///
/// A document whose headings are all `level=2` would otherwise produce a contents list
/// that is entirely "nested" — indented under an item that does not exist.
fn flatten_to_top(entries: &mut [NavEntry]) {
    let Some(shallowest) = entries.iter().map(|entry| entry.depth).min() else {
        return;
    };
    for entry in entries {
        entry.depth -= shallowest;
    }
}

/// Walk a nav body's items, resolving each and recursing into nested ones.
fn push_items(
    items: &[Item],
    depth: usize,
    template: &str,
    document: &Document,
    out: &mut Vec<NavEntry>,
) {
    for item in items {
        let text = item.text.trim();
        if !text.is_empty() {
            let (name, target) = match split_link(text) {
                // `[Name](target)` — the author named it; the template does not apply.
                Some((name, target)) => (name.to_string(), target.to_string()),
                // A bare target takes its name from the template.
                None => (
                    expand(template, text, out.len() + 1, document),
                    text.to_string(),
                ),
            };
            out.push(NavEntry {
                target,
                name,
                depth,
            });
        }
        if !item.nested.is_empty() {
            push_items(&item.nested, depth + 1, template, document, out);
        }
    }
}

/// The document's own contents: every heading below the title, as `#id` entries.
fn contents_of(document: &Document) -> Vec<NavEntry> {
    let headings = document
        .blocks
        .iter()
        .enumerate()
        .filter(|(_, block)| block.kind == "heading");

    let mut out = Vec::new();
    for (index, block) in headings {
        // The first block being a level-1 heading makes it the document's title: a
        // contents list that begins with the title of the page it is on says nothing.
        if index == 0 && block.level <= 1 {
            continue;
        }
        out.push(NavEntry {
            target: format!("#{}", block.id),
            name: block.text.trim().to_string(),
            depth: usize::from(block.level.saturating_sub(1)),
        });
    }
    out
}

/// Split `[Name](target)` into its two halves, or `None` when `text` is a bare target.
///
/// The whole entry must be the link — a line with prose around a link is a bare target
/// as far as nav is concerned, because guessing which part is the destination is how a
/// broken link gets built silently.
fn split_link(text: &str) -> Option<(&str, &str)> {
    let rest = text.strip_prefix('[')?;
    let close = rest.find("](")?;
    let target = rest[close + 2..].strip_suffix(')')?;
    let name = &rest[..close];
    if target.contains(')') {
        return None;
    }
    Some((name.trim(), target.trim()))
}

/// Expand a `label` template for `target`, the `n`-th entry of the block.
///
/// Known tokens are `{name}`, `{target}`, `{path}`, and `{n}`. An unknown token is left
/// exactly as written: a name is prose an author chose, and quietly deleting part of it
/// is worse than showing a brace.
fn expand(template: &str, target: &str, n: usize, document: &Document) -> String {
    let (path, fragment) = split_target(target);
    let mut out = String::with_capacity(template.len());
    let mut rest = template;

    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else { break };
        let token = &after[..close];
        out.push_str(&rest[..open]);
        match token {
            "name" => out.push_str(&name_of(path, fragment, document)),
            "target" => out.push_str(target),
            "path" => out.push_str(path),
            "n" => out.push_str(&n.to_string()),
            other => {
                out.push('{');
                out.push_str(other);
                out.push('}');
            }
        }
        rest = &after[close + 1..];
    }

    out.push_str(rest);
    out
}

/// Split a target into its path and fragment halves (`setup.dx#install`).
fn split_target(target: &str) -> (&str, &str) {
    match target.split_once('#') {
        Some((path, fragment)) => (path, fragment),
        None => (target, ""),
    }
}

/// The most specific name a target carries.
///
/// A fragment pointing into *this* document is named by the heading it addresses, which is
/// the one case where the real title is available without reading a file. Otherwise the
/// fragment, then the file stem, then the raw path.
fn name_of(path: &str, fragment: &str, document: &Document) -> String {
    if path.is_empty() && !fragment.is_empty() {
        if let Some(block) = document.blocks.iter().find(|block| block.id == fragment) {
            if block.kind == "heading" {
                return block.text.trim().to_string();
            }
        }
    }
    if !fragment.is_empty() {
        return fragment.to_string();
    }
    let file = path.rsplit('/').next().unwrap_or(path);
    file.strip_suffix(".dx").unwrap_or(file).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::parse;

    fn nav_of(source: &str) -> Vec<NavEntry> {
        let document = parse(source);
        let block = document
            .blocks
            .iter()
            .find(|block| block.kind == "nav")
            .expect("a nav block");
        entries(block, &document)
    }

    #[test]
    fn a_named_entry_keeps_the_name_the_author_wrote() {
        let nav = nav_of("::nav id=n\n- [Install first](setup.dx)\n::end\n");
        assert_eq!(nav[0].name, "Install first");
        assert_eq!(nav[0].target, "setup.dx");
    }

    #[test]
    fn a_bare_target_is_named_by_the_default_template() {
        let nav = nav_of("::nav id=n\n- guides/setup.dx\n- api.dx#errors\n::end\n");
        assert_eq!(nav[0].name, "setup");
        assert_eq!(nav[1].name, "errors");
    }

    #[test]
    fn a_fragment_in_this_document_is_named_by_its_heading() {
        let nav = nav_of(
            "::heading level=1 id=title\nGuide\n::end\n\n::nav id=n\n- #results\n::end\n\n\
             ::heading level=2 id=results\nWhat came out\n::end\n",
        );
        assert_eq!(nav[0].name, "What came out");
    }

    #[test]
    fn the_label_template_customizes_every_borrowed_name() {
        let nav = nav_of("::nav id=n label=\"{n}. {name} ({target})\"\n- a.dx\n- b.dx\n::end\n");
        assert_eq!(nav[0].name, "1. a (a.dx)");
        assert_eq!(nav[1].name, "2. b (b.dx)");
    }

    #[test]
    fn an_unknown_token_is_shown_rather_than_dropped() {
        let nav = nav_of("::nav id=n label=\"{name} {nope}\"\n- a.dx\n::end\n");
        assert_eq!(nav[0].name, "a {nope}");
    }

    #[test]
    fn a_named_entry_ignores_the_template() {
        let nav = nav_of("::nav id=n label=\"{n}. {name}\"\n- [Kept](a.dx)\n::end\n");
        assert_eq!(nav[0].name, "Kept");
    }

    #[test]
    fn an_indented_entry_nests() {
        let nav = nav_of("::nav id=n\n- a.dx\n  - b.dx\n::end\n");
        assert_eq!((nav[0].depth, nav[1].depth), (0, 1));
    }

    #[test]
    fn an_empty_nav_is_the_documents_own_contents() {
        let nav = nav_of(
            "::heading level=1 id=title\nGuide\n::end\n\n::nav id=n\n::end\n\n\
             ::heading level=2 id=one\nFirst\n::end\n\n::heading level=3 id=two\nDeeper\n::end\n",
        );
        // Depths are relative: the shallowest heading is a top-level entry, whatever its
        // level, so a document of only level-2 headings is not indented under nothing.
        assert_eq!(
            nav,
            vec![
                NavEntry {
                    target: "#one".into(),
                    name: "First".into(),
                    depth: 0
                },
                NavEntry {
                    target: "#two".into(),
                    name: "Deeper".into(),
                    depth: 1
                },
            ]
        );
    }

    #[test]
    fn contents_of_a_document_with_no_headings_is_empty_not_invented() {
        let nav = nav_of("::paragraph id=p\nJust prose.\n::end\n\n::nav id=n\n::end\n");
        assert!(nav.is_empty());
    }

    #[test]
    fn prose_around_a_link_is_treated_as_a_target_not_guessed_at() {
        // Splitting "see [x](y) now" would invent a destination the author did not write.
        assert_eq!(split_link("see [x](y) now"), None);
        assert_eq!(split_link("[x](y)"), Some(("x", "y")));
    }

    #[test]
    fn a_non_ascii_name_survives_untouched() {
        let nav = nav_of("::nav id=n\n- [Ünïcode — ページ](a.dx)\n::end\n");
        assert_eq!(nav[0].name, "Ünïcode — ページ");
    }
}
