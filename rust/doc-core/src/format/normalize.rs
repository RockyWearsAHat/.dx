//! Block normalization shared by the DOCSRC writer and parser.
//!
//! Normalization resolves each block's type against the allow-list, assigns a deterministic
//! unique id, canonicalizes class/hidden, and shapes per-type fields. It is the single point
//! where a document built from any source becomes canonical, so the writer and parser agree.

use super::is_known_block_type;
use super::util::{clamp_heading_level_u8, js_trim, js_trim_end, normalize_class_name, IdRegistry};
use crate::model::{Block, Item};

/// Normalize a parsed block: resolve type, assign a unique id, canonicalize class/hidden,
/// and shape per-type fields. Port of `normalizeBlock`.
fn normalize_block(block: &Block, index: usize, registry: &mut IdRegistry) -> Block {
    let block_type = if is_known_block_type(&block.kind) {
        block.kind.clone()
    } else {
        "paragraph".to_string()
    };

    let id_seed = if !block.id.is_empty() {
        block.id.clone()
    } else if block_type == "heading" {
        block.text.clone()
    } else {
        format!("{block_type}-{}", index + 1)
    };
    let id = registry.ensure_unique(&id_seed);
    let class_name = normalize_class_name(&block.class_name);
    let hidden = block.hidden;

    let mut normalized = Block {
        kind: block_type.clone(),
        id,
        class_name,
        hidden,
        ..Block::default()
    };

    match block_type.as_str() {
        "heading" => {
            normalized.level = clamp_heading_level_u8(block.level);
            let text = js_trim(&block.text);
            normalized.text = if text.is_empty() {
                format!("Section {}", index + 1)
            } else {
                text.to_string()
            };
        }
        "bulleted-list" | "numbered-list" => {
            let items = normalize_list_items(&block.items, &block.text);
            normalized.items = if items.is_empty() {
                vec![Item {
                    text: "List item".to_string(),
                    ..Item::default()
                }]
            } else {
                items
            };
        }
        // Nav keeps whatever entries it has, including none: an empty nav renders this
        // document's contents, so inventing a placeholder entry would replace a working
        // table of contents with the word "List item".
        "nav" => {
            normalized.items = normalize_list_items(&block.items, &block.text);
            normalized.label = js_trim(&block.label).to_string();
        }
        "image" => {
            normalized.src = js_trim(&block.src).to_string();
            normalized.for_block = js_trim(&block.for_block).to_string();
            normalized.alt = js_trim(&block.alt).to_string();
        }
        "checklist" => {
            let items: Vec<Item> = block
                .items
                .iter()
                .map(|item| Item {
                    checked: item.checked,
                    text: js_trim(&item.text).to_string(),
                    ..Item::default()
                })
                .filter(|item| !item.text.is_empty())
                .collect();
            normalized.items = if items.is_empty() {
                vec![Item {
                    checked: false,
                    text: "Item".to_string(),
                    ..Item::default()
                }]
            } else {
                items
            };
        }
        "rule" => {}
        "svg" | "html" | "graph" | "mermaid" => {
            normalized.text = js_trim_end(&block.text).to_string();
        }
        "style" => {
            normalized.text = js_trim_end(&block.text).to_string();
            normalized.media = js_trim(&block.media).to_string();
        }
        // Board: the node lines verbatim, plus the one attribute the viewport carries.
        "board" => {
            normalized.text = js_trim_end(&block.text).to_string();
            normalized.height = block.height;
        }
        // View: the reference and its stated viewport; the body stays as written (the
        // stored form of a `src=` view is empty — hydration fills it, and is never saved).
        "view" => {
            normalized.src = js_trim(&block.src).to_string();
            normalized.width = block.width;
            normalized.height = block.height;
            normalized.text = js_trim_end(&block.text).to_string();
        }
        "stylesheet" => {
            normalized.href = if !block.href.is_empty() {
                js_trim(&block.href).to_string()
            } else {
                js_trim(&block.src).to_string()
            };
            normalized.media = js_trim(&block.media).to_string();
        }
        "script" => {
            normalized.script_type = js_trim(&block.script_type).to_string();
            normalized.src = js_trim(&block.src).to_string();
            normalized.module = block.module;
            normalized.text = js_trim_end(&block.text).to_string();
        }
        "code" => {
            normalized.text = js_trim(&block.text).to_string();
            normalized.language = js_trim(&block.language).to_string();
            normalized.src = js_trim(&block.src).to_string();
            normalized.run = block.run;
            normalized.deps = js_trim(&block.deps).to_string();
            normalized.reads = js_trim(&block.reads).to_string();
            normalized.writes = js_trim(&block.writes).to_string();
            normalized.timeout = block.timeout;
            normalized.format = js_trim(&block.format).to_string();
        }
        "output" => {
            normalized.for_block = js_trim(&block.for_block).to_string();
            normalized.status = js_trim(&block.status).to_string();
            normalized.exit = block.exit;
            normalized.hash = js_trim(&block.hash).to_string();
            normalized.format = js_trim(&block.format).to_string();
            normalized.text = js_trim_end(&block.text).to_string();
        }
        // paragraph, quote, and any unknown type folded to paragraph.
        _ => {
            normalized.text = js_trim(&block.text).to_string();
        }
    }

    normalized
}

/// Normalize list items: keep an explicit item list, or split the block's `text` on
/// newlines as a fallback, dropping blank items.
///
/// Nesting is normalized recursively so a child item is trimmed and blank-filtered exactly
/// like a top-level one, and a nested list survives the round-trip at full depth.
fn normalize_list_items(items: &[Item], text: &str) -> Vec<Item> {
    if items.is_empty() {
        return text
            .split('\n')
            .map(|line| Item {
                text: js_trim(line).to_string(),
                ..Item::default()
            })
            .filter(|item| !item.text.is_empty())
            .collect();
    }

    items
        .iter()
        .map(|item| Item {
            checked: false,
            text: js_trim(&item.text).to_string(),
            nested: normalize_list_items(&item.nested, ""),
        })
        .filter(|item| !item.text.is_empty())
        .collect()
}

/// Normalize a slice of blocks, assigning unique ids in order. An empty input yields the
/// single default paragraph the reference inserts. Port of `normalizeBlocks`.
pub(super) fn normalize_blocks(blocks: &[Block]) -> Vec<Block> {
    let mut registry = IdRegistry::default();
    let normalized: Vec<Block> = blocks
        .iter()
        .enumerate()
        .map(|(index, block)| normalize_block(block, index, &mut registry))
        .collect();

    if !normalized.is_empty() {
        return normalized;
    }

    let default = Block {
        kind: "paragraph".to_string(),
        text: "Start writing here.".to_string(),
        ..Block::default()
    };
    vec![normalize_block(&default, 0, &mut registry)]
}
