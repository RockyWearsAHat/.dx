//! The canonical DOCSRC writer: turn a [`Document`] into byte-exact `.dx` text.

use super::normalize::normalize_blocks;
use super::parse;
use super::util::{clamp_heading_level_u8, format_attribute_value, js_trim, normalize_class_name};
use crate::model::{Block, Document, Item};

/// Build the `::type attrs` opening line for a normalized block. Port of `blockHeader`.
///
/// Also the showing half of [`crate::edit::block_header`]: the header an editing surface
/// puts above a block's body is exactly the line this writer puts in the file, so what a
/// reader sees is what an unchanged save writes back.
pub(crate) fn block_header(block: &Block) -> String {
    let mut attributes: Vec<String> = vec![format!("id={}", format_attribute_value(&block.id))];

    let class_name = normalize_class_name(&block.class_name);
    if !class_name.is_empty() {
        attributes.push(format!("class={}", format_attribute_value(&class_name)));
    }

    if block.hidden {
        attributes.push("hidden".to_string());
    }

    match block.kind.as_str() {
        "heading" => {
            attributes.insert(0, format!("level={}", clamp_heading_level_u8(block.level)));
            format!("::heading {}", attributes.join(" "))
        }
        "code" => {
            if !block.language.is_empty() {
                attributes.push(format!("lang={}", format_attribute_value(&block.language)));
            }
            // Unset `src` adds nothing, so no existing `::code` reformats on its next save.
            if !block.src.is_empty() {
                attributes.push(format!("src={}", format_attribute_value(&block.src)));
            }
            if block.run {
                attributes.push("run".to_string());
            }
            // Unset `open` adds nothing, so no existing `::code` reformats on its next save.
            if block.open {
                attributes.push("open".to_string());
            }
            if block.actions {
                attributes.push("actions".to_string());
            }
            if !block.deps.is_empty() {
                attributes.push(format!("deps={}", format_attribute_value(&block.deps)));
            }
            if !block.reads.is_empty() {
                attributes.push(format!("reads={}", format_attribute_value(&block.reads)));
            }
            if !block.writes.is_empty() {
                attributes.push(format!("writes={}", format_attribute_value(&block.writes)));
            }
            if !block.target.is_empty() {
                attributes.push(format!("target={}", format_attribute_value(&block.target)));
            }
            if !block.setup.is_empty() {
                attributes.push(format!("setup={}", format_attribute_value(&block.setup)));
            }
            if !block.query.is_empty() {
                attributes.push(format!("query={}", format_attribute_value(&block.query)));
            }
            if block.width > 0 {
                attributes.push(format!("width={}", block.width));
            }
            if block.height > 0 {
                attributes.push(format!("height={}", block.height));
            }
            if block.timeout > 0 {
                attributes.push(format!("timeout={}", block.timeout));
            }
            if !block.format.is_empty() {
                attributes.push(format!("format={}", format_attribute_value(&block.format)));
            }
            format!("::code {}", attributes.join(" "))
        }
        "output" => {
            if !block.for_block.is_empty() {
                attributes.push(format!("for={}", format_attribute_value(&block.for_block)));
            }
            if !block.status.is_empty() {
                attributes.push(format!("status={}", format_attribute_value(&block.status)));
            }
            if block.exit != 0 {
                attributes.push(format!("exit={}", block.exit));
            }
            if !block.hash.is_empty() {
                attributes.push(format!("hash={}", format_attribute_value(&block.hash)));
            }
            if !block.format.is_empty() {
                attributes.push(format!("format={}", format_attribute_value(&block.format)));
            }
            format!("::output {}", attributes.join(" "))
        }
        "nav" => {
            if !block.label.is_empty() {
                attributes.push(format!("label={}", format_attribute_value(&block.label)));
            }
            format!("::nav {}", attributes.join(" "))
        }
        // Unset `for` adds nothing, so no existing `::image` reformats on its next save.
        "image" => {
            if !block.src.is_empty() {
                attributes.push(format!("src={}", format_attribute_value(&block.src)));
            }
            if !block.for_block.is_empty() {
                attributes.push(format!("for={}", format_attribute_value(&block.for_block)));
            }
            format!("::image {}", attributes.join(" "))
        }
        // Unset `src` and `media` add nothing, so no existing `::style` reformats on save.
        "style" => {
            if !block.src.is_empty() {
                attributes.push(format!("src={}", format_attribute_value(&block.src)));
            }
            if !block.media.is_empty() {
                attributes.push(format!("media={}", format_attribute_value(&block.media)));
            }
            format!("::style {}", attributes.join(" "))
        }
        "stylesheet" => {
            if !block.href.is_empty() {
                attributes.push(format!("href={}", format_attribute_value(&block.href)));
            }
            if !block.media.is_empty() {
                attributes.push(format!("media={}", format_attribute_value(&block.media)));
            }
            format!("::stylesheet {}", attributes.join(" "))
        }
        "board" => {
            if block.height > 0 {
                attributes.push(format!("height={}", block.height));
            }
            format!("::board {}", attributes.join(" "))
        }
        // Unset attributes serialize to nothing, so adding `view` reformats no document.
        "view" => {
            if !block.src.is_empty() {
                attributes.push(format!("src={}", format_attribute_value(&block.src)));
            }
            if block.width > 0 {
                attributes.push(format!("width={}", block.width));
            }
            if block.height > 0 {
                attributes.push(format!("height={}", block.height));
            }
            format!("::view {}", attributes.join(" "))
        }
        "script" => {
            if !block.script_type.is_empty() {
                attributes.push(format!(
                    "type={}",
                    format_attribute_value(&block.script_type)
                ));
            }
            if !block.src.is_empty() {
                attributes.push(format!("src={}", format_attribute_value(&block.src)));
            }
            if block.module {
                attributes.push("module".to_string());
            }
            format!("::script {}", attributes.join(" "))
        }
        other => format!("::{other} {}", attributes.join(" ")),
    }
}

/// Render nested list items as indented `- text` lines. Port of `blockBody`'s `toLines`.
///
/// Also the showing half of [`crate::edit::body`] for list blocks: the editable text of a
/// list is exactly the body lines the canonical writer would put in the file, so nesting is
/// visible in the field and survives the save.
pub(crate) fn list_lines(items: &[Item], depth: usize, out: &mut Vec<String>) {
    let indent = "  ".repeat(depth);
    for item in items {
        let text = js_trim(&item.text);
        if !text.is_empty() {
            out.push(format!("{indent}- {text}"));
        }
        if !item.nested.is_empty() {
            list_lines(&item.nested, depth + 1, out);
        }
    }
}

/// Build the body text (between header and `::end`) for a block. Port of `blockBody`.
fn block_body(block: &Block) -> String {
    match block.kind.as_str() {
        // Lists and nav share a body shape: indented `- text` lines. A nav with no
        // entries writes an empty body, which is how "show this document's contents"
        // survives the round-trip.
        "bulleted-list" | "numbered-list" | "nav" => {
            let mut lines = Vec::new();
            list_lines(&block.items, 0, &mut lines);
            lines.join("\n")
        }
        "checklist" => block
            .items
            .iter()
            .map(|item| {
                let marker = if item.checked { "[x]" } else { "[ ]" };
                format!("{marker} {}", js_trim(&item.text))
            })
            .collect::<Vec<String>>()
            .join("\n"),
        "image" => block.alt.clone(),
        "stylesheet" | "rule" => String::new(),
        // style/script and the paragraph/quote/code/svg/html/graph/mermaid default all use
        // the block's `text` field verbatim.
        _ => block.text.clone(),
    }
}

/// Escape every close token in a body, so a document can carry dx syntax as content.
///
/// A body line shaped like a close token (`::end` alone, or trailing after whitespace) would
/// end the block when read back, truncating the document at its first example. Writing one
/// backslash immediately before the token makes the line content again, and
/// [`super::parse::close_token`] takes exactly that one back off — so the ladder is reversible
/// at every depth and the round trip stays byte-for-byte.
///
/// Complexity: `O(n)` in the body's byte size.
fn escape_close_tokens(body: &str) -> String {
    let carries_token = body
        .as_bytes()
        .windows("::end".len())
        .any(|window| window.eq_ignore_ascii_case(b"::end"));
    if !carries_token {
        return body.to_string();
    }
    body.split('\n')
        .map(|line| match parse::close_token(line) {
            Some(token) => format!("{}\\{}", &line[..token.start], &line[token.start..]),
            None => line.to_string(),
        })
        .collect::<Vec<String>>()
        .join("\n")
}

/// The separator written between two canonical blocks.
pub const BLOCK_SEPARATOR: &str = "\n\n";

/// Serialize a [`Document`] into its canonical DOCSRC blocks, one string per block.
///
/// Each returned string is a complete `::type attrs` / body / `::end` unit with no
/// trailing newline. Joining them with [`BLOCK_SEPARATOR`] and appending a single `\n`
/// reproduces [`stringify`] exactly — that identity is what lets storage keep a document
/// as independent per-block pieces and still rebuild the canonical file byte-for-byte.
///
/// The blocks are re-normalized first, so a document built from any source produces
/// stable, canonical output (unique ids, clamped heading levels, default paragraph when
/// empty). Normalization happens once for the whole document, so ids stay unique across
/// the returned pieces.
///
/// Complexity: `O(n)` in the document's total byte size.
pub fn stringify_blocks(document: &Document) -> Vec<String> {
    normalize_blocks(&document.blocks)
        .iter()
        .map(|block| {
            let header = block_header(block);
            let body = escape_close_tokens(&block_body(block));
            format!("{header}\n{body}\n::end")
        })
        .collect()
}

/// Serialize a [`Document`] into canonical DOCSRC text, byte-identical to the reference
/// `stringifyDocFile`. Only the document's blocks are written; metadata is not emitted.
///
/// This is [`stringify_blocks`] joined with [`BLOCK_SEPARATOR`] plus a trailing newline;
/// the two share one implementation so a per-block round-trip can never drift from a
/// whole-document one.
pub fn stringify(document: &Document) -> String {
    let body = stringify_blocks(document).join(BLOCK_SEPARATOR);
    format!("{body}\n")
}
