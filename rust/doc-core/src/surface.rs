//! What an editing surface may offer a person: the authoring vocabulary, and the node
//! geometry a board's boxes obey.
//!
//! `editor/surface/edit.js` runs a caret and a keyboard and asks the engine for everything
//! else — but a completion menu and a drag handle need a few of these facts *before* any
//! call goes out, so the surface carries them as constants. Carrying them is fine; deciding
//! them is not. They are decided here, once, from the same registries the format and the
//! renderer use, and [`vocabulary`] hands the whole set to a JavaScript host as JSON.
//!
//! The mirror in `edit.js` is pinned to this function by
//! `editor/vscode/test/vocabulary.test.mjs`, so a kind added to the format and forgotten in
//! the surface — or a minimum size changed on one side only — fails a suite instead of
//! shipping a menu that offers a block the engine will refuse and a drag that stops at a
//! size the renderer would not have chosen. Both numbers had already drifted apart once.

use crate::edit::AUTHORABLE;
use crate::render::board::{FIT_MAX, FIT_MIN_HEIGHT, FIT_MIN_WIDTH, FIT_PADDING};

/// Blocks a reader may open by clicking: the ones a person wrote.
///
/// Narrower than [`AUTHORABLE`] on purpose — `rule` and `board` are authored by a surface
/// but have no body a caret belongs in (a board is opened by clicking *inside* a node,
/// which opens that node's own block).
pub const EDITABLE: &[&str] = &[
    "paragraph",
    "heading",
    "quote",
    "code",
    "bulleted-list",
    "numbered-list",
    "checklist",
    "html",
    "svg",
    "mermaid",
    "view",
];

/// Blocks that are never edited, whatever they look like.
///
/// `output` is what the code produced — editing it would be writing down a result that was
/// never computed. `nav` is derived from the document's own headings. Both are windows onto
/// something else, and the way to change them is to change the something else.
pub const READ_ONLY: &[&str] = &["output", "nav"];

/// Blocks where Return types a newline instead of finishing: a line break is content in
/// these, so it takes a modifier to say "done".
pub const MULTILINE: &[&str] = &[
    "code",
    "bulleted-list",
    "numbered-list",
    "checklist",
    "html",
    "svg",
    "mermaid",
    "view",
];

/// Blocks whose body is source — a listing, a drawing, author markup. Their field is the
/// mono column and is never decorated: source means exactly what it spells.
pub const SOURCE: &[&str] = &["code", "html", "svg", "mermaid", "view"];

/// What the tag line can say: one entry per kind a person may write, each completing to a
/// line that is already a valid header — the attribute a kind cannot live without arrives
/// with it, caret ready for its value.
///
/// This is the *retype* vocabulary, which is wider than [`AUTHORABLE`]: retyping a
/// paragraph's header is how an `image` or a `view` is written, because those kinds live in
/// their attributes rather than in a body somebody types.
pub const KINDS: &[&str] = &[
    "::paragraph",
    "::heading level=2",
    "::quote",
    "::code lang=",
    "::bulleted-list",
    "::numbered-list",
    "::checklist",
    "::rule",
    "::image src=",
    "::html",
    "::svg",
    "::mermaid",
    "::board",
    "::view src=",
];

/// The attributes each kind's header may carry, beyond the universal `id`/`class`/`hidden`.
pub const ATTRS: &[(&str, &[&str])] = &[
    ("heading", &["level"]),
    (
        "code",
        &[
            "lang", "src", "run", "open", "deps", "reads", "writes", "timeout", "format",
        ],
    ),
    ("image", &["src"]),
    ("stylesheet", &["href", "media"]),
    ("script", &["type", "src", "module"]),
    ("nav", &["label"]),
    ("board", &["height"]),
    ("view", &["src", "width", "height"]),
];

/// Attributes that are flags: present or absent, never `key=value`.
pub const BARE_ATTRS: &[&str] = &["run", "open", "hidden", "module"];

/// The whole vocabulary and the surface geometry, as the JSON a JavaScript host reads.
///
/// Keys are camelCase because the reader is JavaScript. The numbers under `node` are the
/// renderer's own: a node dragged to the smallest box a reader can make comes out the size
/// `w=fit`/`h=fit` would have chosen, and a surface that re-fits a board magnifies no more
/// than a static render does.
pub fn vocabulary() -> String {
    let mut out = String::with_capacity(1024);
    out.push('{');
    write_list(&mut out, "editable", EDITABLE);
    out.push(',');
    write_list(&mut out, "readOnly", READ_ONLY);
    out.push(',');
    write_list(&mut out, "multiline", MULTILINE);
    out.push(',');
    write_list(&mut out, "source", SOURCE);
    out.push(',');
    write_list(&mut out, "authorable", AUTHORABLE);
    out.push(',');
    write_list(&mut out, "kinds", KINDS);
    out.push(',');
    write_list(&mut out, "bareAttrs", BARE_ATTRS);
    out.push_str(",\"attrs\":{");
    for (index, (kind, attrs)) in ATTRS.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        write_list(&mut out, kind, attrs);
    }
    out.push_str("},\"node\":{");
    out.push_str(&format!("\"minWidth\":{FIT_MIN_WIDTH},"));
    out.push_str(&format!("\"minHeight\":{FIT_MIN_HEIGHT},"));
    out.push_str(&format!("\"fitMax\":{FIT_MAX},"));
    out.push_str(&format!("\"padding\":{FIT_PADDING}"));
    out.push_str("}}");
    out
}

/// Write `"key":["a","b"]` into `out`.
///
/// Every string in this module is an ASCII identifier — a kind, an attribute, a `::` tag
/// line — so quoting is all the escaping they need, and
/// [`every_word_is_a_plain_identifier`](tests::every_word_is_a_plain_identifier) holds them
/// to it rather than leaving the assumption unstated.
fn write_list(out: &mut String, key: &str, words: &[&str]) {
    out.push('"');
    out.push_str(key);
    out.push_str("\":[");
    for (index, word) in words.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push('"');
        out.push_str(word);
        out.push('"');
    }
    out.push(']');
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::is_known_block_type;

    /// Every kind any list names is a kind the format understands — otherwise a surface
    /// offers a block that normalizes silently to `paragraph`.
    #[test]
    fn every_kind_named_here_is_one_the_format_keeps() {
        for kind in EDITABLE
            .iter()
            .chain(READ_ONLY)
            .chain(MULTILINE)
            .chain(SOURCE)
            .chain(AUTHORABLE)
        {
            assert!(is_known_block_type(kind), "`{kind}` is not a block type");
        }
        for (kind, _) in ATTRS {
            assert!(is_known_block_type(kind), "`{kind}` is not a block type");
        }
        for line in KINDS {
            let kind = line
                .trim_start_matches("::")
                .split_whitespace()
                .next()
                .expect("a tag line names a kind");
            assert!(is_known_block_type(kind), "`{kind}` is not a block type");
        }
    }

    /// An editable kind a person cannot also write is a surface that opens a block it could
    /// never have created — every one of them is authorable.
    #[test]
    fn everything_editable_can_also_be_written() {
        for kind in EDITABLE {
            assert!(AUTHORABLE.contains(kind), "`{kind}` cannot be authored");
        }
    }

    /// A kind cannot be both editable and read-only: the surface would have to choose, and
    /// which one it chose would depend on the order it checked.
    #[test]
    fn nothing_is_both_editable_and_read_only() {
        for kind in READ_ONLY {
            assert!(!EDITABLE.contains(kind), "`{kind}` is on both lists");
        }
    }

    /// Every flag attribute is one some kind actually carries.
    #[test]
    fn every_flag_belongs_to_a_kind_that_carries_it() {
        for flag in BARE_ATTRS {
            // `hidden` is universal — every kind may carry it — so it is not in ATTRS.
            if *flag == "hidden" {
                continue;
            }
            assert!(
                ATTRS.iter().any(|(_, attrs)| attrs.contains(flag)),
                "`{flag}` is a flag on no kind"
            );
        }
    }

    /// The JSON is written by hand, so the assumption that lets it skip escaping is held
    /// here rather than left to hold by luck.
    #[test]
    fn every_word_is_a_plain_identifier() {
        let words = EDITABLE
            .iter()
            .chain(READ_ONLY)
            .chain(MULTILINE)
            .chain(SOURCE)
            .chain(AUTHORABLE)
            .chain(KINDS)
            .chain(BARE_ATTRS)
            .chain(ATTRS.iter().map(|(kind, _)| kind))
            .chain(ATTRS.iter().flat_map(|(_, attrs)| attrs.iter()));
        for word in words {
            assert!(
                word.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '=' | ':' | ' ')),
                "`{word}` would need JSON escaping"
            );
        }
    }

    /// The shape a host parses: the keys it reads, and the geometry it drags against.
    #[test]
    fn the_json_carries_every_list_and_the_node_geometry() {
        let json = vocabulary();

        for key in [
            "editable",
            "readOnly",
            "multiline",
            "source",
            "authorable",
            "kinds",
            "bareAttrs",
            "attrs",
            "node",
        ] {
            assert!(
                json.contains(&format!("\"{key}\":")),
                "no `{key}` in {json}"
            );
        }
        assert!(json.contains("\"minWidth\":120"));
        assert!(json.contains("\"minHeight\":56"));
        assert!(json.contains("\"fitMax\":1.5"));
        assert!(json.contains("\"padding\":24"));
        assert!(json.contains("\"::view src=\""));
        assert!(json.contains("\"code\":[\"lang\""));
    }
}
