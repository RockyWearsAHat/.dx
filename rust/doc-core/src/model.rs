//! The in-memory document model every view and every store path shares.
//!
//! A document is metadata plus an ordered list of typed blocks. Nothing here is a wire
//! format: a block is stored as the canonical DOCSRC text `format::stringify` writes for
//! it, so no field can be lost to a codec that had not heard of it. Document metadata
//! values are kept as their raw JSON text, so the core needs no JSON dependency.

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
    /// Sibling-file reference: an `image` source, or the `src=` of a `code`, `view`, or
    /// `style` block whose content is the named file's current text (`resolve::hydrate`).
    pub src: String,
    /// Image alt text (only for `image`).
    pub alt: String,
    /// Stylesheet href (only for `stylesheet`).
    pub href: String,
    /// Stylesheet media query (only for `stylesheet`).
    pub media: String,
    /// Name template for a `nav` entry that gives a target but no name (only for `nav`),
    /// e.g. `label="{n}. {name}"`. Empty means the default, `{name}`.
    pub label: String,
    /// List/checklist items.
    pub items: Vec<Item>,
    /// Whether a `code` block is executable (the bare `run` attribute).
    pub run: bool,
    /// Whether a `code` block's listing starts expanded (the bare `open` attribute).
    /// The fold is still there — a reader can close it — but the page opens showing the
    /// listing, for the document whose code *is* the content.
    pub open: bool,
    /// Libraries an executable `code` block needs (`deps="numpy requests"`).
    pub deps: String,
    /// Comma-separated sibling files an executable `code` block reads at run time
    /// (`reads=site/index.html,site/site.css`). Each path obeys the reference path law,
    /// and the files' current text is part of the run's fingerprint — so editing a
    /// declared file makes the recorded output stale exactly like editing the code would.
    pub reads: String,
    /// Comma-separated folders inside the document's own folder an executable `code`
    /// block may write at run time (`writes=target,generated`). Each path obeys the
    /// reference path law, and the grant is part of the run's fingerprint — changing
    /// what a block may touch re-opens review exactly like changing its code would.
    /// Unset means the block writes only its own sandbox directory.
    pub writes: String,
    /// Seconds an executable `code` block may run before it is killed; `0` means default.
    pub timeout: u32,
    /// Id of the `code` block an `output` block reports on (`for=` attribute). On an
    /// `image`, the runnable block whose run produces the pictured file: the render
    /// vouches for the picture only while that block's recorded output says `ok`, and
    /// calls the image out otherwise — pixels prove nothing by themselves.
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
    /// Viewport height in CSS pixels of a `board` block, the framed page of a `view`
    /// block, or the browser a `lang=capture` block opens; `0` means the renderer's (or
    /// capture's own) default.
    pub height: u32,
    /// Viewport width in CSS pixels the framed page of a `view` block, or the browser a
    /// `lang=capture` block opens, is laid out at; `0` means the default. For a `view`
    /// the *displayed* width is the column or the board node's stated box — the frame is
    /// scaled into it uniformly; a capture's screenshot is saved at this width exactly.
    pub width: u32,
    /// The live URL a `lang=capture` block opens (`target=` attribute), never resolved
    /// as a sibling file the way `src` is. Empty for every other `code` block.
    pub target: String,
    /// A shell command a `lang=capture` block runs first, network allowed, before it
    /// opens `target` — most often what starts the server `target` names (`setup=`
    /// attribute). Empty for every other `code` block, and optional even for capture.
    pub setup: String,
    /// Whether a `lang=capture` block's body is the action shorthand (`wait`, `click`,
    /// `hover`, `type`, `key`, `scroll`, `eval` — the bare `actions` attribute) rather
    /// than raw JavaScript. Ignored outside `lang=capture`.
    pub actions: bool,
}

/// Names of the language runners the platform can execute, in catalogue order.
///
/// A `code` block marked `run` is executable when [`runner_for_language`] maps its
/// `language` to one of these.
pub const RUNNERS: &[&str] = &[
    "python",
    "node",
    "typescript",
    "bash",
    "rust",
    "go",
    "ruby",
    "deno",
    "c",
    "cpp",
    "java",
    "swift",
    "capture",
];

/// Map a `code` block's `language` to the runner that executes it, or `None` when the
/// language has no runner and the block is display-only.
///
/// Aliases are folded here so authors can write the language name they know: `py` and
/// `python3` both run under `python`, `js`/`javascript` under `node`, `ts` under
/// `typescript` (the machine's own Node toolchain — `deno` stays its own explicit
/// choice), `sh`/`shell`/`zsh` under `bash`, `rs` under `rust`, and `c++`/`cc`/`cxx`
/// under `cpp`. `capture` names itself: it is not a language, it is dx's own driver for
/// a `target=` URL, so there is no alias to fold.
#[must_use]
pub fn runner_for_language(language: &str) -> Option<&'static str> {
    match language.trim().to_ascii_lowercase().as_str() {
        "python" | "python3" | "py" => Some("python"),
        "node" | "js" | "javascript" | "mjs" => Some("node"),
        "ts" | "typescript" => Some("typescript"),
        "deno" => Some("deno"),
        "bash" | "sh" | "shell" | "zsh" => Some("bash"),
        "rust" | "rs" => Some("rust"),
        "go" | "golang" => Some("go"),
        "ruby" | "rb" => Some("ruby"),
        "c" => Some("c"),
        "cpp" | "c++" | "cc" | "cxx" => Some("cpp"),
        "java" => Some("java"),
        "swift" => Some("swift"),
        "capture" => Some("capture"),
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
    /// Position of the block called `id`, or `None` when no block carries it.
    ///
    /// This is the document's own rule for what "the block called `id`" means, and every
    /// caller uses it: editing, rendering one block, and the command line all match ids the
    /// same way. Matching ignores case and a leading `#`, so `#Intro` and `intro` find the
    /// same block — a reader copying an id out of a heading link should not have to know the
    /// difference.
    #[must_use]
    pub fn block_index(&self, id: &str) -> Option<usize> {
        let wanted = id.trim().trim_start_matches('#').to_ascii_lowercase();
        self.blocks
            .iter()
            .position(|block| block.id.to_ascii_lowercase() == wanted)
    }

    /// Text of the first `heading` block, or empty when there is none.
    pub fn first_heading_text(&self) -> &str {
        for block in &self.blocks {
            if block.kind == "heading" {
                return &block.text;
            }
        }
        ""
    }

    /// The document's display title: its metadata title, else its first heading, else the
    /// file stem of `relative` (the caller's path for it — pass `""` to get `""` back and
    /// supply your own last resort).
    ///
    /// This is the one derivation every surface shares — the store's summaries, the CLI's
    /// listings, the rendered page's `<title>`. It lives here so the same document cannot
    /// carry different names depending on which surface said them.
    #[must_use]
    pub fn display_title(&self, relative: &str) -> String {
        for candidate in [self.title.trim(), self.first_heading_text().trim()] {
            if !candidate.is_empty() {
                return candidate.to_string();
            }
        }
        std::path::Path::new(relative).file_stem().map_or_else(
            || relative.to_string(),
            |stem| stem.to_string_lossy().into_owned(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_titles_fall_back_from_metadata_to_heading_to_file_stem() {
        let titled = Document {
            title: "Stated".to_string(),
            ..Document::default()
        };
        assert_eq!(titled.display_title("a/b.dx"), "Stated");

        let heading = Document {
            blocks: vec![Block {
                kind: "heading".to_string(),
                text: "First Heading".to_string(),
                ..Block::default()
            }],
            ..Document::default()
        };
        assert_eq!(heading.display_title("a/b.dx"), "First Heading");

        let bare = Document::default();
        assert_eq!(bare.display_title("a/plain-name.dx"), "plain-name");
        assert_eq!(bare.display_title(""), "", "no path, no invented name");
    }

    #[test]
    fn typescript_routes_to_the_node_toolchain_and_deno_stays_explicit() {
        assert_eq!(runner_for_language("ts"), Some("typescript"));
        assert_eq!(runner_for_language("typescript"), Some("typescript"));
        assert_eq!(runner_for_language("deno"), Some("deno"));
    }

    #[test]
    fn direct_toolchain_languages_route_with_their_aliases() {
        assert_eq!(runner_for_language("c"), Some("c"));
        for alias in ["cpp", "c++", "cc", "cxx"] {
            assert_eq!(runner_for_language(alias), Some("cpp"), "{alias}");
        }
        assert_eq!(runner_for_language("java"), Some("java"));
        assert_eq!(runner_for_language("swift"), Some("swift"));
    }

    #[test]
    fn every_routed_runner_is_in_the_catalogue() {
        for language in [
            "python", "js", "ts", "deno", "sh", "rust", "go", "ruby", "c", "c++", "java", "swift",
        ] {
            let runner = runner_for_language(language).expect(language);
            assert!(RUNNERS.contains(&runner), "{runner} missing from RUNNERS");
        }
    }

    #[test]
    fn an_unknown_language_has_no_runner() {
        assert_eq!(runner_for_language("css"), None);
        assert_eq!(runner_for_language(""), None);
    }
}
