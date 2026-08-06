//! Base64 (RFC 4648) encoding and decoding — the platform's one copy of the codec.
//!
//! It lives in `doc-core` because hydration embeds an `::image src=` file as a `data:`
//! URI, and the other callers sit above this crate: the MCP server encodes captured
//! images into tool results, and the DevTools client decodes the frames the browser
//! hands back (both through `doc_shot::base64`, which re-exports this module). Owned
//! outright rather than taken as a dependency, and shared by every caller.

/// The standard base64 alphabet (RFC 4648).
const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Encode `bytes` as standard base64 with `=` padding.
#[must_use]
pub fn encode(bytes: &[u8]) -> String {
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

/// Decode standard base64, tolerating padding and line breaks, rejecting anything else.
///
/// # Errors
/// Returns a message naming the offending byte when `text` is not base64.
pub fn decode(text: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut block: u32 = 0;
    let mut bits: u32 = 0;

    for byte in text.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\r' | b'\n' => continue,
            other => return Err(format!("not base64: byte 0x{other:02x}")),
        };
        block = block << 6 | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((block >> bits) as u8);
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_matches_the_rfc_4648_test_vectors() {
        assert_eq!(encode(b""), "");
        assert_eq!(encode(b"f"), "Zg==");
        assert_eq!(encode(b"fo"), "Zm8=");
        assert_eq!(encode(b"foo"), "Zm9v");
        assert_eq!(encode(b"foob"), "Zm9vYg==");
        assert_eq!(encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn encode_handles_binary_bytes_including_high_values() {
        assert_eq!(encode(&[0x00, 0xff, 0x80]), "AP+A");
        assert_eq!(encode(&[0x89, b'P', b'N', b'G']), "iVBORw==");
    }

    #[test]
    fn encoded_length_is_always_a_multiple_of_four() {
        for length in 0..40 {
            let encoded = encode(&vec![0x41; length]);
            assert_eq!(encoded.len() % 4, 0, "bad padding at length {length}");
        }
    }

    #[test]
    fn decode_reverses_encode_for_every_length() {
        for length in 0..40 {
            let bytes: Vec<u8> = (0..length as u8).map(|n| n.wrapping_mul(37)).collect();
            assert_eq!(decode(&encode(&bytes)).expect("round trip"), bytes);
        }
    }

    #[test]
    fn decode_tolerates_line_breaks_and_padding() {
        assert_eq!(decode("Zm9v\r\nYmFy").expect("decode"), b"foobar");
        assert_eq!(decode("Zg==").expect("decode"), b"f");
    }

    #[test]
    fn decode_rejects_bytes_outside_the_alphabet() {
        let error = decode("Zm9v!").expect_err("should refuse");
        assert!(error.contains("0x21"), "{error}");
    }
}
