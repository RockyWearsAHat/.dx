//! DOCSRC — the canonical, human/AI-shared text serialization of a [`Document`].
//!
//! `docs/dx-format-contract.md` is the authority on the behavior; this module is its
//! implementation, and owns two halves of the format contract:
//!
//! - [`stringify`] — the **canonical writer**: turns a [`Document`] into DOCSRC text. One
//!   `::type attrs` opening line per block, body lines as-is, a standalone `::end`, one
//!   blank line between blocks, and a trailing newline. It never emits synthetic
//!   `paragraph-N` wrappers or single-line `::x ... ::end` blocks.
//! - [`parse`] — the **reader/normalizer**: turns DOCSRC (or `@doc` frontmatter, or legacy
//!   Markdown) into a normalized [`Document`], recovering malformed inline forms. It
//!   preserves `id`/`class`, list-item boundaries, and block order, and never turns a valid
//!   block into a paragraph of literal `::heading … ::end`.
//!
//! `::style`/`::stylesheet`/`::script` blocks are presentation-only and carry no searchable
//! text in the model.
//!
//! Round-trip determinism is the contract: `parse` then `stringify` converges to canonical
//! form, and `stringify` is idempotent on already-canonical input. See
//! `docs/dx-format-contract.md` for the authoritative behavior spec.
//!
//! # Module layout
//! The implementation is split into cohesive submodules, all private except for the two
//! re-exported entry points:
//!
//! - [`util`] — JS-semantic string/number leaf helpers and the unique-id registry.
//! - [`attrs`] — block-header attribute-string scanning.
//! - [`lines`] — per-line matchers (headers, inline blocks, list/checklist items).
//! - [`normalize`] — block normalization shared by the writer and parser.
//! - [`stringify`] — the canonical writer ([`stringify`]).
//! - [`parse`] — the DOCSRC block-assembly scanner.
//! - [`source`] — top-level strategy selection and the public reader ([`parse`]).
//! - [`legacy`] — the legacy-Markdown fallback parser.

mod attrs;
mod legacy;
mod lines;
mod normalize;
mod parse;
mod source;
mod stringify;
mod util;

pub use source::parse;
pub use stringify::{stringify, stringify_blocks, BLOCK_SEPARATOR};

// The list-line rules, shared with `crate::edit` so a surface shows a list as the lines the
// writer puts in the file and reads them back by the parser's own grammar — one definition
// of each rule, two directions.
pub(crate) use lines::{build_nested_list_structure, parse_checklist_line, parse_list_items};
pub(crate) use stringify::list_lines;

// The id rule itself, shared with `crate::edit` so a surface can ask what an id *would*
// become before the writer names it — one slugifier, not a second one that could disagree
// with the registry about which spellings are the same id.
pub(crate) use util::slugify_heading;

/// The block kinds DOCSRC understands. Any other `::type` opening normalizes to
/// `paragraph`, matching the reference `BLOCK_TYPES` allow-list.
const BLOCK_TYPES: &[&str] = &[
    "heading",
    "paragraph",
    "bulleted-list",
    "numbered-list",
    "quote",
    "code",
    "image",
    "checklist",
    "nav",
    "rule",
    "style",
    "stylesheet",
    "svg",
    "html",
    "graph",
    "mermaid",
    "script",
    "output",
];

/// Whether `kind` is a recognized DOCSRC block type.
fn is_known_block_type(kind: &str) -> bool {
    BLOCK_TYPES.contains(&kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse then stringify, the canonical recovery round-trip used by most assertions.
    fn round_trip(text: &str) -> String {
        stringify(&parse(text))
    }

    #[test]
    fn writes_canonical_heading_and_paragraph() {
        // Captured from the TS reference: stringifyDocFile(parseDocFile(...)).
        let input =
            "::heading level=1 id=h\nHi\n::end\n\n::paragraph id=intro\nHello world\n::end\n";
        assert_eq!(round_trip(input), input);
    }

    #[test]
    fn a_nav_block_round_trips_including_an_empty_one() {
        // The empty body is the feature — it means "this document's contents" — so the
        // writer must not fill it in, the way an empty list gets a placeholder item.
        let input = "::nav id=side class=sidebar label=\"{n}. {name}\"\n\
                     - [Setup](setup.dx)\n  - api.dx#errors\n::end\n\n::nav id=contents\n\n::end\n";
        assert_eq!(round_trip(input), input);
    }

    #[test]
    fn a_nav_without_a_label_template_writes_no_label_attribute() {
        // Additive rule: a new attribute serializes to nothing when unset, or every
        // existing document reformats on its next save.
        assert_eq!(
            round_trip("::nav id=n\n- a.dx\n::end\n"),
            "::nav id=n\n- a.dx\n::end\n"
        );
    }

    #[test]
    fn recovers_inline_single_line_block() {
        assert_eq!(
            round_trip("::heading level=2 id=h Hello there ::end\n"),
            "::heading level=2 id=h\nHello there\n::end\n"
        );
    }

    #[test]
    fn recovers_trailing_end_on_content_line_and_rewrites_lang() {
        assert_eq!(
            round_trip("::code id=c language=js\nconst x = 1; ::end\n"),
            "::code id=c lang=js\nconst x = 1;\n::end\n"
        );
    }

    #[test]
    fn a_line_that_merely_mentions_the_close_token_keeps_all_of_its_text() {
        // Writing *about* the format destroyed content: the recovery rule for a trailing
        // `::end` matched the token anywhere in a line, so a sentence explaining what `::end`
        // is was cut at the moment it said so, and the rest of the block went with it.
        let prose = "::paragraph id=p\nA block ends with `::end` on its own line.\n::end\n";
        assert_eq!(round_trip(prose), prose);

        // The same defect truncated a drawing at a label, taking every element after it.
        let drawing = "::svg id=s\n<text>::end</text>\n<line x1=\"0\"/>\n::end\n";
        assert_eq!(round_trip(drawing), drawing);

        // And the documented recovery still recovers: whitespace before, nothing after.
        assert_eq!(
            round_trip("::code id=c lang=js\nconst x = 1; ::end\n"),
            "::code id=c lang=js\nconst x = 1;\n::end\n"
        );
    }

    #[test]
    fn a_paragraph_with_a_generated_id_is_not_mistaken_for_a_wrapper() {
        // `paragraph-N` is exactly the id the writer assigns an unnamed paragraph. Treating
        // it as a legacy wrapper to unwrap meant writing a document and reading it back
        // destroyed the paragraph, replacing it with the default placeholder.
        let input = "::paragraph id=paragraph-12\nSome line here\n::end\n";
        assert_eq!(round_trip(input), input);
    }

    #[test]
    fn an_unnamed_paragraph_survives_a_write_then_read() {
        // The writer generates `paragraph-2` here; reading that back must return it intact.
        let written = round_trip("::heading level=1 id=h\nT\n::end\n\n::paragraph\nBody.\n::end\n");
        assert!(
            written.contains("Body."),
            "paragraph body lost on write: {written}"
        );
        assert_eq!(
            round_trip(&written),
            written,
            "write/read is not a fixed point"
        );
    }

    #[test]
    fn ignores_orphan_end() {
        assert_eq!(
            round_trip("::end\n::paragraph id=p\nHi\n::end\n"),
            "::paragraph id=p\nHi\n::end\n"
        );
    }

    #[test]
    fn folds_unknown_block_type_to_paragraph_without_literal_markers() {
        // Grill-me #1/#8: a recognized `::weird` opening must not survive as literal text.
        let out = round_trip("::weird id=w\nbody\n::end\n");
        assert_eq!(out, "::paragraph id=w\nbody\n::end\n");
        assert!(!out.contains("::weird"));
    }

    #[test]
    fn preserves_hidden_and_class_attributes() {
        assert_eq!(
            round_trip("::paragraph id=p hidden\nSecret\n::end\n"),
            "::paragraph id=p hidden\nSecret\n::end\n"
        );
        assert_eq!(
            round_trip("::paragraph id=p class=\"foo bar\"\nText\n::end\n"),
            "::paragraph id=p class=\"foo bar\"\nText\n::end\n"
        );
    }

    #[test]
    fn numbered_list_writes_dash_markers() {
        assert_eq!(
            round_trip("::numbered-list id=n\n1. first\n2. second\n::end\n"),
            "::numbered-list id=n\n- first\n- second\n::end\n"
        );
    }

    #[test]
    fn nested_list_keeps_every_item_at_its_depth() {
        // Every written item must come back: dropping the indented ones silently destroyed
        // list content on save.
        let input = "::bulleted-list id=l\n- a\n  - a1\n  - a2\n- b\n::end\n";
        assert_eq!(round_trip(input), input);
        assert_eq!(round_trip(&round_trip(input)), input);
    }

    #[test]
    fn deeply_nested_list_survives_and_is_idempotent() {
        let input = "::bulleted-list id=l\n- a\n  - a1\n    - a1a\n  - a2\n- b\n::end\n";
        assert_eq!(round_trip(input), input);
    }

    #[test]
    fn checklist_items_survive_a_save() {
        // The body used to be written empty, erasing every item in the list.
        let input = "::checklist id=c\n[x] done\n[ ] todo\n::end\n";
        assert_eq!(round_trip(input), input);
        let items = &parse(input).blocks[0].items;
        assert_eq!(items.len(), 2);
        assert!(items[0].checked);
        assert!(!items[1].checked);
    }

    #[test]
    fn script_module_round_trips() {
        assert_eq!(
            round_trip("::script id=s type=text/javascript module\nconsole.log(1)\n::end\n"),
            "::script id=s type=text/javascript module\nconsole.log(1)\n::end\n"
        );
    }

    #[test]
    fn image_writes_src_attr_and_alt_body() {
        assert_eq!(
            round_trip("::image id=i src=foo.png\nAlt text\n::end\n"),
            "::image id=i src=foo.png\nAlt text\n::end\n"
        );
    }

    #[test]
    fn empty_input_yields_default_paragraph() {
        assert_eq!(
            round_trip(""),
            "::paragraph id=paragraph-1\nStart writing here.\n::end\n"
        );
    }

    #[test]
    fn slugifies_and_clamps_heading() {
        assert_eq!(
            round_trip("::heading level=9 id=\"my heading\"\nHello, World! 123\n::end\n"),
            "::heading level=4 id=my-heading\nHello, World! 123\n::end\n"
        );
        assert_eq!(
            round_trip("::heading level=abc id=h\nT\n::end\n"),
            "::heading level=1 id=h\nT\n::end\n"
        );
    }

    #[test]
    fn deduplicates_block_ids() {
        assert_eq!(
            round_trip("::paragraph id=p\na\n::end\n\n::paragraph id=p\nb\n::end\n"),
            "::paragraph id=p\na\n::end\n\n::paragraph id=p-2\nb\n::end\n"
        );
    }

    #[test]
    fn parses_doc_frontmatter_header() {
        let text = "@doc\ntitle: My Title\nsummary: A summary\ntags: a, b, c\nmeta.owner: alex\nmeta.count: 5\n---\n::heading level=1 id=h\nHi\n::end\n";
        let doc = parse(text);
        assert_eq!(doc.title, "My Title");
        assert_eq!(doc.summary, "A summary");
        assert_eq!(doc.tags, vec!["a", "b", "c"]);
        assert_eq!(
            doc.meta,
            vec![
                ("owner".to_string(), "\"alex\"".to_string()),
                ("count".to_string(), "5".to_string()),
            ]
        );
        assert_eq!(stringify(&doc), "::heading level=1 id=h\nHi\n::end\n");
    }

    #[test]
    fn legacy_markdown_is_converted_to_blocks() {
        assert_eq!(
            round_trip("# Title\n\nA paragraph.\n\n- a\n- b\n"),
            "::heading level=1 id=title\nTitle\n::end\n\n::paragraph id=paragraph-2\nA paragraph.\n::end\n\n::bulleted-list id=bulleted-list-3\n- a\n- b\n::end\n"
        );
    }

    #[test]
    fn legacy_quote_and_code_fence() {
        assert_eq!(
            round_trip("> quoted line\n> second\n"),
            "::quote id=quote-1\nquoted line\nsecond\n::end\n"
        );
        assert_eq!(
            round_trip("```js\nconst x=1;\n```\n"),
            "::code id=code-1 lang=js\nconst x=1;\n::end\n"
        );
    }

    #[test]
    fn runnable_code_attributes_round_trip() {
        let input =
            "::code id=demo lang=python run deps=\"requests rich\" timeout=45\nprint(1)\n::end\n";
        assert_eq!(round_trip(input), input);
        let block = &parse(input).blocks[0];
        assert!(block.run);
        assert_eq!(block.deps, "requests rich");
        assert_eq!(block.timeout, 45);
    }

    #[test]
    fn plain_code_gains_no_execution_attributes() {
        // Non-runnable code must serialize exactly as before, or every existing doc drifts.
        let input = "::code id=c lang=js\nconst x = 1;\n::end\n";
        assert_eq!(round_trip(input), input);
    }

    #[test]
    fn output_block_round_trips_with_status_and_exit() {
        let input = "::output id=demo-output for=demo status=error exit=2\nboom\n::end\n";
        assert_eq!(round_trip(input), input);
        let block = &parse(input).blocks[0];
        assert_eq!(block.for_block, "demo");
        assert_eq!(block.status, "error");
        assert_eq!(block.exit, 2);
    }

    #[test]
    fn successful_output_omits_the_default_exit_code() {
        assert_eq!(
            round_trip("::output id=o for=demo status=ok exit=0\nhi\n::end\n"),
            "::output id=o for=demo status=ok\nhi\n::end\n"
        );
    }

    #[test]
    fn drawing_blocks_declare_and_carry_their_output_format() {
        let input = "::code id=chart lang=python run format=svg\nprint(1)\n::end\n";
        assert_eq!(round_trip(input), input);
        assert_eq!(parse(input).blocks[0].format, "svg");

        let output = "::output id=o for=chart status=ok format=svg\n<svg></svg>\n::end\n";
        assert_eq!(round_trip(output), output);
    }

    #[test]
    fn parse_then_stringify_is_idempotent() {
        let once = round_trip("::heading level=1 id=h\nHi\n::end\n");
        assert_eq!(round_trip(&once), once);
    }

    /// Byte-exact canonical output for each real example and document, captured from this
    /// crate with `dx fmt`.
    ///
    /// These are captured output, not scripture. Regenerating one is correct when a defect is
    /// being fixed — several of them once encoded genuine data loss — but a fixture edited to
    /// make a convenient change pass is how a regression gets blessed. Refresh the pair, read
    /// the diff, and say what changed and why.
    const REAL_DOC_CASES: &[(&str, &str)] = &[
        (
            include_str!("../../tests/fixtures/welcome.input.dx"),
            include_str!("../../tests/fixtures/welcome.expected.dx"),
        ),
        (
            include_str!("../../tests/fixtures/tutorial.input.dx"),
            include_str!("../../tests/fixtures/tutorial.expected.dx"),
        ),
        (
            include_str!("../../tests/fixtures/block-reference.input.dx"),
            include_str!("../../tests/fixtures/block-reference.expected.dx"),
        ),
        (
            include_str!("../../tests/fixtures/compactness-comparison.input.dx"),
            include_str!("../../tests/fixtures/compactness-comparison.expected.dx"),
        ),
        (
            include_str!("../../tests/fixtures/footprint-pair.input.dx"),
            include_str!("../../tests/fixtures/footprint-pair.expected.dx"),
        ),
        (
            include_str!("../../tests/fixtures/compact-proof.input.dx"),
            include_str!("../../tests/fixtures/compact-proof.expected.dx"),
        ),
    ];

    #[test]
    fn real_documents_round_trip_byte_identical() {
        for (raw, expected) in REAL_DOC_CASES {
            let produced = stringify(&parse(raw));
            assert_eq!(
                &produced, expected,
                "canonical output diverged from the TS reference for a real document"
            );
            // And idempotent: re-parsing canonical output reproduces it.
            assert_eq!(&stringify(&parse(&produced)), expected);
        }
    }
}
