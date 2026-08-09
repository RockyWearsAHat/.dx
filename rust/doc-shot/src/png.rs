//! The platform's one PNG codec — decode a capture, resample it in linear light, encode
//! the result.
//!
//! This exists for one reason: a reading capture is rasterized larger than the page it
//! delivers ([`crate::ShotOptions::oversample`]) and averaged back down here, so a
//! hairline rule or a faint mark survives the shrink as gray ink instead of falling
//! between the raster and vanishing. The shrink has to preserve shapes, which means it
//! has to happen in linear light — averaging sRGB bytes directly darkens every edge and
//! is exactly the "just whatever" resize this module replaces.
//!
//! The codec covers what a browser screenshot needs and no more: 8-bit depth, the four
//! plain color types, no interlacing. Anything else is refused with a sentence, never
//! misread.

use miniz_oxide::deflate::compress_to_vec_zlib;
use miniz_oxide::inflate::decompress_to_vec_zlib_with_limit;

/// Most pixels one decode will materialize as raw RGBA (~256 MB).
///
/// A capture this crate took is far under it; a PNG *claiming* more is asking this
/// process to allocate until it dies, and is refused instead — the same rule the
/// `dxz` frames follow: bound anything sized by input.
const MAX_PIXELS: u64 = 64_000_000;

/// DEFLATE effort for the encoder: the sweet spot where a screenshot stops shrinking
/// meaningfully but the encode stays a small fraction of the capture's browser launch.
const DEFLATE_LEVEL: u8 = 6;

/// A decoded image: 8-bit RGBA, row-major, no padding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    /// Width in pixels, never zero.
    pub width: u32,
    /// Height in pixels, never zero.
    pub height: u32,
    /// Exactly `width × height × 4` bytes.
    pub rgba: Vec<u8>,
}

/// Shrink a PNG by an integer `factor`, averaging each `factor × factor` block in
/// linear light, and hand back the re-encoded PNG.
///
/// A `factor` of 1 (or 0) is the identity and returns the bytes untouched. The output
/// covers the whole input: edge blocks that fall short of `factor` pixels average what
/// is there, so no row or column is dropped.
///
/// # Errors
/// A sentence when the PNG cannot be decoded (truncated, interlaced, an exotic color
/// type) — the caller still holds the original bytes and can decide to use them as they
/// are.
pub fn downsample(png: &[u8], factor: u32) -> Result<Vec<u8>, String> {
    if factor <= 1 {
        return Ok(png.to_vec());
    }
    let image = decode(png)?;
    Ok(encode(&shrink(&image, factor)))
}

/// The linear-light box average of each `factor × factor` block of `image`.
fn shrink(image: &Image, factor: u32) -> Image {
    // sRGB byte → linear intensity, tabled once: the transfer curve is the whole reason
    // this averaging is honest, and 256 entries make it free.
    let mut linear = [0f32; 256];
    for (byte, slot) in linear.iter_mut().enumerate() {
        let value = byte as f32 / 255.0;
        *slot = if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        };
    }
    let to_srgb = |value: f32| -> u8 {
        let value = if value <= 0.003_130_8 {
            value * 12.92
        } else {
            1.055 * value.powf(1.0 / 2.4) - 0.055
        };
        (value.clamp(0.0, 1.0) * 255.0).round() as u8
    };

    let out_width = image.width.div_ceil(factor).max(1);
    let out_height = image.height.div_ceil(factor).max(1);
    let mut rgba = Vec::with_capacity(out_width as usize * out_height as usize * 4);
    for out_y in 0..out_height {
        let y0 = out_y * factor;
        let y1 = (y0 + factor).min(image.height);
        for out_x in 0..out_width {
            let x0 = out_x * factor;
            let x1 = (x0 + factor).min(image.width);
            // Color is averaged weighted by coverage (alpha), alpha by area: the color
            // of transparent pixels is meaningless and must not bleed into the block.
            let (mut sum, mut coverage, mut area) = ([0f32; 3], 0f32, 0f32);
            for y in y0..y1 {
                for x in x0..x1 {
                    let at = ((y * image.width + x) * 4) as usize;
                    let alpha = image.rgba[at + 3] as f32 / 255.0;
                    for channel in 0..3 {
                        sum[channel] += linear[image.rgba[at + channel] as usize] * alpha;
                    }
                    coverage += alpha;
                    area += 1.0;
                }
            }
            for channel in sum {
                rgba.push(if coverage > 0.0 {
                    to_srgb(channel / coverage)
                } else {
                    0
                });
            }
            rgba.push((coverage / area.max(1.0) * 255.0).round() as u8);
        }
    }
    Image {
        width: out_width,
        height: out_height,
        rgba,
    }
}

/// Bytes-per-pixel of each supported color type, or `None` for one this codec refuses.
fn channels(color_type: u8) -> Option<usize> {
    match color_type {
        0 => Some(1), // grayscale
        2 => Some(3), // RGB
        4 => Some(2), // grayscale + alpha
        6 => Some(4), // RGBA
        _ => None,    // palette needs a PLTE this codec has no reason to carry
    }
}

/// Stitch vertically-scrolled strips into one PNG: every strip must share the first's
/// width, rows are concatenated top to bottom, and the whole is re-encoded.
///
/// This is how a tall document is captured as a single picture: a `::view`'s sandboxed
/// frame only paints inside the viewport, so one full-height viewport ships the deep
/// frames as empty paper — the capture scrolls a window over the page and joins the
/// strips here instead.
///
/// # Errors
/// A sentence when the strips disagree about width, or when there are none.
pub fn stack(strips: &[Image]) -> Result<Vec<u8>, String> {
    let first = strips.first().ok_or("no strips to stitch")?;
    let width = first.width;
    let mut height: u32 = 0;
    let mut rgba = Vec::new();
    for strip in strips {
        if strip.width != width {
            return Err(format!(
                "strip width {} does not match the page's {width}",
                strip.width
            ));
        }
        height = height
            .checked_add(strip.height)
            .ok_or("the stitched page is too tall")?;
        rgba.extend_from_slice(&strip.rgba);
    }
    Ok(encode(&Image {
        width,
        height,
        rgba,
    }))
}

/// Decode an 8-bit, non-interlaced PNG into RGBA.
///
/// # Errors
/// A sentence naming what was malformed or unsupported. Chunk CRCs are not verified —
/// the input is a file this process just watched the browser write, and a flipped bit
/// surfaces as a filter or inflate error anyway.
pub fn decode(png: &[u8]) -> Result<Image, String> {
    let rest = png
        .strip_prefix(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A][..])
        .ok_or("not a PNG: missing signature")?;

    let mut chunks = ChunkReader { rest };
    let (kind, header) = chunks.next().ok_or("not a PNG: no chunks")??;
    if kind != *b"IHDR" || header.len() != 13 {
        return Err("not a PNG: the first chunk is not a 13-byte IHDR".into());
    }
    let width = u32::from_be_bytes(header[0..4].try_into().expect("13-byte IHDR"));
    let height = u32::from_be_bytes(header[4..8].try_into().expect("13-byte IHDR"));
    let (depth, color_type, interlace) = (header[8], header[9], header[12]);
    if width == 0 || height == 0 {
        return Err("empty image".into());
    }
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(format!(
            "a {width}x{height} image is larger than this codec will hold"
        ));
    }
    if depth != 8 || interlace != 0 {
        return Err(format!(
            "only 8-bit non-interlaced PNGs are supported (depth {depth}, interlace {interlace})"
        ));
    }
    let channels =
        channels(color_type).ok_or(format!("unsupported PNG color type {color_type}"))?;

    let mut deflated = Vec::new();
    for chunk in chunks {
        let (kind, data) = chunk?;
        match &kind {
            b"IDAT" => deflated.extend_from_slice(data),
            b"IEND" => break,
            _ => {} // ancillary chunks carry nothing a resample needs
        }
    }

    let stride = width as usize * channels;
    let raw_length = (stride + 1) * height as usize;
    let raw = decompress_to_vec_zlib_with_limit(&deflated, raw_length)
        .map_err(|error| format!("broken PNG image data: {error}"))?;
    if raw.len() != raw_length {
        return Err(format!(
            "PNG image data holds {} bytes where {raw_length} were declared",
            raw.len()
        ));
    }

    // Undo the per-row filter, then widen whatever channel layout this is to RGBA.
    let mut pixels = vec![0u8; stride * height as usize];
    for row in 0..height as usize {
        let filter = raw[row * (stride + 1)];
        let line = &raw[row * (stride + 1) + 1..(row + 1) * (stride + 1)];
        let start = row * stride;
        for (offset, &byte) in line.iter().enumerate() {
            let left = |pixels: &[u8]| {
                offset
                    .checked_sub(channels)
                    .map_or(0, |back| pixels[start + back])
            };
            let up = |pixels: &[u8]| {
                row.checked_sub(1)
                    .map_or(0, |_| pixels[start - stride + offset])
            };
            let up_left = |pixels: &[u8]| match (row.checked_sub(1), offset.checked_sub(channels)) {
                (Some(_), Some(back)) => pixels[start - stride + back],
                _ => 0,
            };
            let predicted = match filter {
                0 => 0,
                1 => left(&pixels),
                2 => up(&pixels),
                3 => ((left(&pixels) as u16 + up(&pixels) as u16) / 2) as u8,
                4 => paeth(left(&pixels), up(&pixels), up_left(&pixels)),
                other => return Err(format!("unknown PNG row filter {other}")),
            };
            pixels[start + offset] = byte.wrapping_add(predicted);
        }
    }

    let rgba = match color_type {
        6 => pixels,
        2 => pixels
            .chunks_exact(3)
            .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255])
            .collect(),
        0 => pixels
            .iter()
            .flat_map(|&gray| [gray, gray, gray, 255])
            .collect(),
        _ => pixels
            .chunks_exact(2)
            .flat_map(|pixel| [pixel[0], pixel[0], pixel[0], pixel[1]])
            .collect(),
    };
    Ok(Image {
        width,
        height,
        rgba,
    })
}

/// Walks a PNG's chunk stream, yielding `(type, data)` with every length bound-checked.
struct ChunkReader<'a> {
    rest: &'a [u8],
}

impl<'a> Iterator for ChunkReader<'a> {
    type Item = Result<([u8; 4], &'a [u8]), String>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.rest.is_empty() {
            return None;
        }
        if self.rest.len() < 8 {
            return Some(Err("truncated PNG chunk header".into()));
        }
        let length = u32::from_be_bytes(self.rest[0..4].try_into().expect("checked")) as usize;
        let kind: [u8; 4] = self.rest[4..8].try_into().expect("checked");
        let end = 8usize
            .checked_add(length)
            .and_then(|end| end.checked_add(4))
            .filter(|&end| end <= self.rest.len());
        let Some(end) = end else {
            return Some(Err(format!(
                "PNG chunk {} declares {length} bytes past the end of the file",
                String::from_utf8_lossy(&kind)
            )));
        };
        let data = &self.rest[8..8 + length];
        self.rest = &self.rest[end..];
        Some(Ok((kind, data)))
    }
}

/// The Paeth predictor: whichever neighbour is closest to `left + up - up_left`.
fn paeth(left: u8, up: u8, up_left: u8) -> u8 {
    let estimate = left as i16 + up as i16 - up_left as i16;
    let (to_left, to_up, to_up_left) = (
        (estimate - left as i16).abs(),
        (estimate - up as i16).abs(),
        (estimate - up_left as i16).abs(),
    );
    if to_left <= to_up && to_left <= to_up_left {
        left
    } else if to_up <= to_up_left {
        up
    } else {
        up_left
    }
}

/// Encode an [`Image`] as a PNG.
///
/// A fully opaque image is written as RGB — a quarter of the raw bytes for the alpha
/// channel a screenshot never uses. Each row takes whichever filter leaves it flattest
/// (the standard minimum-sum heuristic), which is most of what separates a small PNG
/// from a naive one.
#[must_use]
pub fn encode(image: &Image) -> Vec<u8> {
    let opaque = image.rgba.chunks_exact(4).all(|pixel| pixel[3] == 255);
    let (color_type, channels) = if opaque { (2u8, 3usize) } else { (6u8, 4usize) };
    let stride = image.width as usize * channels;

    let mut pixels = Vec::with_capacity(stride * image.height as usize);
    for pixel in image.rgba.chunks_exact(4) {
        pixels.extend_from_slice(&pixel[..channels]);
    }

    let mut raw = Vec::with_capacity((stride + 1) * image.height as usize);
    let mut filtered = vec![0u8; stride];
    for row in 0..image.height as usize {
        let line = &pixels[row * stride..(row + 1) * stride];
        let previous = row
            .checked_sub(1)
            .map(|above| &pixels[above * stride..row * stride]);
        let mut best: Option<(u32, u8)> = None;
        for filter in 0..=4u8 {
            let mut sum = 0u32;
            for (offset, &byte) in line.iter().enumerate() {
                let left = offset.checked_sub(channels).map_or(0, |back| line[back]);
                let up = previous.map_or(0, |above| above[offset]);
                let up_left = match (previous, offset.checked_sub(channels)) {
                    (Some(above), Some(back)) => above[back],
                    _ => 0,
                };
                let predicted = match filter {
                    0 => 0,
                    1 => left,
                    2 => up,
                    3 => ((left as u16 + up as u16) / 2) as u8,
                    _ => paeth(left, up, up_left),
                };
                let value = byte.wrapping_sub(predicted);
                filtered[offset] = value;
                // The heuristic's "flatness": residuals as signed magnitudes.
                sum += (value as i8).unsigned_abs() as u32;
            }
            if best.is_none_or(|(least, _)| sum < least) {
                best = Some((sum, filter));
                raw.truncate(row * (stride + 1));
                raw.push(filter);
                raw.extend_from_slice(&filtered);
            }
        }
    }

    let mut header = Vec::with_capacity(13);
    header.extend_from_slice(&image.width.to_be_bytes());
    header.extend_from_slice(&image.height.to_be_bytes());
    header.extend_from_slice(&[8, color_type, 0, 0, 0]);

    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    chunk(&mut png, b"IHDR", &header);
    chunk(
        &mut png,
        b"IDAT",
        &compress_to_vec_zlib(&raw, DEFLATE_LEVEL),
    );
    chunk(&mut png, b"IEND", &[]);
    png
}

/// Append one chunk: length, type, data, CRC over type + data.
fn chunk(png: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    png.extend_from_slice(&(data.len() as u32).to_be_bytes());
    png.extend_from_slice(kind);
    png.extend_from_slice(data);
    let mut crc = crc32(0xFFFF_FFFF, kind);
    crc = crc32(crc, data);
    png.extend_from_slice(&(crc ^ 0xFFFF_FFFF).to_be_bytes());
}

/// CRC-32 (the PNG/IEEE polynomial), continued from `state`.
fn crc32(state: u32, data: &[u8]) -> u32 {
    let mut crc = state;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & (crc & 1).wrapping_neg());
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(width: u32, height: u32, pixels: &[[u8; 4]]) -> Image {
        Image {
            width,
            height,
            rgba: pixels.iter().flatten().copied().collect(),
        }
    }

    #[test]
    fn stitched_strips_read_back_as_one_page_in_order() {
        let top = image(2, 1, &[[255, 0, 0, 255], [0, 255, 0, 255]]);
        let bottom = image(2, 2, &[[0; 4], [1, 2, 3, 255], [9, 9, 9, 255], [255; 4]]);
        let stitched =
            decode(&stack(&[top.clone(), bottom.clone()]).expect("stitch")).expect("png");
        assert_eq!((stitched.width, stitched.height), (2, 3));
        let joined: Vec<u8> = top.rgba.iter().chain(bottom.rgba.iter()).copied().collect();
        assert_eq!(stitched.rgba, joined);
    }

    #[test]
    fn strips_of_different_widths_are_refused_with_both_numbers() {
        let wide = image(3, 1, &[[0; 4], [0; 4], [0; 4]]);
        let narrow = image(2, 1, &[[0; 4], [0; 4]]);
        let error = stack(&[wide, narrow]).expect_err("mismatch");
        assert!(error.contains('2') && error.contains('3'), "{error}");
    }

    #[test]
    fn an_image_round_trips_through_encode_and_decode() {
        let original = image(
            2,
            2,
            &[
                [255, 0, 0, 255],
                [0, 255, 0, 128],
                [0, 0, 255, 255],
                [17, 34, 51, 0],
            ],
        );
        assert_eq!(decode(&encode(&original)).expect("round trip"), original);
    }

    #[test]
    fn an_opaque_image_round_trips_through_the_rgb_form() {
        let original = image(2, 1, &[[10, 20, 30, 255], [200, 100, 0, 255]]);
        let png = encode(&original);
        assert_eq!(decode(&png).expect("round trip"), original);
        // Color type 2 sits at byte 9 of the 13-byte IHDR, 8 signature + 8 chunk header in.
        assert_eq!(png[8 + 8 + 9], 2, "an opaque image is written as RGB");
    }

    #[test]
    fn every_row_filter_decodes() {
        // A gradient forces Sub/Up/Paeth to win on different rows; the proof is simply
        // that whatever the encoder picked, the decoder returns the exact pixels.
        let pixels: Vec<[u8; 4]> = (0..64)
            .map(|at| [(at * 4) as u8, (at * 7 % 256) as u8, 255 - at as u8, 255])
            .collect();
        let original = image(8, 8, &pixels);
        assert_eq!(decode(&encode(&original)).expect("round trip"), original);
    }

    #[test]
    fn downsampling_averages_in_linear_light_not_in_srgb() {
        // A black/white checker: the honest average of the two is 0.5 in linear light,
        // which is sRGB ~188 — a plain byte average would say 127 and darken the page.
        let checker = image(
            2,
            2,
            &[
                [0, 0, 0, 255],
                [255, 255, 255, 255],
                [255, 255, 255, 255],
                [0, 0, 0, 255],
            ],
        );
        let shrunk =
            decode(&downsample(&encode(&checker), 2).expect("downsample")).expect("decode");
        assert_eq!((shrunk.width, shrunk.height), (1, 1));
        assert_eq!(shrunk.rgba[0], 188, "the average lives in linear light");
    }

    #[test]
    fn a_hairline_survives_the_shrink_as_gray_ink() {
        // A one-pixel black column in a 4x2 white image: shrunk by 2 it must remain
        // visibly darker than the paper, never rounded away to white.
        let pixels: Vec<[u8; 4]> = (0..8)
            .map(|at| {
                if at % 4 == 1 {
                    [0, 0, 0, 255]
                } else {
                    [255; 4]
                }
            })
            .collect();
        let shrunk = decode(&downsample(&encode(&image(4, 2, &pixels)), 2).expect("downsample"))
            .expect("decode");
        assert_eq!((shrunk.width, shrunk.height), (2, 1));
        assert!(
            shrunk.rgba[0] < 220,
            "the line kept ink: {}",
            shrunk.rgba[0]
        );
        assert_eq!(shrunk.rgba[4], 255, "the paper stayed paper");
    }

    #[test]
    fn a_factor_of_one_is_the_identity() {
        let png = encode(&image(1, 1, &[[9, 9, 9, 255]]));
        assert_eq!(downsample(&png, 1).expect("identity"), png);
        assert_eq!(downsample(&png, 0).expect("identity"), png);
    }

    #[test]
    fn edges_that_fall_short_of_the_factor_are_still_covered() {
        let original = image(3, 3, &[[100, 100, 100, 255]; 9]);
        let shrunk =
            decode(&downsample(&encode(&original), 2).expect("downsample")).expect("decode");
        assert_eq!((shrunk.width, shrunk.height), (2, 2));
        assert_eq!(
            &shrunk.rgba[..3],
            &[100, 100, 100],
            "a flat field stays flat"
        );
    }

    #[test]
    fn transparent_pixels_do_not_bleed_their_color_into_the_block() {
        // A transparent black pixel beside an opaque white one: the average must stay
        // white at half coverage, not gray.
        let mixed = image(2, 1, &[[0, 0, 0, 0], [255, 255, 255, 255]]);
        let shrunk = decode(&downsample(&encode(&mixed), 2).expect("downsample")).expect("decode");
        assert_eq!(&shrunk.rgba[..4], &[255, 255, 255, 128]);
    }

    #[test]
    fn hostile_lengths_are_refused_not_allocated() {
        // A chunk claiming bytes past the file's end.
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        png.extend_from_slice(&0xFFFF_FFFFu32.to_be_bytes());
        png.extend_from_slice(b"IHDR");
        assert!(decode(&png).is_err());

        // A header declaring an image bigger than this codec will materialize.
        let vast = {
            let huge = Image {
                width: 1,
                height: 1,
                rgba: vec![0, 0, 0, 255],
            };
            let mut png = encode(&huge);
            png[16..20].copy_from_slice(&100_000u32.to_be_bytes());
            png[20..24].copy_from_slice(&100_000u32.to_be_bytes());
            png
        };
        let refused = decode(&vast).expect_err("10 gigapixels is refused");
        assert!(refused.contains("larger"), "{refused}");
    }

    #[test]
    fn what_this_codec_does_not_speak_is_named_not_misread() {
        let sixteen_bit = {
            let mut png = encode(&image(1, 1, &[[0, 0, 0, 255]]));
            png[8 + 8 + 8] = 16;
            png
        };
        assert!(decode(&sixteen_bit).expect_err("depth").contains("8-bit"));
        assert!(decode(b"plainly not a png")
            .expect_err("magic")
            .contains("signature"));
    }
}
