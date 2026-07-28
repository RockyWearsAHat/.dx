//! `DXBUN5` decoder: parse the compressed byte container back into bundle entries.

use super::{BundleEntry, BundleError, GitFlags, FLAGS_HEADER_MIN, MAGIC};
use crate::compress::decompress;

/// Decode a `DXBUN5` bundle blob back into its entries.
///
/// Returns [`BundleError`] (never panics) on bad magic, a length header that disagrees
/// with the data, a payload that fails to decompress, a path that is not valid UTF-8, or
/// any field that runs past the end of the decompressed payload.
///
/// Complexity: `O(total bytes)` time and space in the decompressed payload, since the
/// cursor advances over each header and packed payload exactly once.
pub fn decode_bundle(bytes: &[u8]) -> Result<Vec<BundleEntry>, BundleError> {
    if bytes.len() < FLAGS_HEADER_MIN || &bytes[..MAGIC.len()] != MAGIC {
        return Err(BundleError::InvalidMagic);
    }

    let declared = u32::from_le_bytes([
        bytes[MAGIC.len()],
        bytes[MAGIC.len() + 1],
        bytes[MAGIC.len() + 2],
        bytes[MAGIC.len() + 3],
    ]) as usize;
    let compressed = &bytes[FLAGS_HEADER_MIN..];
    if compressed.len() != declared {
        return Err(BundleError::LengthMismatch);
    }

    let payload = decompress(compressed)?;
    decode_payload(&payload)
}

/// Parse the decompressed payload (entry count, headers, then packed blobs).
fn decode_payload(payload: &[u8]) -> Result<Vec<BundleEntry>, BundleError> {
    let mut cursor = Cursor {
        buf: payload,
        off: 0,
    };
    let count = cursor.u32_le()? as usize;

    // First pass: read every fixed header (path, flags, payload length).
    let mut headers: Vec<(String, GitFlags, usize)> = Vec::with_capacity(count);
    for _ in 0..count {
        let path_len = cursor.byte()? as usize;
        let path = cursor.utf8(path_len)?;
        let git = GitFlags::from_u8(cursor.byte()?);
        let packed_len = cursor.u32_le()? as usize;
        headers.push((path, git, packed_len));
    }

    // Second pass: slice each packed payload in declaration order.
    let mut entries = Vec::with_capacity(count);
    for (path, git, packed_len) in headers {
        let packed = cursor.bytes(packed_len)?.to_vec();
        entries.push(BundleEntry { path, git, packed });
    }
    Ok(entries)
}

/// A forward-only reader over a decompressed bundle payload.
struct Cursor<'a> {
    buf: &'a [u8],
    off: usize,
}

impl<'a> Cursor<'a> {
    fn byte(&mut self) -> Result<u8, BundleError> {
        let value = *self.buf.get(self.off).ok_or(BundleError::Truncated)?;
        self.off += 1;
        Ok(value)
    }

    fn u32_le(&mut self) -> Result<u32, BundleError> {
        let slice = self.bytes(4)?;
        Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
    }

    fn bytes(&mut self, len: usize) -> Result<&'a [u8], BundleError> {
        let end = self.off.checked_add(len).ok_or(BundleError::Truncated)?;
        let slice = self.buf.get(self.off..end).ok_or(BundleError::Truncated)?;
        self.off = end;
        Ok(slice)
    }

    fn utf8(&mut self, len: usize) -> Result<String, BundleError> {
        let slice = self.bytes(len)?;
        // Paths must be exact UTF-8 (unlike tolerant body text), so a bad path is an error.
        core::str::from_utf8(slice)
            .map(str::to_string)
            .map_err(|_| BundleError::InvalidPath)
    }
}
