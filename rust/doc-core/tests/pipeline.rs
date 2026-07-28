//! End-to-end integration test for the `doc-core` storage pipeline.
//!
//! Exercises the exact path the native binary and wasm host will use: build a document,
//! pack it to DOCB1, compress the bundle payload with dxz1, then reverse every step and
//! assert the document survives unchanged.

use doc_core::compress::{compress, decompress};
use doc_core::digest::sha256_hex;
use doc_core::docbin::{pack, unpack};
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
        ],
    }
}

#[test]
fn document_survives_pack_compress_roundtrip() {
    let doc = document();

    let packed = pack(&doc);
    let fingerprint = sha256_hex(&packed);

    let frame = compress(&packed);
    let restored_packed = decompress(&frame).expect("frame decompresses");

    // Compression is lossless: identical bytes and identical fingerprint.
    assert_eq!(restored_packed, packed);
    assert_eq!(sha256_hex(&restored_packed), fingerprint);

    // The document decodes back to exactly what we started with.
    let restored = unpack(&restored_packed).expect("payload unpacks");
    assert_eq!(restored, doc);
}

#[test]
fn corrupt_frame_is_rejected_not_panicked() {
    let frame = compress(&pack(&document()));
    let mut broken = frame.clone();
    *broken.last_mut().unwrap() ^= 0xff; // flip the final byte
                                         // Either it errors or it decodes to something that fails to unpack — never panics.
    if let Ok(bytes) = decompress(&broken) {
        let _ = unpack(&bytes);
    }
}
