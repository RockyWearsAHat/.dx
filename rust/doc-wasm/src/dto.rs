//! JSON data-transfer objects that mirror the [`doc_core::model`] types one-to-one.
//!
//! `doc-core` is deliberately serde-free, so the JavaScript boundary needs its own
//! serializable mirror of [`Document`], [`Block`], and [`Item`]. These DTOs use
//! `camelCase` field names (`className`, `scriptType`, …) so the JSON is idiomatic on the
//! JS side and needs no renaming there. Conversions in both
//! directions are total and lossless: `core -> dto -> core` reproduces the original
//! document exactly, which is what keeps `parse`/`stringify` round-trips stable.

use doc_core::model::{Block, Document, Item};
use serde::{Deserialize, Serialize};

/// Serializable mirror of [`doc_core::model::Item`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemDto {
    /// Whether a checklist item is ticked (ignored for plain list items).
    #[serde(default)]
    pub checked: bool,
    /// The item's text.
    #[serde(default)]
    pub text: String,
    /// Child items for nested lists (empty when there are none).
    #[serde(default)]
    pub nested: Vec<ItemDto>,
}

/// Serializable mirror of [`doc_core::model::Block`].
///
/// Field names are `camelCase` (`className`, `scriptType`) so the JSON crosses the
/// boundary in the shape a JavaScript host reads directly.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockDto {
    /// Block type (`heading`, `paragraph`, `code`, …). Serialized as `type`.
    #[serde(rename = "type", default)]
    pub kind: String,
    /// Optional block identifier (`id=` attribute).
    #[serde(default)]
    pub id: String,
    /// Space-joined CSS class list (`class=` attribute).
    #[serde(default)]
    pub class_name: String,
    /// Whether the block carries the boolean `hidden` attribute.
    #[serde(default)]
    pub hidden: bool,
    /// `type=` attribute of a `script` block.
    #[serde(default)]
    pub script_type: String,
    /// Whether a `script` block carries the boolean `module` attribute.
    #[serde(default)]
    pub module: bool,
    /// Heading level, 1..=4 (only for `heading`).
    #[serde(default)]
    pub level: u8,
    /// Inline/body text.
    #[serde(default)]
    pub text: String,
    /// Programming language (only for `code`).
    #[serde(default)]
    pub language: String,
    /// Image source (only for `image`).
    #[serde(default)]
    pub src: String,
    /// Image alt text (only for `image`).
    #[serde(default)]
    pub alt: String,
    /// Stylesheet href (only for `stylesheet`).
    #[serde(default)]
    pub href: String,
    /// Stylesheet media query (only for `stylesheet`).
    #[serde(default)]
    pub media: String,
    /// Name template for a `nav` entry given as a bare target (only for `nav`).
    #[serde(default)]
    pub label: String,
    /// List/checklist/nav items.
    #[serde(default)]
    pub items: Vec<ItemDto>,
    /// Whether a `code` block is executable (the bare `run` attribute).
    #[serde(default)]
    pub run: bool,
    /// Whether a `code` block's listing starts expanded (the bare `open` attribute).
    #[serde(default)]
    pub open: bool,
    /// Whether a `lang=capture` block's body is the action shorthand rather than raw
    /// JavaScript (the bare `actions` attribute).
    #[serde(default)]
    pub actions: bool,
    /// Libraries an executable `code` block needs.
    #[serde(default)]
    pub deps: String,
    /// Comma-separated sibling files an executable `code` block declares it reads.
    #[serde(default)]
    pub reads: String,
    /// Comma-separated folders an executable `code` block may write, inside the
    /// document's own folder.
    #[serde(default)]
    pub writes: String,
    /// The live URL a `lang=capture` block opens (`target=` attribute).
    #[serde(default)]
    pub target: String,
    /// A shell command a `lang=capture` block runs first, before it opens `target`
    /// (`setup=` attribute).
    #[serde(default)]
    pub setup: String,
    /// The dot-path a `lang=query` block extracts from the JSON it reads (`query=`
    /// attribute), e.g. `status.stepsDone` or `items[2].name`.
    #[serde(default)]
    pub query: String,
    /// Seconds an executable `code` block may run before it is killed; `0` means default.
    #[serde(default)]
    pub timeout: u32,
    /// Id of the `code` block an `output` block reports on.
    #[serde(default)]
    pub for_block: String,
    /// Result of the run recorded on an `output` block: `ok` or `error`.
    #[serde(default)]
    pub status: String,
    /// Process exit code recorded on an `output` block.
    #[serde(default)]
    pub exit: i32,
    /// Fingerprint of the code + dependencies that produced an `output` block.
    #[serde(default)]
    pub hash: String,
    /// How a run's output is displayed: empty, `svg`, or `html`.
    #[serde(default)]
    pub format: String,
    /// Viewport height in CSS pixels of a `board` block, the framed page of a `view`
    /// block, or the browser a `lang=capture` block opens; `0` means the default.
    #[serde(default)]
    pub height: u32,
    /// Viewport width in CSS pixels the framed page of a `view` block, or the browser a
    /// `lang=capture` block opens, is laid out at; `0` means the default.
    #[serde(default)]
    pub width: u32,
}

/// Serializable mirror of [`doc_core::model::Document`].
///
/// `meta` is preserved as ordered `(key, raw_json_value)` pairs — exactly the bytes the
/// binary codec stores — so it survives the boundary without a JSON re-encode that could
/// reorder keys or alter number formatting.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentDto {
    /// Document title (may be derived from the first heading).
    #[serde(default)]
    pub title: String,
    /// Short summary line.
    #[serde(default)]
    pub summary: String,
    /// Tag list.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Metadata entries as ordered `(key, raw_json_value)` pairs.
    #[serde(default)]
    pub meta: Vec<(String, String)>,
    /// Ordered document blocks.
    #[serde(default)]
    pub blocks: Vec<BlockDto>,
}

impl From<&Item> for ItemDto {
    fn from(item: &Item) -> Self {
        ItemDto {
            checked: item.checked,
            text: item.text.clone(),
            nested: item.nested.iter().map(ItemDto::from).collect(),
        }
    }
}

impl From<&ItemDto> for Item {
    fn from(dto: &ItemDto) -> Self {
        Item {
            checked: dto.checked,
            text: dto.text.clone(),
            nested: dto.nested.iter().map(Item::from).collect(),
        }
    }
}

impl From<&Block> for BlockDto {
    fn from(block: &Block) -> Self {
        BlockDto {
            kind: block.kind.clone(),
            id: block.id.clone(),
            class_name: block.class_name.clone(),
            hidden: block.hidden,
            script_type: block.script_type.clone(),
            module: block.module,
            level: block.level,
            text: block.text.clone(),
            language: block.language.clone(),
            src: block.src.clone(),
            alt: block.alt.clone(),
            href: block.href.clone(),
            media: block.media.clone(),
            label: block.label.clone(),
            items: block.items.iter().map(ItemDto::from).collect(),
            run: block.run,
            open: block.open,
            actions: block.actions,
            deps: block.deps.clone(),
            reads: block.reads.clone(),
            writes: block.writes.clone(),
            target: block.target.clone(),
            setup: block.setup.clone(),
            query: block.query.clone(),
            timeout: block.timeout,
            for_block: block.for_block.clone(),
            status: block.status.clone(),
            exit: block.exit,
            hash: block.hash.clone(),
            format: block.format.clone(),
            height: block.height,
            width: block.width,
        }
    }
}

impl From<&BlockDto> for Block {
    fn from(dto: &BlockDto) -> Self {
        Block {
            kind: dto.kind.clone(),
            id: dto.id.clone(),
            class_name: dto.class_name.clone(),
            hidden: dto.hidden,
            script_type: dto.script_type.clone(),
            module: dto.module,
            level: dto.level,
            text: dto.text.clone(),
            language: dto.language.clone(),
            src: dto.src.clone(),
            alt: dto.alt.clone(),
            href: dto.href.clone(),
            media: dto.media.clone(),
            label: dto.label.clone(),
            items: dto.items.iter().map(Item::from).collect(),
            run: dto.run,
            open: dto.open,
            actions: dto.actions,
            deps: dto.deps.clone(),
            reads: dto.reads.clone(),
            writes: dto.writes.clone(),
            target: dto.target.clone(),
            setup: dto.setup.clone(),
            query: dto.query.clone(),
            timeout: dto.timeout,
            for_block: dto.for_block.clone(),
            status: dto.status.clone(),
            exit: dto.exit,
            hash: dto.hash.clone(),
            format: dto.format.clone(),
            height: dto.height,
            width: dto.width,
        }
    }
}

impl From<&Document> for DocumentDto {
    fn from(doc: &Document) -> Self {
        DocumentDto {
            title: doc.title.clone(),
            summary: doc.summary.clone(),
            tags: doc.tags.clone(),
            meta: doc.meta.clone(),
            blocks: doc.blocks.iter().map(BlockDto::from).collect(),
        }
    }
}

impl From<&DocumentDto> for Document {
    fn from(dto: &DocumentDto) -> Self {
        Document {
            title: dto.title.clone(),
            summary: dto.summary.clone(),
            tags: dto.tags.clone(),
            meta: dto.meta.clone(),
            blocks: dto.blocks.iter().map(Block::from).collect(),
        }
    }
}
