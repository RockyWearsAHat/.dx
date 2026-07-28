//! `DXBUN5` encoder: serialize bundle entries into the compressed byte container.

use super::{BundleEntry, FLAGS_HEADER_MIN, MAGIC};
use crate::compress::compress;

/// Encode entries into a `DXBUN5` bundle blob.
///
/// Entries are written in the given order; the same order is recovered by
/// [`super::decode_bundle`]. Paths longer than 255 bytes are truncated at a UTF-8 char
/// boundary so the single-byte path length never overflows (callers should keep paths
/// short; this is a safety clamp, not a feature).
///
/// Complexity: `O(total bytes)` time and space across every entry's path and packed
/// payload, since each is copied once into the buffer that is then `dxz1`-compressed.
pub fn encode_bundle(entries: &[BundleEntry]) -> Vec<u8> {
    let mut payload: Vec<u8> = Vec::new();
    payload.extend_from_slice(&(entries.len() as u32).to_le_bytes());

    for entry in entries {
        let path_bytes = clamp_path(&entry.path);
        payload.push(path_bytes.len() as u8);
        payload.extend_from_slice(path_bytes);
        payload.push(entry.git.to_u8());
        payload.extend_from_slice(&(entry.packed.len() as u32).to_le_bytes());
    }
    for entry in entries {
        payload.extend_from_slice(&entry.packed);
    }

    let compressed = compress(&payload);
    let mut out = Vec::with_capacity(FLAGS_HEADER_MIN + compressed.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
    out.extend_from_slice(&compressed);
    out
}

/// Clamp a path to at most 255 UTF-8 bytes on a char boundary.
fn clamp_path(path: &str) -> &[u8] {
    if path.len() <= 255 {
        return path.as_bytes();
    }
    let mut end = 255;
    while end > 0 && !path.is_char_boundary(end) {
        end -= 1;
    }
    &path.as_bytes()[..end]
}
