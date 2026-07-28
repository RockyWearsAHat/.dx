//! `DXBUN5` — the bundle container that stores many packed documents in one blob.
//!
//! A bundle is the on-disk archive format (`.doc/.repo-docs.bin`,
//! `.doc/.local-docs.bin` in the reference). Each entry pairs a document path with its
//! per-file git tracking flags and the document's DOCB1-packed bytes. The whole entry
//! table plus payloads are concatenated into a single payload, which is then compressed
//! with the `dxz1` codec ([`crate::compress`]). File I/O is a host concern; this module
//! only encodes and decodes the byte container, so it stays `wasm32`-friendly.
//!
//! The container is split across submodules: [`encode`] writes a bundle, [`decode`] reads
//! one back, and this module owns the shared types ([`GitFlags`], [`BundleEntry`],
//! [`BundleError`]) and the on-wire layout constants.
//!
//! # Byte layout
//! ```text
//!   [0..6]   magic b"DXBUN5"
//!   [6..10]  u32 LE = length of the dxz1-compressed payload that follows
//!   [10..]   compress(payload)
//!
//!   payload (after decompress):
//!   [0..4]   u32 LE entry count
//!   then, for every entry in order, a header:
//!     [u8]   path byte length (<= 255)
//!     [..]   path, UTF-8
//!     [u8]   git flags (bit0 tracked, bit1 untracked, bit2 ignored,
//!                        bit3 modified, bit4 staged)
//!     [u32 LE] packed payload length
//!   then every entry's packed payload, concatenated in the same order.
//! ```

mod decode;
mod encode;

pub use decode::decode_bundle;
pub use encode::encode_bundle;

use crate::compress::DecompressError;
use core::fmt;

/// The bundle magic prefix.
pub(super) const MAGIC: &[u8] = b"DXBUN5";
/// The minimum header length (magic plus the compressed-payload length field).
pub(super) const FLAGS_HEADER_MIN: usize = MAGIC.len() + 4;

/// Per-document git tracking state, mirroring the reference's working-tree flags.
///
/// The five booleans are independent state observed from git; this type only carries
/// and (de)serializes them. Bit positions in the packed `u8` are fixed by the format:
/// bit0 `tracked`, bit1 `untracked`, bit2 `ignored`, bit3 `modified`, bit4 `staged`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GitFlags {
    /// The file is tracked by git (committed at least once).
    pub tracked: bool,
    /// The file is present in the working tree but not tracked.
    pub untracked: bool,
    /// The file is ignored by git (matches a `.gitignore` rule).
    pub ignored: bool,
    /// The file has uncommitted modifications in the working tree.
    pub modified: bool,
    /// The file has staged changes in the index.
    pub staged: bool,
}

impl GitFlags {
    /// Pack the five flags into a `u8` (bit0 tracked … bit4 staged).
    pub fn to_u8(self) -> u8 {
        u8::from(self.tracked)
            | (u8::from(self.untracked) << 1)
            | (u8::from(self.ignored) << 2)
            | (u8::from(self.modified) << 3)
            | (u8::from(self.staged) << 4)
    }

    /// Unpack the five flags from a `u8` (bit0 tracked … bit4 staged).
    ///
    /// Bits 5..7 are reserved and ignored, so unknown high bits decode cleanly.
    pub fn from_u8(byte: u8) -> Self {
        GitFlags {
            tracked: byte & 0b0_0001 != 0,
            untracked: byte & 0b0_0010 != 0,
            ignored: byte & 0b0_0100 != 0,
            modified: byte & 0b0_1000 != 0,
            staged: byte & 0b1_0000 != 0,
        }
    }
}

/// One document stored in a bundle: its path, git flags, and DOCB1-packed bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleEntry {
    /// Repo-relative document path (the `.dx` stub location). UTF-8, at most 255 bytes.
    pub path: String,
    /// Git tracking flags for this document's file.
    pub git: GitFlags,
    /// The document packed with [`crate::docbin::pack`].
    pub packed: Vec<u8>,
}

/// An error encountered while decoding a `DXBUN5` bundle.
#[derive(Debug, PartialEq, Eq)]
pub enum BundleError {
    /// The bundle is shorter than the header or lacks the `DXBUN5` magic.
    InvalidMagic,
    /// The declared compressed-payload length does not match the available bytes.
    LengthMismatch,
    /// The compressed payload failed to decompress.
    Decompress(DecompressError),
    /// The decompressed payload ended mid-field.
    Truncated,
    /// A path declared a length that is not valid UTF-8.
    InvalidPath,
}

impl fmt::Display for BundleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BundleError::InvalidMagic => f.write_str("DXBUN5: invalid magic"),
            BundleError::LengthMismatch => f.write_str("DXBUN5: payload length mismatch"),
            BundleError::Decompress(inner) => write!(f, "DXBUN5: {inner}"),
            BundleError::Truncated => f.write_str("DXBUN5: truncated payload"),
            BundleError::InvalidPath => f.write_str("DXBUN5: path is not valid UTF-8"),
        }
    }
}

impl std::error::Error for BundleError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            BundleError::Decompress(inner) => Some(inner),
            _ => None,
        }
    }
}

impl From<DecompressError> for BundleError {
    fn from(inner: DecompressError) -> Self {
        BundleError::Decompress(inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::digest::sha256_hex;
    use crate::docbin::pack;
    use crate::format::parse;

    fn entry(path: &str, git: GitFlags, packed: Vec<u8>) -> BundleEntry {
        BundleEntry {
            path: path.to_string(),
            git,
            packed,
        }
    }

    fn packed_example(name: &str) -> Vec<u8> {
        let source = std::fs::read_to_string(format!(
            "{}/../../examples/{name}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .expect("read example fixture");
        pack(&parse(&source))
    }

    #[test]
    fn git_flags_pack_round_trip() {
        for bits in 0u8..=0b1_1111 {
            let flags = GitFlags::from_u8(bits);
            assert_eq!(flags.to_u8(), bits);
        }
        let all = GitFlags {
            tracked: true,
            untracked: true,
            ignored: true,
            modified: true,
            staged: true,
        };
        assert_eq!(all.to_u8(), 0b1_1111);
        // High reserved bits are ignored on decode.
        assert_eq!(GitFlags::from_u8(0b1110_0000), GitFlags::default());
    }

    #[test]
    fn round_trips_zero_entries() {
        let bytes = encode_bundle(&[]);
        assert_eq!(&bytes[..MAGIC.len()], MAGIC);
        assert_eq!(decode_bundle(&bytes).unwrap(), Vec::<BundleEntry>::new());
    }

    #[test]
    fn round_trips_one_entry() {
        let flags = GitFlags {
            tracked: true,
            modified: true,
            ..GitFlags::default()
        };
        let entries = vec![entry("docs/a.dx", flags, vec![1, 2, 3, 4, 5])];
        let bytes = encode_bundle(&entries);
        assert_eq!(decode_bundle(&bytes).unwrap(), entries);
    }

    #[test]
    fn round_trips_many_entries_with_unicode_paths() {
        let entries = vec![
            entry("docs/welcome.dx", GitFlags::default(), vec![9, 8, 7]),
            entry(
                "docs/архитектура.dx",
                GitFlags {
                    untracked: true,
                    ..GitFlags::default()
                },
                vec![],
            ),
            entry(
                "notes/日本語/メモ.dx",
                GitFlags {
                    ignored: true,
                    staged: true,
                    ..GitFlags::default()
                },
                vec![42; 100],
            ),
        ];
        let bytes = encode_bundle(&entries);
        assert_eq!(decode_bundle(&bytes).unwrap(), entries);
    }

    #[test]
    fn round_trips_real_packed_examples() {
        let entries = vec![
            entry(
                "examples/welcome.dx",
                GitFlags {
                    tracked: true,
                    ..GitFlags::default()
                },
                packed_example("welcome.dx"),
            ),
            entry(
                "examples/tutorial.dx",
                GitFlags {
                    tracked: true,
                    modified: true,
                    ..GitFlags::default()
                },
                packed_example("tutorial.dx"),
            ),
        ];
        let bytes = encode_bundle(&entries);
        let decoded = decode_bundle(&bytes).unwrap();
        assert_eq!(decoded, entries);
        // Packed bytes survive intact (digest is stable across the bundle round-trip).
        assert_eq!(
            sha256_hex(&decoded[0].packed),
            sha256_hex(&entries[0].packed)
        );
    }

    #[test]
    fn round_trips_large_packed_document() {
        // A >64 KB packed payload exercises the u32 length fields and the codec.
        let large = vec![0xABu8; 80_000];
        let entries = vec![entry("big.dx", GitFlags::default(), large)];
        let bytes = encode_bundle(&entries);
        assert_eq!(decode_bundle(&bytes).unwrap(), entries);
    }

    #[test]
    fn rejects_bad_magic() {
        assert_eq!(decode_bundle(b"").unwrap_err(), BundleError::InvalidMagic);
        assert_eq!(
            decode_bundle(b"XXXXXX\x00\x00\x00\x00").unwrap_err(),
            BundleError::InvalidMagic
        );
    }

    #[test]
    fn rejects_length_mismatch() {
        let mut bytes = encode_bundle(&[entry("a.dx", GitFlags::default(), vec![1, 2])]);
        bytes.push(0); // declared length no longer matches trailing bytes.
        assert_eq!(
            decode_bundle(&bytes).unwrap_err(),
            BundleError::LengthMismatch
        );
    }

    #[test]
    fn rejects_garbage_payload() {
        // Valid header framing, but the "compressed" region is not a dxz1 frame.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&4u32.to_le_bytes());
        bytes.extend_from_slice(&[0, 1, 2, 3]);
        assert!(matches!(
            decode_bundle(&bytes).unwrap_err(),
            BundleError::Decompress(_)
        ));
    }

    #[test]
    fn rejects_truncated_payload() {
        // A valid dxz1 frame whose decoded payload claims one entry but is too short.
        let truncated = crate::compress::compress(&5u32.to_le_bytes()); // count = 5, no headers.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&(truncated.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&truncated);
        assert_eq!(decode_bundle(&bytes).unwrap_err(), BundleError::Truncated);
    }
}
