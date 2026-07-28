//! The in-memory document model shared by the binary codec and the DOCSRC parser.
//!
//! A document is metadata plus an ordered list of typed blocks. The model mirrors the
//! TypeScript reference shape closely enough that the binary codec round-trips
//! byte-for-byte. Document metadata values are kept as their raw JSON text (exactly the
//! bytes the wire format stores), so the core needs no JSON dependency.

/// One item of a list or checklist block. `checked` is meaningful only for checklists.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Item {
    /// Whether a checklist item is ticked (ignored for plain list items).
    pub checked: bool,
    /// The item's text.
    pub text: String,
    /// Child items for nested bulleted/numbered lists (empty when there are none).
    pub nested: Vec<Item>,
}

/// A single document block. Unused fields stay empty for a given `kind`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Block {
    /// Block type: `heading`, `paragraph`, `bulleted-list`, `code`, `image`, etc.
    pub kind: String,
    /// Optional block identifier (`id=` attribute).
    pub id: String,
    /// Space-joined CSS class list (`class=` attribute), empty when none.
    pub class_name: String,
    /// Whether the block carries the boolean `hidden` attribute.
    pub hidden: bool,
    /// `type=` attribute of a `script` block (e.g. `text/javascript`), empty otherwise.
    pub script_type: String,
    /// Whether a `script` block carries the boolean `module` attribute.
    pub module: bool,
    /// Heading level, 1..=4 (only for `heading`).
    pub level: u8,
    /// Inline/body text (paragraph, heading, quote, code, style, svg, html, …).
    pub text: String,
    /// Programming language (only for `code`).
    pub language: String,
    /// Image source (only for `image`).
    pub src: String,
    /// Image alt text (only for `image`).
    pub alt: String,
    /// Stylesheet href (only for `stylesheet`).
    pub href: String,
    /// Stylesheet media query (only for `stylesheet`).
    pub media: String,
    /// List/checklist items.
    pub items: Vec<Item>,
    /// Whether a `code` block is executable (the bare `run` attribute).
    pub run: bool,
    /// Libraries an executable `code` block needs (`deps="numpy requests"`).
    pub deps: String,
    /// Seconds an executable `code` block may run before it is killed; `0` means default.
    pub timeout: u32,
    /// Id of the `code` block an `output` block reports on (`for=` attribute).
    pub for_block: String,
    /// Result of the run recorded on an `output` block: `ok` or `error`.
    pub status: String,
    /// Process exit code recorded on an `output` block.
    pub exit: i32,
    /// Fingerprint of the code + dependencies that produced an `output` block, so a
    /// re-run can tell a still-current result from a stale one.
    pub hash: String,
    /// How a run's output should be displayed: empty for plain text, or `svg` / `html`
    /// when the code prints markup that should be rendered as a picture rather than
    /// quoted as source. Set on the `code` block; carried onto its `output`.
    pub format: String,
}

/// Names of the language runners the platform can execute, in catalogue order.
///
/// A `code` block marked `run` is executable when [`runner_for_language`] maps its
/// `language` to one of these.
pub const RUNNERS: &[&str] = &["python", "node", "bash", "rust", "go", "ruby", "deno"];

/// Map a `code` block's `language` to the runner that executes it, or `None` when the
/// language has no runner and the block is display-only.
///
/// Aliases are folded here so authors can write the language name they know: `py` and
/// `python3` both run under `python`, `js`/`javascript`/`ts`/`typescript` under `node`,
/// `sh`/`shell`/`zsh` under `bash`, and `rs` under `rust`.
#[must_use]
pub fn runner_for_language(language: &str) -> Option<&'static str> {
    match language.trim().to_ascii_lowercase().as_str() {
        "python" | "python3" | "py" => Some("python"),
        "node" | "js" | "javascript" | "mjs" => Some("node"),
        "ts" | "typescript" => Some("deno"),
        "deno" => Some("deno"),
        "bash" | "sh" | "shell" | "zsh" => Some("bash"),
        "rust" | "rs" => Some("rust"),
        "go" | "golang" => Some("go"),
        "ruby" | "rb" => Some("ruby"),
        _ => None,
    }
}

/// A complete document: front-matter metadata plus ordered blocks.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Document {
    /// Document title (may be derived from the first heading).
    pub title: String,
    /// Short summary line.
    pub summary: String,
    /// Tag list.
    pub tags: Vec<String>,
    /// Metadata entries as `(key, raw_json_value)` pairs, preserving insertion order.
    pub meta: Vec<(String, String)>,
    /// Ordered document blocks.
    pub blocks: Vec<Block>,
}

impl Document {
    /// Text of the first `heading` block, or empty when there is none.
    pub fn first_heading_text(&self) -> &str {
        for block in &self.blocks {
            if block.kind == "heading" {
                return &block.text;
            }
        }
        ""
    }
}
