//! End-to-end integration test for the `doc-core` storage pipeline.
//!
//! Exercises the exact path the native store and the wasm host use: build a document,
//! split it into content-addressed chunks, encode the pack, then reverse every step and
//! assert the canonical source survives byte-for-byte.

use doc_core::chunk::{decode_pack, encode_pack, join, split, Pack, PackError};
use doc_core::compress::{compress, decompress};
use doc_core::digest::sha256_hex;
use doc_core::format::{parse, stringify};
use doc_core::model::{Block, Document, Item};

fn document() -> Document {
    Document {
        title: "Pipeline".to_string(),
        summary: "Round-trip across the whole core.".to_string(),
        tags: vec!["rust".to_string(), "wasm".to_string()],
        meta: vec![("owner".to_string(), "\"rocky\"".to_string())],
        blocks: vec![
            Block {
                kind: "heading".to_string(),
                level: 1,
                text: "Pipeline".to_string(),
                ..Block::default()
            },
            Block {
                kind: "paragraph".to_string(),
                text: "Body that repeats. Body that repeats.".to_string(),
                ..Block::default()
            },
            Block {
                kind: "checklist".to_string(),
                items: vec![
                    Item {
                        checked: true,
                        text: "ported digest".to_string(),
                        ..Item::default()
                    },
                    Item {
                        checked: true,
                        text: "ported codec".to_string(),
                        ..Item::default()
                    },
                    Item {
                        checked: false,
                        text: "ported parser".to_string(),
                        ..Item::default()
                    },
                ],
                ..Block::default()
            },
            // Execution attributes an earlier binary codec silently dropped.
            Block {
                kind: "code".to_string(),
                id: "stats".to_string(),
                language: "python".to_string(),
                text: "print(1)".to_string(),
                run: true,
                deps: "numpy".to_string(),
                timeout: 45,
                format: "svg".to_string(),
                ..Block::default()
            },
        ],
    }
}

#[test]
fn document_survives_split_pack_roundtrip() {
    let doc = document();
    let canonical = stringify(&doc);

    let pack = Pack::build(vec![("notes.dx", &doc)]);
    let bytes = encode_pack(&pack);
    let decoded = decode_pack(&bytes).expect("pack decodes");

    assert_eq!(
        decoded.source("notes.dx").expect("entry present"),
        canonical
    );
    // And the recovered source re-parses to the same canonical form.
    assert_eq!(stringify(&parse(&canonical)), canonical);
}

#[test]
fn every_authored_attribute_survives_storage() {
    // A structured binary codec dropped run/deps/timeout/format and every block id.
    // Storage keeps canonical text, so the attributes come back or the test fails loudly.
    let restored = parse(
        &Pack::build(vec![("notes.dx", &document())])
            .source("notes.dx")
            .expect("entry"),
    );
    let code = restored
        .blocks
        .iter()
        .find(|block| block.kind == "code")
        .expect("code block survived");
    assert_eq!(code.id, "stats");
    assert!(code.run);
    assert_eq!(code.deps, "numpy");
    assert_eq!(code.timeout, 45);
    assert_eq!(code.format, "svg");

    let checklist = restored
        .blocks
        .iter()
        .find(|block| block.kind == "checklist")
        .expect("checklist survived");
    assert_eq!(checklist.items.len(), 3);
    assert_eq!(
        checklist.items.iter().filter(|item| item.checked).count(),
        2
    );
}

#[test]
fn chunk_bodies_are_addressed_by_content_and_compress_losslessly() {
    let doc = document();
    let chunks = split(&doc);
    for chunk in &chunks {
        assert_eq!(chunk.hash, sha256_hex(chunk.text.as_bytes()));
        let frame = compress(chunk.text.as_bytes());
        assert_eq!(
            decompress(&frame).expect("frame decompresses"),
            chunk.text.as_bytes()
        );
    }
    let texts: Vec<&str> = chunks.iter().map(|chunk| chunk.text.as_str()).collect();
    assert_eq!(join(texts), stringify(&doc));
}

#[test]
fn corrupt_pack_is_rejected_not_panicked() {
    let bytes = encode_pack(&Pack::build(vec![("notes.dx", &document())]));

    // Flip a payload byte: either it errors, or it decodes to something harmless.
    let mut broken = bytes.clone();
    if let Some(last) = broken.last_mut() {
        *last ^= 0xff;
    }
    let _ = decode_pack(&broken);

    // Truncation is caught by the declared length.
    let mut short = bytes;
    short.pop();
    assert_eq!(decode_pack(&short).unwrap_err(), PackError::LengthMismatch);

    // Random bytes are not a pack.
    assert_eq!(
        decode_pack(b"not a pack at all").unwrap_err(),
        PackError::InvalidMagic
    );
}

/// A tiny deterministic pseudo-random generator, so the storage sweep below is adversarial
/// without being flaky: the same run happens on every machine and in every CI job.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*, chosen because it is four lines and needs no dependency.
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn pick<'a, T>(&mut self, options: &'a [T]) -> &'a T {
        &options[(self.next() % options.len() as u64) as usize]
    }
}

/// Text fragments that have each broken this code at some point: non-ASCII prose, the
/// format's own delimiters, blank lines, trailing spaces, and an empty string.
const NASTY: &[&str] = &[
    "plain text",
    "",
    "  leading and trailing  ",
    "líne with ünïcode — ページ 中文 🎉",
    "::end",
    "::heading level=1 id=fake",
    "line one\n\nline three",
    "tabs\tand\tmore",
    "a very long line that goes on well past any comfortable width so wrapping and match \
     finding both have something to chew on, twice over, twice over, twice over",
    "```fenced```",
    "- looks like a list item",
    "[x] looks like a checklist item",
];

/// Every byte of every document must survive the whole storage path.
///
/// This is the one property the format cannot compromise on, so it is checked against
/// generated documents rather than only hand-written ones: 200 documents built from every
/// block kind, with bodies drawn from inputs that have caused real data loss before. For
/// each one, three things must hold — the canonical text is what the chunks concatenate to,
/// the pack gives it back exactly, and every chunk is addressed by the hash of its own
/// bytes.
#[test]
fn no_generated_document_loses_a_byte_through_the_store() {
    let kinds = [
        "heading",
        "paragraph",
        "quote",
        "code",
        "output",
        "bulleted-list",
        "numbered-list",
        "checklist",
        "nav",
        "rule",
        "image",
        "svg",
        "html",
        "mermaid",
        "style",
        "script",
    ];
    let mut rng = Rng(0x0DDB_A11C_0FFE_E123);

    for round in 0..200u32 {
        let block_count = 1 + (rng.next() % 12) as usize;
        let mut blocks = Vec::with_capacity(block_count);
        for _ in 0..block_count {
            let kind = (*rng.pick(&kinds)).to_string();
            let body = (*rng.pick(NASTY)).to_string();
            let items = (0..(rng.next() % 4))
                .map(|_| Item {
                    checked: rng.next().is_multiple_of(2),
                    text: (*rng.pick(NASTY)).to_string(),
                    nested: (0..(rng.next() % 3))
                        .map(|_| Item {
                            text: (*rng.pick(NASTY)).to_string(),
                            ..Item::default()
                        })
                        .collect(),
                })
                .collect();
            blocks.push(Block {
                kind,
                level: (rng.next() % 6) as u8,
                text: body.clone(),
                alt: body.clone(),
                src: "picture.png".to_string(),
                language: (*rng.pick(&["python", "rust", "", "bash"])).to_string(),
                run: rng.next().is_multiple_of(2),
                deps: (*rng.pick(&["", "numpy", "serde requests"])).to_string(),
                timeout: (rng.next() % 90) as u32,
                for_block: (*rng.pick(&["", "demo"])).to_string(),
                status: (*rng.pick(&["", "ok", "error"])).to_string(),
                exit: (rng.next() % 3) as i32,
                hash: (*rng.pick(&["", "abc123"])).to_string(),
                format: (*rng.pick(&["", "svg", "html"])).to_string(),
                class_name: (*rng.pick(&["", "sidebar wide"])).to_string(),
                label: (*rng.pick(&["", "{n}. {name}"])).to_string(),
                hidden: rng.next().is_multiple_of(4),
                items,
                ..Block::default()
            });
        }

        let document = Document {
            blocks,
            ..Document::default()
        };
        // Canonical form is the thing storage promises to preserve, so compare against it
        // rather than against the generated model, which is not yet normalized.
        let canonical = stringify(&document);
        let document = parse(&canonical);
        let canonical = stringify(&document);

        let chunks = split(&document);
        let texts: Vec<&str> = chunks.iter().map(|chunk| chunk.text.as_str()).collect();
        assert_eq!(join(texts), canonical, "round {round}: chunks lost bytes");
        for chunk in &chunks {
            assert_eq!(
                chunk.hash,
                sha256_hex(chunk.text.as_bytes()),
                "round {round}: chunk is not addressed by its own content"
            );
        }

        let path = format!("generated-{round}.dx");
        let pack = encode_pack(&Pack::build(vec![(path.as_str(), &document)]));
        let decoded = decode_pack(&pack).expect("pack decodes");
        assert_eq!(
            decoded.source(&path).expect("entry"),
            canonical,
            "round {round}: pack lost bytes"
        );
    }
}
