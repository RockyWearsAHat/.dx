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
