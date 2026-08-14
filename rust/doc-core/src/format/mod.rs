//! DOCSRC — the canonical, human/AI-shared text serialization of a [`Document`].
//!
//! `docs/dx-format-contract.dx` is the authority on the behavior; this module is its
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
//! `docs/dx-format-contract.dx` for the authoritative behavior spec.
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
mod layout;
mod legacy;
mod lines;
mod mermaid;
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

// The header rules, shared with `crate::edit` the same way: a block's header is the line
// the canonical writer puts in the file (`block_header`), and a typed header is read by
// the scanner's own grammar (`header_line_facts`) — never by a second parser grown in an
// editing surface.
pub(crate) use parse::header_line_facts;
pub(crate) use stringify::block_header;

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
    "board",
    "view",
    "graph",
    "mermaid",
    "script",
    "output",
];

/// Whether `kind` is a recognized DOCSRC block type.
///
/// Shared with `crate::edit`, which refuses a typed header naming a kind the format would
/// otherwise silently fold to `paragraph` — a retype that quietly became prose would be
/// content loss with no sentence saying so.
pub(crate) fn is_known_block_type(kind: &str) -> bool {
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

    /// A document has to be able to show dx syntax, or the format cannot document itself:
    /// the format contract, the block reference, and every tutorial that prints a block were
    /// all cut off at their first example, because a body line reading `::end` closed the
    /// block that was quoting it.
    #[test]
    fn a_block_can_carry_dx_syntax_because_the_writer_escapes_the_close_token() {
        let document = parse(
            "::code id=example lang=text\n::paragraph id=intro\nHello\n\\::end\n::end\n\n\
             ::paragraph id=after\nStill here.\n::end\n",
        );

        // The example survives whole — including its own closing line — and so does the
        // block after it, which the early close used to swallow.
        assert_eq!(document.blocks.len(), 2);
        assert_eq!(
            document.blocks[0].text,
            "::paragraph id=intro\nHello\n::end"
        );
        assert_eq!(document.blocks[1].text, "Still here.");

        // And the writer puts the escape back, so the file reads as itself next time.
        let written = stringify(&document);
        assert!(written.contains("\\::end\n::end"), "{written}");
        assert_eq!(stringify(&parse(&written)), written);
    }

    /// The escape is a ladder — one backslash per level — which is what makes it reversible:
    /// a document quoting a document quoting a block still reads back byte-for-byte.
    #[test]
    fn escaping_the_close_token_is_reversible_at_every_depth() {
        for body in ["::end", "\\::end", "\\\\::end", "} ::end", "::END"] {
            let mut document = parse("::code id=q lang=text\nx\n::end\n");
            document.blocks[0].text = body.to_string();
            let written = stringify(&document);
            assert_eq!(
                parse(&written).blocks[0].text,
                body,
                "body {body:?} did not survive {written:?}"
            );
            assert_eq!(stringify(&parse(&written)), written, "{written}");
        }
    }

    /// The narrow rule still holds: `::end` inside a sentence is prose, not a close token,
    /// and prose that merely mentions it must not grow a backslash on save.
    #[test]
    fn a_mention_of_the_close_token_mid_line_is_left_exactly_as_written() {
        let prose = "::paragraph id=p\na block ends with `::end` on its own line\n::end\n";
        assert_eq!(round_trip(prose), prose);
    }

    #[test]
    fn a_view_round_trips_and_unset_attributes_write_nothing() {
        // The stored form of a `src=` view is the reference and an empty body; `width`
        // and `height` appear only when stated, so adding the kind reformats no document.
        let full = "::view id=shipped src=site/index.html width=1180 height=760\n\n::end\n";
        assert_eq!(round_trip(full), full);
        let bare = "::view id=shipped src=site/index.html\n\n::end\n";
        assert_eq!(round_trip(bare), bare);
    }

    #[test]
    fn an_image_round_trips_its_producer_and_unset_for_writes_nothing() {
        // `for=` ties the picture to the run that produces its file. Additive rule: an
        // image that states no producer must not gain the attribute on its next save.
        let tied = "::image id=shot src=frames/one.png for=frames\nthe first frame\n::end\n";
        assert_eq!(round_trip(tied), tied);
        let bare = "::image id=shot src=frames/one.png\nthe first frame\n::end\n";
        assert_eq!(round_trip(bare), bare);
    }

    #[test]
    fn a_style_src_round_trips_and_a_srcless_style_is_unchanged() {
        // The stored form of a `src=` style is the reference and an empty body —
        // hydration fills it, and is never saved. Additive rule: a style block stating
        // no src must not gain the attribute on its next save.
        let referenced = "::style id=dress src=theme/site.css\n\n::end\n";
        assert_eq!(round_trip(referenced), referenced);
        let inline = "::style id=dress\np { color: red }\n::end\n";
        assert_eq!(round_trip(inline), inline);
    }

    #[test]
    fn an_open_code_block_round_trips_and_a_closed_one_is_unchanged() {
        // `open` starts the listing expanded — the author saying the code is the content.
        // Additive rule: a code block that does not state it must not gain the attribute
        // on its next save.
        let expanded = "::code id=q lang=sql open\nselect 1;\n::end\n";
        assert_eq!(round_trip(expanded), expanded);
        let folded = "::code id=q lang=sql\nselect 1;\n::end\n";
        assert_eq!(round_trip(folded), folded);
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
    fn a_bare_keyword_opening_an_inline_body_stays_prose() {
        // `run`/`open` are code attributes; on any other kind the word is the body's
        // first word, never a swallowed attribute — an inline heading beginning with
        // "Open" must keep it.
        assert_eq!(
            round_trip("::heading level=2 id=h Open questions ::end\n"),
            "::heading level=2 id=h\nOpen questions\n::end\n"
        );
        assert_eq!(
            round_trip("::paragraph id=p run with it ::end\n"),
            "::paragraph id=p\nrun with it\n::end\n"
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
    fn declared_reads_round_trip_and_unset_writes_nothing() {
        let input = "::code id=check lang=python run reads=site/index.html,site/site.css\n\
                     print(1)\n::end\n";
        assert_eq!(round_trip(input), input);
        assert_eq!(
            parse(input).blocks[0].reads,
            "site/index.html,site/site.css"
        );
        // Additive: a block that declares nothing serializes exactly as it always has.
        let bare = "::code id=check lang=python run\nprint(1)\n::end\n";
        assert_eq!(round_trip(bare), bare);
    }

    #[test]
    fn a_write_grant_round_trips_and_unset_writes_nothing() {
        let input = "::code id=test lang=bash run writes=target,generated\ncargo test\n::end\n";
        assert_eq!(round_trip(input), input);
        assert_eq!(parse(input).blocks[0].writes, "target,generated");
        // Additive: a block granting nothing serializes exactly as it always has.
        let bare = "::code id=test lang=bash run\ncargo test\n::end\n";
        assert_eq!(round_trip(bare), bare);
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
    fn a_board_round_trips_with_its_node_lines_verbatim() {
        // The body is reference lines, one per node; the scanner keeps them verbatim so a
        // key this version has never heard of survives the round-trip.
        let input = "::board id=plan height=520\n- ideas x=40 y=40 w=280\n- sketch x=380 y=40 w=320 to=ideas future=yes\n::end\n";
        assert_eq!(round_trip(input), input);
        let block = &parse(input).blocks[0];
        assert_eq!(block.height, 520);
        assert!(block.text.contains("future=yes"));
    }

    #[test]
    fn a_board_without_a_height_writes_no_height_attribute() {
        // Additive rule: an unset attribute serializes to nothing, or every existing
        // document reformats on its next save.
        let input = "::board id=plan\n- a x=0 y=0\n::end\n";
        assert_eq!(round_trip(input), input);
        assert_eq!(parse(input).blocks[0].height, 0);
    }

    #[test]
    fn an_empty_board_round_trips_empty() {
        // An empty canvas is a canvas waiting for its first node, not a defect to fill in.
        let input = "::board id=plan\n\n::end\n";
        assert_eq!(round_trip(input), input);
    }

    #[test]
    fn parse_then_stringify_is_idempotent() {
        let once = round_trip("::heading level=1 id=h\nHi\n::end\n");
        assert_eq!(round_trip(&once), once);
    }

    #[test]
    fn a_markdown_task_bullet_is_a_checklist_item_not_item_text() {
        // `- [ ] item` is how Markdown spells a task; treating the bullet as text turned
        // every pasted task list into `[ ] - [ ] item` on the next save.
        let document = parse("::checklist id=s\n- [ ] one\n- [x] two\n::end\n");
        let items = &document.blocks[0].items;
        assert_eq!(items.len(), 2);
        assert_eq!((items[0].text.as_str(), items[0].checked), ("one", false));
        assert_eq!((items[1].text.as_str(), items[1].checked), ("two", true));
        assert_eq!(
            stringify(&document),
            "::checklist id=s\n[ ] one\n[x] two\n::end\n"
        );
    }

    #[test]
    fn prose_between_typed_blocks_is_adopted_not_destroyed() {
        // A mixed document — Markdown prose around `::` blocks — is what a person or an
        // agent naturally writes. The scanner once skipped every line that was not block
        // syntax, so the next save silently erased the author's own words.
        let mixed =
            "# Plan\n\nThe goal.\n\n::code id=c lang=bash run\necho hi\n::end\n\nAfterword.\n";
        let document = parse(mixed);
        let kinds: Vec<&str> = document
            .blocks
            .iter()
            .map(|block| block.kind.as_str())
            .collect();
        assert_eq!(kinds, ["heading", "paragraph", "code", "paragraph"]);
        assert_eq!(document.blocks[1].text, "The goal.");
        assert_eq!(document.blocks[3].text, "Afterword.");
        // And the adopted form is stable: canonical output round-trips unchanged.
        let once = round_trip(mixed);
        assert_eq!(round_trip(&once), once);
    }

    /// Byte-exact canonical output for each real example and document, captured from this
    /// crate with `dx fmt`.
    ///
    /// These are captured output, not scripture. Regenerating one is correct when a defect is
    /// being fixed — four of them once encoded genuine data loss — but a fixture edited to
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
