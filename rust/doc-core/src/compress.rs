//! `dxz1` — the document platform's own byte compressor.
//!
//! A self-contained LZSS (Lempel–Ziv sliding-window) codec. Because the platform owns
//! both ends of the bundle format, the codec is not required to interoperate with
//! DEFLATE or brotli — only to round-trip exactly. This Rust implementation produces
//! **byte-identical** frames to the TypeScript reference (`src/core/compress.ts`), so a
//! bundle written by the native binary decompresses in the wasm editor and vice versa.
//!
//! Frame layout:
//! ```text
//!   [0..3]  magic 'DXZ1'
//!   [4..7]  original byte length (u32, big-endian)
//!   [8..]   token stream
//! ```
//! Token stream: a flag byte precedes each group of up to 8 tokens; bit `(7 - i)`, MSB
//! first, marks token `i` as a back-reference (`1`) or literal (`0`).

use core::fmt;

const MAGIC: u32 = 0x4458_5a31; // 'DXZ1'
const HEADER_LENGTH: usize = 8;

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

/// Compress raw bytes into a `dxz1` frame. Infallible: every input round-trips.
///
/// Complexity: `O(n · MAX_CHAIN)` time, where `n` is the input length and `MAX_CHAIN`
/// is the fixed cap on match-candidate probes per position — so the LZSS match search is
/// bounded to linear time in `n` — and `O(n)` extra space for the hash chain tables.
pub fn compress(input: &[u8]) -> Vec<u8> {
    let n = input.len();
    let mut out: Vec<u8> = Vec::with_capacity(HEADER_LENGTH + n);
    out.extend_from_slice(&MAGIC.to_be_bytes());
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

/// An error encountered while decoding a `dxz1` frame.
#[derive(Debug, PartialEq, Eq)]
pub enum DecompressError {
    /// The frame is shorter than the header or has the wrong magic bytes.
    InvalidMagic,
    /// The token stream ended before producing the declared number of bytes.
    Truncated,
    /// A back-reference pointed outside the already-decoded output.
    CorruptMatch,
}

impl fmt::Display for DecompressError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            DecompressError::InvalidMagic => "dxz1: invalid frame magic",
            DecompressError::Truncated => "dxz1: truncated frame",
            DecompressError::CorruptMatch => "dxz1: corrupt match token",
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

/// Decompress a `dxz1` frame produced by [`compress`].
///
/// Complexity: `O(n)` time and space in the decompressed output length, since each output
/// byte is produced once by copying a literal or an already-emitted back-reference.
pub fn decompress(frame: &[u8]) -> Result<Vec<u8>, DecompressError> {
    if frame.len() < HEADER_LENGTH || read_u32_be(frame, 0) != MAGIC {
        return Err(DecompressError::InvalidMagic);
    }

    let original_length = read_u32_be(frame, 4) as usize;
    let mut out: Vec<u8> = Vec::with_capacity(original_length);
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
        let frame = compress(bytes);
        assert_eq!(&frame[0..4], b"DXZ1");
        assert_eq!(decompress(&frame).unwrap(), bytes);
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

    #[test]
    fn matches_typescript_reference_vectors() {
        // Frames captured from the TypeScript reference (`src/core/compress.ts`). Equal
        // bytes here prove native and wasm builds interoperate with each other.
        fn hex(bytes: &[u8]) -> String {
            bytes.iter().map(|b| format!("{b:02x}")).collect()
        }
        assert_eq!(hex(&compress(b"")), "44585a310000000000");
        assert_eq!(hex(&compress(b"abc")), "44585a310000000300616263");
        assert_eq!(
            hex(&compress(b"hello hello hello hello")),
            "44585a31000000170268656c6c6f200e0006"
        );
        assert_eq!(
            hex(&compress("::heading level=1 id=x\nHi\n::end\n".as_bytes())),
            "44585a3100000020003a3a68656164696e0067206c6576656c3d00312069643d780a4800690a3a3a656e640a"
        );
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
