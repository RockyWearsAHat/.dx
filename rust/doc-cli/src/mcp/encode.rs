//! Base64 encoding for image payloads.
//!
//! MCP carries images as base64 text, and this is the only place the platform needs the
//! encoding — small enough to own outright rather than take a dependency for.

/// The standard base64 alphabet (RFC 4648).
const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Encode `bytes` as standard base64 with `=` padding.
#[must_use]
pub fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);

    for chunk in bytes.chunks(3) {
        let block = u32::from(chunk[0]) << 16
            | u32::from(chunk.get(1).copied().unwrap_or(0)) << 8
            | u32::from(chunk.get(2).copied().unwrap_or(0));

        for offset in 0..4 {
            if offset <= chunk.len() {
                let index = (block >> (18 - offset * 6)) & 0b11_1111;
                out.push(ALPHABET[index as usize] as char);
            } else {
                out.push('=');
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_rfc_4648_test_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn encodes_binary_bytes_including_high_values() {
        assert_eq!(base64(&[0x00, 0xff, 0x80]), "AP+A");
        assert_eq!(base64(&[0x89, b'P', b'N', b'G']), "iVBORw==");
    }

    #[test]
    fn output_length_is_always_a_multiple_of_four() {
        for length in 0..40 {
            let encoded = base64(&vec![0x41; length]);
            assert_eq!(encoded.len() % 4, 0, "bad padding at length {length}");
        }
    }
}
