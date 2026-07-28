//! The canonical DOCSRC writer: turn a [`Document`] into byte-exact `.dx` text.

use super::normalize::normalize_blocks;
use super::util::{clamp_heading_level_u8, format_attribute_value, js_trim, normalize_class_name};
use crate::model::{Block, Document, Item};

/// Build the `::type attrs` opening line for a normalized block. Port of `blockHeader`.
fn block_header(block: &Block) -> String {
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
            if block.run {
                attributes.push("run".to_string());
            }
            if !block.deps.is_empty() {
                attributes.push(format!("deps={}", format_attribute_value(&block.deps)));
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
        "image" => {
            if !block.src.is_empty() {
                attributes.push(format!("src={}", format_attribute_value(&block.src)));
            }
            format!("::image {}", attributes.join(" "))
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
fn list_lines(items: &[Item], depth: usize, out: &mut Vec<String>) {
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
        "bulleted-list" | "numbered-list" => {
            let mut lines = Vec::new();
            list_lines(&block.items, 0, &mut lines);
            lines.join("\n")
        }
        "image" => block.alt.clone(),
        "stylesheet" | "rule" => String::new(),
        // style/script and the paragraph/quote/code/svg/html/graph/mermaid default all use
        // the block's `text` field verbatim. Checklist falls here too — matching the
        // reference, which has no checklist case and so emits an empty body.
        _ => block.text.clone(),
    }
}

/// Serialize a [`Document`] into canonical DOCSRC text, byte-identical to the reference
/// `stringifyDocFile`. Only the document's blocks are written; metadata is not emitted.
///
/// The blocks are re-normalized first, so a document built from any source produces stable,
/// canonical output (unique ids, clamped heading levels, default paragraph when empty).
pub fn stringify(document: &Document) -> String {
    let normalized = normalize_blocks(&document.blocks);
    let chunks: Vec<String> = normalized
        .iter()
        .map(|block| {
            let header = block_header(block);
            let body = block_body(block);
            format!("{header}\n{body}\n::end")
        })
        .collect();
    let body = chunks.join("\n\n");
    format!("{body}\n")
}
