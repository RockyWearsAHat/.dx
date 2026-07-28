//! `DOCB1` — the compact binary representation of a [`Document`].
//!
//! A varint-prefixed, tag-per-block encoding produced by [`pack`] and consumed by
//! [`unpack`]. The byte layout matches the TypeScript reference (`src/doc-binary.ts`)
//! exactly, so packed documents are portable between the native and wasm builds and
//! between this engine and the legacy TS reference. Format version 2 omits per-block
//! ids (ids are reconstructed by the parser layer, not stored here).

use crate::model::{Block, Document, Item};
use core::fmt;

const MAGIC: &[u8] = b"DOCB1";
const VERSION: u64 = 2;

/// Map a block kind to its DOCB1 type code (unknown kinds encode as `paragraph`).
fn type_code(kind: &str) -> u8 {
    match kind {
        "heading" => 1,
        "paragraph" => 2,
        "bulleted-list" => 3,
        "numbered-list" => 4,
        "quote" => 5,
        "code" => 6,
        "image" => 7,
        "rule" => 8,
        "checklist" => 9,
        "style" => 10,
        "stylesheet" => 11,
        "svg" => 12,
        "html" => 13,
        "graph" => 14,
        "mermaid" => 15,
        _ => 2,
    }
}

/// Map a DOCB1 type code back to a block kind (unknown codes decode as `paragraph`).
fn code_kind(code: u8) -> &'static str {
    match code {
        1 => "heading",
        3 => "bulleted-list",
        4 => "numbered-list",
        5 => "quote",
        6 => "code",
        7 => "image",
        8 => "rule",
        9 => "checklist",
        10 => "style",
        11 => "stylesheet",
        12 => "svg",
        13 => "html",
        14 => "graph",
        15 => "mermaid",
        _ => "paragraph",
    }
}

fn encode_varint(out: &mut Vec<u8>, value: u64) {
    let mut n = value;
    while n >= 0x80 {
        out.push((n as u8 & 0x7f) | 0x80);
        n >>= 7;
    }
    out.push(n as u8);
}

fn encode_string(out: &mut Vec<u8>, text: &str) {
    let bytes = text.as_bytes();
    encode_varint(out, bytes.len() as u64);
    out.extend_from_slice(bytes);
}

fn encode_block(out: &mut Vec<u8>, block: &Block) {
    out.push(type_code(&block.kind));
    // Version 2 stores no per-block id.

    match block.kind.as_str() {
        "heading" => {
            out.push(block.level.clamp(1, 4));
            encode_string(out, &block.text);
        }
        "bulleted-list" | "numbered-list" => {
            encode_varint(out, block.items.len() as u64);
            for item in &block.items {
                encode_string(out, &item.text);
            }
        }
        "code" => {
            encode_string(out, &block.language);
            encode_string(out, &block.text);
        }
        "image" => {
            encode_string(out, &block.src);
            encode_string(out, &block.alt);
        }
        "stylesheet" => {
            let href = if block.href.is_empty() {
                &block.src
            } else {
                &block.href
            };
            encode_string(out, href);
            encode_string(out, &block.media);
        }
        "rule" => {}
        "checklist" => {
            encode_varint(out, block.items.len() as u64);
            for item in &block.items {
                out.push(u8::from(item.checked));
                encode_string(out, &item.text);
            }
        }
        // style/svg/html/graph/mermaid and the paragraph/quote default all store text.
        _ => encode_string(out, &block.text),
    }
}

/// Pack a [`Document`] into a DOCB1 byte payload.
///
/// Complexity: `O(n)` time and space in the document's total byte size, since every field
/// of every block is encoded exactly once into the output buffer.
pub fn pack(document: &Document) -> Vec<u8> {
    let heading = document.first_heading_text();
    let stored_title =
        if !document.title.is_empty() && !heading.is_empty() && document.title == heading {
            ""
        } else {
            &document.title
        };

    let mut out = Vec::with_capacity(MAGIC.len() + 16);
    out.extend_from_slice(MAGIC);
    encode_varint(&mut out, VERSION);
    encode_string(&mut out, stored_title);
    encode_string(&mut out, &document.summary);

    encode_varint(&mut out, document.tags.len() as u64);
    for tag in &document.tags {
        encode_string(&mut out, tag);
    }

    encode_varint(&mut out, document.meta.len() as u64);
    for (key, value) in &document.meta {
        encode_string(&mut out, key);
        encode_string(&mut out, value);
    }

    encode_varint(&mut out, document.blocks.len() as u64);
    for block in &document.blocks {
        encode_block(&mut out, block);
    }
    out
}

/// An error encountered while decoding a DOCB1 payload.
#[derive(Debug, PartialEq, Eq)]
pub enum UnpackError {
    /// The payload is too short or lacks the `DOCB1` magic.
    InvalidMagic,
    /// The payload ended mid-field.
    Truncated,
}

impl fmt::Display for UnpackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            UnpackError::InvalidMagic => "DOCB1: invalid magic",
            UnpackError::Truncated => "DOCB1: truncated payload",
        })
    }
}

impl std::error::Error for UnpackError {}

/// A forward-only reader over a DOCB1 payload.
struct Cursor<'a> {
    buf: &'a [u8],
    off: usize,
}

impl Cursor<'_> {
    fn byte(&mut self) -> Result<u8, UnpackError> {
        let value = *self.buf.get(self.off).ok_or(UnpackError::Truncated)?;
        self.off += 1;
        Ok(value)
    }

    fn varint(&mut self) -> Result<u64, UnpackError> {
        let mut shift = 0u32;
        let mut value = 0u64;
        loop {
            let byte = self.byte()?;
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
            shift += 7;
        }
    }

    fn string(&mut self) -> Result<String, UnpackError> {
        let len = self.varint()? as usize;
        let end = self.off.checked_add(len).ok_or(UnpackError::Truncated)?;
        let slice = self.buf.get(self.off..end).ok_or(UnpackError::Truncated)?;
        self.off = end;
        // Lossy decode mirrors Node's tolerant `buffer.toString('utf8')`.
        Ok(String::from_utf8_lossy(slice).into_owned())
    }
}

fn decode_block(cursor: &mut Cursor) -> Result<Block, UnpackError> {
    let code = cursor.byte()?;
    let kind = code_kind(code).to_string();
    let mut block = Block {
        kind: kind.clone(),
        ..Block::default()
    };

    match kind.as_str() {
        "heading" => {
            block.level = cursor.byte()?;
            block.text = cursor.string()?;
        }
        "bulleted-list" | "numbered-list" => {
            let count = cursor.varint()?;
            for _ in 0..count {
                block.items.push(Item {
                    checked: false,
                    text: cursor.string()?,
                    ..Item::default()
                });
            }
        }
        "code" => {
            block.language = cursor.string()?;
            block.text = cursor.string()?;
        }
        "image" => {
            block.src = cursor.string()?;
            block.alt = cursor.string()?;
        }
        "stylesheet" => {
            block.href = cursor.string()?;
            block.media = cursor.string()?;
        }
        "rule" => {}
        "checklist" => {
            let count = cursor.varint()?;
            for _ in 0..count {
                let checked = cursor.byte()? == 1;
                block.items.push(Item {
                    checked,
                    text: cursor.string()?,
                    ..Item::default()
                });
            }
        }
        _ => block.text = cursor.string()?,
    }

    Ok(block)
}

/// Unpack a DOCB1 byte payload into a [`Document`].
///
/// Complexity: `O(n)` time and space in the blob length, since the cursor advances over
/// each byte once while reconstructing the document.
pub fn unpack(blob: &[u8]) -> Result<Document, UnpackError> {
    if blob.len() < MAGIC.len() + 1 || &blob[..MAGIC.len()] != MAGIC {
        return Err(UnpackError::InvalidMagic);
    }

    let mut cursor = Cursor {
        buf: blob,
        off: MAGIC.len(),
    };
    let _version = cursor.varint()?;

    let decoded_title = cursor.string()?;
    let summary = cursor.string()?;

    let tag_count = cursor.varint()?;
    let mut tags = Vec::with_capacity(tag_count as usize);
    for _ in 0..tag_count {
        tags.push(cursor.string()?);
    }

    let meta_count = cursor.varint()?;
    let mut meta = Vec::with_capacity(meta_count as usize);
    for _ in 0..meta_count {
        let key = cursor.string()?;
        let value = cursor.string()?;
        meta.push((key, value));
    }

    let block_count = cursor.varint()?;
    let mut blocks = Vec::with_capacity(block_count as usize);
    for _ in 0..block_count {
        blocks.push(decode_block(&mut cursor)?);
    }

    let mut document = Document {
        title: decoded_title,
        summary,
        tags,
        meta,
        blocks,
    };
    if document.title.is_empty() {
        document.title = document.first_heading_text().to_string();
    }
    Ok(document)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Document {
        Document {
            title: "Architecture Notes".to_string(),
            summary: "What this covers.".to_string(),
            tags: vec!["architecture".to_string(), "docs".to_string()],
            meta: vec![("owner".to_string(), "\"alex\"".to_string())],
            blocks: vec![
                Block {
                    kind: "heading".to_string(),
                    level: 1,
                    text: "Architecture Notes".to_string(),
                    ..Block::default()
                },
                Block {
                    kind: "paragraph".to_string(),
                    text: "Body text.".to_string(),
                    ..Block::default()
                },
                Block {
                    kind: "bulleted-list".to_string(),
                    items: vec![
                        Item {
                            checked: false,
                            text: "one".to_string(),
                            ..Item::default()
                        },
                        Item {
                            checked: false,
                            text: "two".to_string(),
                            ..Item::default()
                        },
                    ],
                    ..Block::default()
                },
            ],
        }
    }

    #[test]
    fn round_trips_documents() {
        let doc = sample();
        let packed = pack(&doc);
        assert_eq!(&packed[..5], MAGIC);
        let restored = unpack(&packed).unwrap();
        assert_eq!(restored, doc);
    }

    #[test]
    fn title_equal_to_heading_is_reconstructed() {
        // When the title equals the first heading, it is not stored, then rebuilt.
        let doc = sample();
        let packed = pack(&doc);
        let restored = unpack(&packed).unwrap();
        assert_eq!(restored.title, "Architecture Notes");
    }

    #[test]
    fn matches_typescript_reference_vector() {
        // Captured from the TypeScript reference (`src/doc-binary.ts`, packDocument).
        let hex: String = pack(&sample()).iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            hex,
            "444f43423102001157686174207468697320636f766572732e020c61726368697465637475726504646f637301056f776e65720622616c65782203010112417263686974656374757265204e6f746573020a426f647920746578742e0302036f6e650374776f"
        );
    }

    #[test]
    fn rejects_bad_magic() {
        assert_eq!(unpack(b"XXXXX\x02").unwrap_err(), UnpackError::InvalidMagic);
        assert_eq!(unpack(b"").unwrap_err(), UnpackError::InvalidMagic);
    }
}
