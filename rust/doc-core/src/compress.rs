//! `dxz` — the document platform's byte compressor, and the frames it writes.
//!
//! Storage compactness is a product requirement here: a document's content lives in a
//! committed pack, so every byte saved is a byte not carried in the repository forever.
//! Compression is therefore **chosen per payload, not fixed**: [`compress`] encodes the
//! input each supported way and keeps the smallest result. That has two consequences worth
//! relying on — the stored form is never larger than the raw bytes plus a header, and a
//! better codec can be added later without a migration.
//!
//! # Frames
//! Every frame opens with four magic bytes naming its codec, then the original byte length:
//!
//! | Magic  | Codec | Written by [`compress`] | Read by [`decompress`] |
//! |--------|-------|-------------------------|------------------------|
//! | `DXZ1` | Self-contained LZSS | no (superseded) | **yes** |
//! | `DXZ2` | DEFLATE (`miniz_oxide`, pure Rust) | yes, when smallest | yes |
//! | `DXZ3` | Stored, uncompressed | yes, when nothing beats it | yes |
//!
//! `DXZ1` is still decoded and always will be: packs written before `DXZ2` existed are
//! committed in real repositories, and a format that stops reading its own history loses
//! documents. This is why the codec is named in the frame rather than assumed.
//!
//! DEFLATE was chosen over the heavier options (brotli, LZMA) because it is pure Rust with
//! no `unsafe`, compiles to `wasm32` for the editor without bloating the bundle, and closes
//! most of the gap: on this repository's own examples it stores what LZSS wrote in 49% of
//! source size in 32% instead.
//!
//! Frame layout:
//! ```text
//!   [0..4]  magic — 'DXZ1' | 'DXZ2' | 'DXZ3'
//!   [4..8]  original byte length (u32, big-endian)
//!   [8..]   codec payload
//! ```
//! For `DXZ1` the payload is the LZSS token stream: a flag byte precedes each group of up
//! to 8 tokens; bit `(7 - i)`, MSB first, marks token `i` as a back-reference (`1`) or a
//! literal (`0`).

use core::fmt;

/// Magic of the original self-contained LZSS frame: decoded forever, no longer written.
const MAGIC_LZSS: &[u8; 4] = b"DXZ1";
/// Magic of a DEFLATE frame.
const MAGIC_DEFLATE: &[u8; 4] = b"DXZ2";
/// Magic of a stored (uncompressed) frame.
const MAGIC_STORED: &[u8; 4] = b"DXZ3";
/// Magic plus the original-length field.
const HEADER_LENGTH: usize = 8;

/// The largest output any `dxz` frame may decode to.
///
/// A frame's declared length is four bytes of untrusted input, so it bounds an allocation and
/// an inflate only if it is itself bounded. 256 MiB is far above any repository's compressed
/// documents and far below the size at which a claim becomes a way to stop the process.
const MAX_DECOMPRESSED: usize = 256 * 1024 * 1024;

/// Compression level used for DEFLATE frames: the highest, because a pack is written once
/// and read many times, and it is committed.
const DEFLATE_LEVEL: u8 = 10;

const MIN_MATCH: usize = 3;
const MAX_MATCH: usize = 258; // length byte 0..=255 maps to MIN_MATCH..=258
const MAX_DISTANCE: usize = 0xffff; // distance fits in two bytes
const HASH_SIZE: usize = 1 << 15;
const HASH_MASK: usize = HASH_SIZE - 1;
const MAX_CHAIN: usize = 128; // bound the match search

/// Hash the 3-byte sequence at `pos` into the chain-table index space.
#[inline]
fn hash_at(input: &[u8], pos: usize) -> usize {
    (((input[pos] as usize) << 10) ^ ((input[pos + 1] as usize) << 5) ^ (input[pos + 2] as usize))
        & HASH_MASK
}

/// Compress raw bytes into the smallest frame any supported codec produces.
///
/// Infallible, and never inflating: if no codec beats the input itself, the bytes are
/// stored as they are and the cost is the 8-byte header. Every frame this returns
/// round-trips through [`decompress`] exactly — that is asserted for text, binary,
/// repetitive, random, and empty input, because storage that loses a byte is the one defect
/// this format cannot survive.
///
/// Complexity: `O(n)` in the input size, plus DEFLATE's own bounded match search.
#[must_use]
pub fn compress(input: &[u8]) -> Vec<u8> {
    let deflated = miniz_oxide::deflate::compress_to_vec(input, DEFLATE_LEVEL);
    if HEADER_LENGTH + deflated.len() < HEADER_LENGTH + input.len() {
        return frame(MAGIC_DEFLATE, input.len(), &deflated);
    }
    frame(MAGIC_STORED, input.len(), input)
}

/// Wrap raw bytes in a frame that stores them exactly as they are.
///
/// The same `DXZ3` frame [`compress`] falls back to when no codec beats the input, offered
/// as a deliberate choice for the one caller that wants it: a file whose consumer is git,
/// which deltas plain bytes between revisions and cannot delta a compressed stream. Round-trips
/// through [`decompress`] like any other frame — reading never has to know which was chosen.
///
/// Complexity: `O(n)` in the input size.
#[must_use]
pub fn store_as_is(input: &[u8]) -> Vec<u8> {
    frame(MAGIC_STORED, input.len(), input)
}

/// Assemble a frame: magic, original length, payload.
fn frame(magic: &[u8; 4], original_length: usize, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LENGTH + payload.len());
    out.extend_from_slice(magic);
    out.extend_from_slice(&(original_length as u32).to_be_bytes());
    out.extend_from_slice(payload);
    out
}

/// Compress raw bytes into a `DXZ1` LZSS frame.
///
/// No longer written by [`compress`] — DEFLATE is smaller on every realistic document —
/// but kept because the decoder must stay honest about what it can produce, and the
/// round-trip tests exercise both directions of the frame that existing packs contain.
///
/// Complexity: `O(n · MAX_CHAIN)` time, where `n` is the input length and `MAX_CHAIN`
/// is the fixed cap on match-candidate probes per position — so the LZSS match search is
/// bounded to linear time in `n` — and `O(n)` extra space for the hash chain tables.
#[cfg_attr(not(test), allow(dead_code))]
fn lzss_compress(input: &[u8]) -> Vec<u8> {
    let n = input.len();
    let mut out: Vec<u8> = Vec::with_capacity(HEADER_LENGTH + n);
    out.extend_from_slice(MAGIC_LZSS);
    out.extend_from_slice(&(n as u32).to_be_bytes());

    let mut head = vec![-1i32; HASH_SIZE];
    let mut prev = vec![-1i32; n.max(1)];

    let mut flag_index = out.len();
    out.push(0);
    let mut flag_bits = 0u32;

    let mut pos = 0usize;
    while pos < n {
        let mut best_len = MIN_MATCH - 1;
        let mut best_dist = 0usize;

        if pos + MIN_MATCH <= n {
            let max_len = MAX_MATCH.min(n - pos);
            let mut candidate = head[hash_at(input, pos)];
            let mut chain = MAX_CHAIN;
            while candidate >= 0 && chain > 0 {
                let candidate_pos = candidate as usize;
                let distance = pos - candidate_pos;
                if distance > MAX_DISTANCE {
                    break;
                }
                let mut len = 0usize;
                while len < max_len && input[candidate_pos + len] == input[pos + len] {
                    len += 1;
                }
                if len > best_len {
                    best_len = len;
                    best_dist = distance;
                    if len == max_len {
                        break;
                    }
                }
                candidate = prev[candidate_pos];
                chain -= 1;
            }
        }

        if best_len >= MIN_MATCH {
            emit(
                &mut out,
                &mut flag_index,
                &mut flag_bits,
                true,
                &[
                    (best_len - MIN_MATCH) as u8,
                    ((best_dist >> 8) & 0xff) as u8,
                    (best_dist & 0xff) as u8,
                ],
            );
            let end = pos + best_len;
            while pos < end {
                insert(input, &mut head, &mut prev, pos, n);
                pos += 1;
            }
        } else {
            emit(
                &mut out,
                &mut flag_index,
                &mut flag_bits,
                false,
                &[input[pos]],
            );
            insert(input, &mut head, &mut prev, pos, n);
            pos += 1;
        }
    }

    out
}

/// Append one token to `out`, managing the rolling flag byte at `flag_index`.
fn emit(
    out: &mut Vec<u8>,
    flag_index: &mut usize,
    flag_bits: &mut u32,
    is_match: bool,
    bytes: &[u8],
) {
    if *flag_bits == 8 {
        *flag_index = out.len();
        out.push(0);
        *flag_bits = 0;
    }
    if is_match {
        out[*flag_index] |= 1 << (7 - *flag_bits);
    }
    *flag_bits += 1;
    out.extend_from_slice(bytes);
}

/// Record the hash-chain entry for `pos` (requires a full 3-byte window).
fn insert(input: &[u8], head: &mut [i32], prev: &mut [i32], pos: usize, n: usize) {
    if pos + MIN_MATCH <= n {
        let h = hash_at(input, pos);
        prev[pos] = head[h];
        head[h] = pos as i32;
    }
}

/// An error encountered while decoding a `dxz` frame.
#[derive(Debug, PartialEq, Eq)]
pub enum DecompressError {
    /// The frame is shorter than the header, or names a codec this build cannot read.
    InvalidMagic,
    /// The payload ended before producing the declared number of bytes.
    Truncated,
    /// A back-reference pointed outside the already-decoded output.
    CorruptMatch,
    /// The DEFLATE payload is damaged.
    Damaged,
}

impl fmt::Display for DecompressError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            DecompressError::InvalidMagic => "dxz: unknown frame codec",
            DecompressError::Truncated => "dxz: truncated frame",
            DecompressError::CorruptMatch => "dxz: corrupt match token",
            DecompressError::Damaged => "dxz: damaged compressed payload",
        };
        f.write_str(message)
    }
}

impl std::error::Error for DecompressError {}

/// Read a big-endian `u32` from `bytes` at `offset`.
#[inline]
fn read_u32_be(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

/// Decompress any `dxz` frame, whichever codec wrote it.
///
/// The codec comes from the frame's magic, so a pack written by an older build still reads.
/// In every case the result is checked against the length the frame declares: a payload
/// that decodes to the wrong size is reported, never returned short.
///
/// Complexity: `O(n)` time and space in the decompressed output length.
pub fn decompress(frame: &[u8]) -> Result<Vec<u8>, DecompressError> {
    if frame.len() < HEADER_LENGTH {
        return Err(DecompressError::InvalidMagic);
    }
    let original_length = read_u32_be(frame, 4) as usize;
    // The declared length is four attacker-controlled bytes, and it is read *before* a single
    // byte of payload is examined. Trusting it to size an allocation lets a twelve-byte frame
    // ask for four gigabytes; trusting it to bound an inflate lets a small payload expand
    // without limit. Neither is a real document, so both are refused here rather than
    // survived.
    if original_length > MAX_DECOMPRESSED {
        return Err(DecompressError::Damaged);
    }
    let payload = &frame[HEADER_LENGTH..];

    // The header-length check above guarantees four bytes; the slice pattern proves it to
    // the compiler, so no fallible conversion (and no panic path) is needed.
    let &[m0, m1, m2, m3, ..] = frame else {
        return Err(DecompressError::InvalidMagic);
    };
    match &[m0, m1, m2, m3] {
        MAGIC_DEFLATE => {
            let out = miniz_oxide::inflate::decompress_to_vec_with_limit(payload, original_length)
                .map_err(|_| DecompressError::Damaged)?;
            if out.len() != original_length {
                return Err(DecompressError::Truncated);
            }
            Ok(out)
        }
        MAGIC_STORED => {
            if payload.len() != original_length {
                return Err(DecompressError::Truncated);
            }
            Ok(payload.to_vec())
        }
        MAGIC_LZSS => lzss_decompress(frame, original_length),
        _ => Err(DecompressError::InvalidMagic),
    }
}

/// Decompress the token stream of a `DXZ1` frame.
fn lzss_decompress(frame: &[u8], original_length: usize) -> Result<Vec<u8>, DecompressError> {
    // Grown as tokens are decoded rather than reserved from the declared length: the frame is
    // untrusted, and the loop below already stops at `Truncated` when the tokens run out, so
    // a frame that claims more than it carries costs only what it actually decoded.
    let mut out: Vec<u8> = Vec::new();
    let mut i = HEADER_LENGTH;

    while out.len() < original_length {
        if i >= frame.len() {
            return Err(DecompressError::Truncated);
        }
        let flag = frame[i];
        i += 1;

        let mut bit = 0;
        while bit < 8 && out.len() < original_length {
            let is_match = (flag >> (7 - bit)) & 1 == 1;
            if is_match {
                if i + 3 > frame.len() {
                    return Err(DecompressError::Truncated);
                }
                let length = frame[i] as usize + MIN_MATCH;
                let distance = ((frame[i + 1] as usize) << 8) | frame[i + 2] as usize;
                i += 3;

                let from = match out.len().checked_sub(distance) {
                    Some(from) if distance >= 1 && out.len() + length <= original_length => from,
                    _ => return Err(DecompressError::CorruptMatch),
                };
                for k in 0..length {
                    let value = out[from + k];
                    out.push(value);
                }
            } else {
                if i >= frame.len() {
                    return Err(DecompressError::Truncated);
                }
                out.push(frame[i]);
                i += 1;
            }
            bit += 1;
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(bytes: &[u8]) {
        // Whatever codec won, the bytes must come back exactly, and the frame must never
        // be bigger than storing the input outright.
        let frame = compress(bytes);
        assert!(
            [MAGIC_DEFLATE, MAGIC_STORED].contains(&frame[0..4].try_into().expect("magic")),
            "unexpected codec: {:?}",
            &frame[0..4]
        );
        assert!(
            frame.len() <= HEADER_LENGTH + bytes.len(),
            "compression inflated"
        );
        assert_eq!(decompress(&frame).unwrap(), bytes);

        // The superseded frame still decodes: packs in real repositories contain it.
        let legacy = lzss_compress(bytes);
        assert_eq!(&legacy[0..4], MAGIC_LZSS);
        assert_eq!(decompress(&legacy).unwrap(), bytes);
    }

    #[test]
    fn round_trips_edge_sizes() {
        for len in [
            0usize, 1, 2, 3, 4, 7, 8, 9, 255, 256, 258, 259, 1000, 65535, 70000,
        ] {
            let zeros = vec![0u8; len];
            round_trip(&zeros);
            let seq: Vec<u8> = (0..len).map(|i| (i & 0xff) as u8).collect();
            round_trip(&seq);
        }
    }

    #[test]
    fn round_trips_repetitive_and_text() {
        round_trip(b"ab".repeat(5000).as_slice());
        round_trip(
            "::heading level=1 id=x\nHello\n::end\n"
                .repeat(300)
                .as_bytes(),
        );
    }

    #[test]
    fn round_trips_overlap_heavy_small_alphabet() {
        // A tiny alphabet maximises overlapping back-references (run encoding).
        let mut state = 0x1234_5678u32;
        let data: Vec<u8> = (0..4000)
            .map(|_| {
                state = state.wrapping_mul(1_103_515_245).wrapping_add(12345);
                (state >> 24) as u8 & 0x03
            })
            .collect();
        round_trip(&data);
    }

    /// A frame's declared length is untrusted input. Before this was bounded, each of these
    /// asked the allocator for the declared size before looking at the payload — so a frame
    /// smaller than this comment could stop the process.
    #[test]
    fn a_frame_cannot_claim_a_length_it_does_not_carry() {
        for magic in [MAGIC_LZSS, MAGIC_DEFLATE, MAGIC_STORED] {
            let mut frame = magic.to_vec();
            frame.extend_from_slice(&u32::MAX.to_be_bytes()); // "four gigabytes follow"
            frame.extend_from_slice(b"they do not");
            assert!(
                decompress(&frame).is_err(),
                "{} accepted an impossible length",
                String::from_utf8_lossy(magic)
            );
        }
    }

    /// A small payload that inflates without limit is the other half of the same problem.
    #[test]
    fn a_deflate_payload_cannot_expand_past_what_the_frame_declared() {
        let bomb = compress(&vec![0u8; 4 * 1024 * 1024]);
        assert_eq!(&bomb[..4], MAGIC_DEFLATE, "a run of zeros should deflate");

        // Keep the payload, understate the length: inflating must stop at the declared size
        // rather than run to completion and hand back more than was promised.
        let mut lying = MAGIC_DEFLATE.to_vec();
        lying.extend_from_slice(&64u32.to_be_bytes());
        lying.extend_from_slice(&bomb[HEADER_LENGTH..]);
        assert!(decompress(&lying).is_err());
    }

    #[test]
    fn frames_written_before_deflate_still_decode() {
        // Frames captured before DEFLATE existed, byte-for-byte. Two things are pinned:
        // the LZSS writer still produces exactly these bytes, and — the part that matters
        // for anyone with a pack already committed — `decompress` still reads them.
        fn hex(bytes: &[u8]) -> String {
            bytes.iter().map(|b| format!("{b:02x}")).collect()
        }
        fn unhex(text: &str) -> Vec<u8> {
            (0..text.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&text[i..i + 2], 16).expect("hex"))
                .collect()
        }

        for (source, frame) in [
            ("", "44585a310000000000"),
            ("abc", "44585a310000000300616263"),
            (
                "hello hello hello hello",
                "44585a31000000170268656c6c6f200e0006",
            ),
            (
                "::heading level=1 id=x\nHi\n::end\n",
                "44585a3100000020003a3a68656164696e0067206c6576656c3d00312069643d780a4800690a3a3a656e640a",
            ),
        ] {
            assert_eq!(hex(&lzss_compress(source.as_bytes())), frame);
            assert_eq!(decompress(&unhex(frame)).expect("legacy frame"), source.as_bytes());
        }
    }

    #[test]
    fn rejects_bad_magic() {
        assert_eq!(
            decompress(b"nope").unwrap_err(),
            DecompressError::InvalidMagic
        );
        assert_eq!(
            decompress(&[0, 1, 2, 3, 4, 5, 6, 7]).unwrap_err(),
            DecompressError::InvalidMagic
        );
    }
}
