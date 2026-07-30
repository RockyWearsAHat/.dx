//! Content-addressed document chunks and the `DXCP1` pack container.
//!
//! A document is stored as an ordered list of **chunks**, one per block, where a chunk is
//! the canonical DOCSRC text of that block addressed by the SHA-256 of those bytes. Two
//! blocks that read the same — across documents, or across two versions of one document —
//! are the same chunk and are stored once.
//!
//! # Why per-block canonical text
//! The chunk payload is the exact text [`crate::format::stringify`] would write for that
//! block, so reassembly is not a re-serialization: [`join`] concatenates the pieces and
//! gets the canonical file back byte-for-byte. Storage therefore cannot lose a field it
//! did not know about, which a structured binary encoding can and did.
//!
//! # Layout
//! [`split`] turns a document into chunks; [`join`] turns chunk texts back into canonical
//! source; [`encode_pack`] / [`decode_pack`] move a whole set of documents through one
//! compressed, deduplicated byte container.
//!
//! This module is host-free (no filesystem, no clock), so it compiles to `wasm32`
//! alongside the rest of `doc-core`.

use crate::compress::{compress, decompress, DecompressError};
use crate::digest::sha256_hex;
use crate::format::{stringify_blocks, BLOCK_SEPARATOR};
use crate::model::Document;
use core::fmt;

/// The pack container magic prefix.
const MAGIC: &[u8] = b"DXCP1";
/// Magic plus the compressed-payload length field.
const HEADER_LENGTH: usize = MAGIC.len() + 4;

/// One addressable piece of a document: the canonical text of a single block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// Lowercase hex SHA-256 of [`Chunk::text`]'s bytes — this chunk's address.
    pub hash: String,
    /// Canonical DOCSRC for exactly one block, with no trailing newline.
    pub text: String,
}

impl Chunk {
    /// Address `text` and wrap it as a chunk.
    #[must_use]
    pub fn new(text: String) -> Self {
        let hash = sha256_hex(text.as_bytes());
        Self { hash, text }
    }

    /// The chunk's payload size in bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.text.len()
    }

    /// Whether the chunk carries no bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}

/// Split `document` into its per-block chunks, in document order.
///
/// Blocks are normalized once for the whole document (so ids stay unique), then each is
/// addressed on its own. Repeated blocks yield repeated hashes; de-duplication is the
/// caller's to do when storing, since order must be preserved here.
///
/// Complexity: `O(n)` in the document's byte size.
#[must_use]
pub fn split(document: &Document) -> Vec<Chunk> {
    stringify_blocks(document)
        .into_iter()
        .map(Chunk::new)
        .collect()
}

/// Rebuild canonical DOCSRC from ordered per-block texts.
///
/// This is the exact inverse of [`split`]: `join(&split(doc) texts)` equals
/// [`crate::format::stringify`] for the same document, byte-for-byte.
#[must_use]
pub fn join<'a, I>(texts: I) -> String
where
    I: IntoIterator<Item = &'a str>,
{
    let body = texts
        .into_iter()
        .collect::<Vec<&str>>()
        .join(BLOCK_SEPARATOR);
    format!("{body}\n")
}

/// A document's entry in a pack: where it lives and which chunks it is made of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackEntry {
    /// Workspace-relative `.dx` path identifying the document.
    pub path: String,
    /// Chunk hashes in document order; duplicates are allowed and meaningful.
    pub chunks: Vec<String>,
}

/// A set of documents plus the chunk bodies they reference, ready to encode.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pack {
    /// Every distinct chunk referenced by [`Pack::entries`], in any order.
    pub chunks: Vec<Chunk>,
    /// The documents in the pack.
    pub entries: Vec<PackEntry>,
}

impl Pack {
    /// Build a pack from `(path, document)` pairs, de-duplicating chunk bodies.
    ///
    /// Complexity: `O(n)` in total document bytes.
    #[must_use]
    pub fn build<'a, I>(documents: I) -> Self
    where
        I: IntoIterator<Item = (&'a str, &'a Document)>,
    {
        let mut pack = Self::default();
        let mut seen: Vec<String> = Vec::new();

        for (path, document) in documents {
            let mut hashes = Vec::new();
            for chunk in split(document) {
                if !seen.contains(&chunk.hash) {
                    seen.push(chunk.hash.clone());
                    pack.chunks.push(chunk.clone());
                }
                hashes.push(chunk.hash);
            }
            pack.entries.push(PackEntry {
                path: path.to_string(),
                chunks: hashes,
            });
        }
        pack
    }

    /// The canonical source of the entry at `path`, or `None` when it is absent or
    /// references a chunk the pack does not carry.
    #[must_use]
    pub fn source(&self, path: &str) -> Option<String> {
        let entry = self.entries.iter().find(|entry| entry.path == path)?;
        let mut texts = Vec::with_capacity(entry.chunks.len());
        for hash in &entry.chunks {
            let chunk = self.chunks.iter().find(|chunk| &chunk.hash == hash)?;
            texts.push(chunk.text.as_str());
        }
        Some(join(texts))
    }
}

/// An error encountered while decoding a `DXCP1` pack.
#[derive(Debug, PartialEq, Eq)]
pub enum PackError {
    /// The payload is too short or lacks the `DXCP1` magic.
    InvalidMagic,
    /// The declared compressed length does not match the bytes present.
    LengthMismatch,
    /// The compressed payload could not be decompressed.
    Corrupt(DecompressError),
    /// The payload ended mid-field.
    Truncated,
    /// An entry referenced a chunk index outside the chunk table.
    UnknownChunk,
}

impl fmt::Display for PackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMagic => f.write_str("DXCP1: not a document pack"),
            Self::LengthMismatch => f.write_str("DXCP1: declared length does not match the file"),
            Self::Corrupt(error) => write!(f, "DXCP1: compressed payload is damaged ({error})"),
            Self::Truncated => f.write_str("DXCP1: pack ends mid-record"),
            Self::UnknownChunk => f.write_str("DXCP1: an entry references a missing chunk"),
        }
    }
}

impl std::error::Error for PackError {}

/// Append `value` as a LEB128 varint.
fn put_varint(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let byte = u8::try_from(value & 0x7f).unwrap_or(0);
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// Append a length-prefixed UTF-8 string.
fn put_string(out: &mut Vec<u8>, text: &str) {
    put_varint(out, text.len() as u64);
    out.extend_from_slice(text.as_bytes());
}

/// A forward-only reader over a decompressed pack payload.
struct Reader<'a> {
    buf: &'a [u8],
    off: usize,
}

impl Reader<'_> {
    fn varint(&mut self) -> Result<u64, PackError> {
        let mut shift = 0u32;
        let mut value = 0u64;
        loop {
            let byte = *self.buf.get(self.off).ok_or(PackError::Truncated)?;
            self.off += 1;
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
            shift += 7;
            if shift > 63 {
                return Err(PackError::Truncated);
            }
        }
    }

    fn string(&mut self) -> Result<String, PackError> {
        let len = usize::try_from(self.varint()?).map_err(|_| PackError::Truncated)?;
        let end = self.off.checked_add(len).ok_or(PackError::Truncated)?;
        let slice = self.buf.get(self.off..end).ok_or(PackError::Truncated)?;
        self.off = end;
        Ok(String::from_utf8_lossy(slice).into_owned())
    }
}

/// Encode `pack` into a compressed `DXCP1` byte container.
///
/// Entries reference chunks by index into the chunk table rather than by hash, so a
/// document costs a few bytes per block beyond the block bodies themselves. The whole
/// payload is `dxz1`-compressed as one stream, so redundancy *between* chunks compresses
/// too.
///
/// Layout:
/// ```text
///   [0..5]   magic b"DXCP1"
///   [5..9]   u32 LE length of the compressed payload
///   [9..]    dxz1(payload)
///
///   payload:
///     varint chunk count, then per chunk: varint byte length, chunk text
///     varint entry count, then per entry: varint path length, path,
///                                        varint chunk count, varint chunk indices
/// ```
#[must_use]
pub fn encode_pack(pack: &Pack) -> Vec<u8> {
    let mut payload = Vec::new();

    put_varint(&mut payload, pack.chunks.len() as u64);
    for chunk in &pack.chunks {
        put_string(&mut payload, &chunk.text);
    }

    put_varint(&mut payload, pack.entries.len() as u64);
    for entry in &pack.entries {
        put_string(&mut payload, &entry.path);
        put_varint(&mut payload, entry.chunks.len() as u64);
        for hash in &entry.chunks {
            let index = pack
                .chunks
                .iter()
                .position(|chunk| &chunk.hash == hash)
                .unwrap_or(usize::MAX);
            put_varint(&mut payload, index as u64);
        }
    }

    let compressed = compress(&payload);
    let mut out = Vec::with_capacity(HEADER_LENGTH + compressed.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
    out.extend_from_slice(&compressed);
    out
}

/// Decode a `DXCP1` container produced by [`encode_pack`].
///
/// Chunk hashes are recomputed from the stored bytes rather than trusted from the file, so
/// a pack cannot claim an address its content does not have.
pub fn decode_pack(bytes: &[u8]) -> Result<Pack, PackError> {
    if bytes.len() < HEADER_LENGTH || &bytes[..MAGIC.len()] != MAGIC {
        return Err(PackError::InvalidMagic);
    }
    let declared = u32::from_le_bytes([bytes[5], bytes[6], bytes[7], bytes[8]]) as usize;
    let body = &bytes[HEADER_LENGTH..];
    if body.len() != declared {
        return Err(PackError::LengthMismatch);
    }

    let payload = decompress(body).map_err(PackError::Corrupt)?;
    let mut reader = Reader {
        buf: &payload,
        off: 0,
    };

    let chunk_count = usize::try_from(reader.varint()?).map_err(|_| PackError::Truncated)?;
    let mut chunks = Vec::with_capacity(chunk_count.min(4096));
    for _ in 0..chunk_count {
        chunks.push(Chunk::new(reader.string()?));
    }

    let entry_count = usize::try_from(reader.varint()?).map_err(|_| PackError::Truncated)?;
    let mut entries = Vec::with_capacity(entry_count.min(4096));
    for _ in 0..entry_count {
        let path = reader.string()?;
        let reference_count =
            usize::try_from(reader.varint()?).map_err(|_| PackError::Truncated)?;
        let mut hashes = Vec::with_capacity(reference_count.min(4096));
        for _ in 0..reference_count {
            let index = usize::try_from(reader.varint()?).map_err(|_| PackError::Truncated)?;
            let chunk = chunks.get(index).ok_or(PackError::UnknownChunk)?;
            hashes.push(chunk.hash.clone());
        }
        entries.push(PackEntry {
            path,
            chunks: hashes,
        });
    }

    Ok(Pack { chunks, entries })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::{parse, stringify};

    /// Every real document in the repository, as the storage layer will see them.
    const REAL_DOCUMENTS: &[(&str, &str)] = &[
        (
            "examples/welcome.dx",
            include_str!("../tests/fixtures/welcome.input.dx"),
        ),
        (
            "examples/tutorial.dx",
            include_str!("../tests/fixtures/tutorial.input.dx"),
        ),
        (
            "examples/showcase.dx",
            include_str!("../tests/fixtures/showcase.input.dx"),
        ),
        (
            "examples/block-reference.dx",
            include_str!("../tests/fixtures/block-reference.input.dx"),
        ),
    ];

    #[test]
    fn joining_split_chunks_reproduces_canonical_source_exactly() {
        for (path, raw) in REAL_DOCUMENTS {
            let document = parse(raw);
            let chunks = split(&document);
            let texts: Vec<&str> = chunks.iter().map(|chunk| chunk.text.as_str()).collect();
            assert_eq!(
                join(texts),
                stringify(&document),
                "chunk round-trip lost bytes for {path}"
            );
        }
    }

    #[test]
    fn a_chunk_is_addressed_by_the_hash_of_its_own_bytes() {
        let chunk = Chunk::new("::paragraph id=p\nhi\n::end".to_string());
        assert_eq!(chunk.hash, sha256_hex(chunk.text.as_bytes()));
        assert_eq!(chunk.hash.len(), 64);
    }

    #[test]
    fn identical_blocks_share_one_chunk_body() {
        let repeated = parse("::paragraph id=a\nsame\n::end\n\n::paragraph id=b\nsame\n::end\n");
        // Different ids mean different canonical text, so these are two chunks.
        assert_eq!(split(&repeated).len(), 2);

        // Two documents whose blocks read identically store one body between them.
        let one = parse("::paragraph id=p\nshared\n::end\n");
        let two = parse("::paragraph id=p\nshared\n::end\n");
        let pack = Pack::build(vec![("a.dx", &one), ("b.dx", &two)]);
        assert_eq!(pack.chunks.len(), 1);
        assert_eq!(pack.entries.len(), 2);
    }

    #[test]
    fn a_pack_round_trips_every_real_document_byte_for_byte() {
        let parsed: Vec<(String, Document)> = REAL_DOCUMENTS
            .iter()
            .map(|(path, raw)| ((*path).to_string(), parse(raw)))
            .collect();
        let pack = Pack::build(parsed.iter().map(|(path, doc)| (path.as_str(), doc)));

        let bytes = encode_pack(&pack);
        let decoded = decode_pack(&bytes).expect("pack decodes");
        assert_eq!(decoded, pack);

        for (path, document) in &parsed {
            assert_eq!(
                decoded.source(path).expect("entry present"),
                stringify(document),
                "pack lost bytes for {path}"
            );
        }
    }

    #[test]
    fn a_pack_is_smaller_than_the_sources_it_holds() {
        let parsed: Vec<(String, Document)> = REAL_DOCUMENTS
            .iter()
            .map(|(path, raw)| ((*path).to_string(), parse(raw)))
            .collect();
        let plain: usize = parsed.iter().map(|(_, doc)| stringify(doc).len()).sum();
        let packed = encode_pack(&Pack::build(
            parsed.iter().map(|(path, doc)| (path.as_str(), doc)),
        ))
        .len();
        assert!(
            packed < plain,
            "pack ({packed} bytes) should beat plain source ({plain} bytes)"
        );
    }

    #[test]
    fn a_missing_document_or_chunk_is_reported_not_guessed() {
        let pack = Pack::build(vec![("a.dx", &parse("::paragraph id=p\nx\n::end\n"))]);
        assert!(pack.source("nope.dx").is_none());

        let mut broken = pack.clone();
        broken.chunks.clear();
        assert!(broken.source("a.dx").is_none());
    }

    #[test]
    fn garbage_is_rejected_with_a_reason_not_a_panic() {
        assert_eq!(decode_pack(b"").unwrap_err(), PackError::InvalidMagic);
        assert_eq!(
            decode_pack(b"nope!!!!!!").unwrap_err(),
            PackError::InvalidMagic
        );

        let mut truncated = encode_pack(&Pack::build(vec![(
            "a.dx",
            &parse("::paragraph id=p\nx\n::end\n"),
        )]));
        truncated.pop();
        assert_eq!(
            decode_pack(&truncated).unwrap_err(),
            PackError::LengthMismatch
        );
    }

    #[test]
    fn an_empty_pack_round_trips() {
        let pack = Pack::default();
        assert_eq!(decode_pack(&encode_pack(&pack)).expect("decode"), pack);
    }

    #[test]
    fn non_ascii_prose_survives_the_pack() {
        let document = parse("::paragraph id=p\nこんにちは — naïve café ✅\n::end\n");
        let pack = Pack::build(vec![("i18n.dx", &document)]);
        let decoded = decode_pack(&encode_pack(&pack)).expect("decode");
        assert_eq!(
            decoded.source("i18n.dx").expect("entry"),
            stringify(&document)
        );
    }
}
