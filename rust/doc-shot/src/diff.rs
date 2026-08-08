//! Compare two captures and say, in a sentence, what changed.
//!
//! This is what lets a visual check be *read* without spending an image: instead of
//! handing back a picture to look at, [`compare`] hands back a [`Verdict`] whose
//! `Display` is one line — `identical`, or `differs: N px in x,y wxh` naming the
//! bounding box of every changed pixel. A caller holding a golden PNG asks "did the
//! render move?" and gets a sentence, not a file.
//!
//! Two rasterizations of the same page are never byte-equal at the edges — antialiasing
//! lands a glyph's gray fringe a shade differently from run to run — so a pixel only
//! counts as changed when some channel moves by more than [`CHANNEL_TOLERANCE`].
//! `identical` therefore means "within antialiasing slack", which is the honest reading
//! of "nothing changed" for a screenshot. Images of different sizes are a stated verdict,
//! not an error: a board that grew *is* the difference the caller asked about.

use std::fmt;

use crate::png::{self, Image};

/// How far one channel may move (out of 255) before its pixel counts as changed.
///
/// Re-rasterizing an antialiased edge shifts its fringe pixels by a shade or two; 3 is
/// enough slack to absorb that while a real edit — ink where paper was, a moved rule —
/// still lands whole channels away and is reported.
pub const CHANNEL_TOLERANCE: u8 = 3;

/// What comparing two images concluded. Its `Display` is the one-line text verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Every pixel is within [`CHANNEL_TOLERANCE`] of its counterpart.
    Identical,
    /// Some pixels changed; the region is their bounding box.
    Differs {
        /// How many pixels moved past the tolerance.
        pixels: u64,
        /// Left edge of the changed region, in pixels from the left.
        x: u32,
        /// Top edge of the changed region, in pixels from the top.
        y: u32,
        /// Width of the changed region, at least 1.
        width: u32,
        /// Height of the changed region, at least 1.
        height: u32,
    },
    /// The images are different sizes, so no pixel comparison is meaningful.
    DimensionMismatch {
        /// Width and height of the actual image.
        actual: (u32, u32),
        /// Width and height of the golden image.
        golden: (u32, u32),
    },
}

impl fmt::Display for Verdict {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Identical => write!(out, "identical"),
            Self::Differs {
                pixels,
                x,
                y,
                width,
                height,
            } => write!(out, "differs: {pixels} px in {x},{y} {width}x{height}"),
            Self::DimensionMismatch {
                actual: (aw, ah),
                golden: (gw, gh),
            } => write!(out, "differs: dimensions {aw}x{ah} vs {gw}x{gh}"),
        }
    }
}

/// Compare a captured image against a golden one, pixel by pixel.
///
/// A pixel counts as changed when any RGBA channel differs by more than
/// [`CHANNEL_TOLERANCE`]; the verdict reports how many changed and the bounding box
/// that holds them all. Different dimensions are the [`Verdict::DimensionMismatch`]
/// verdict, never an error. Complexity: `O(width × height)`.
#[must_use]
pub fn compare(actual: &Image, golden: &Image) -> Verdict {
    if (actual.width, actual.height) != (golden.width, golden.height) {
        return Verdict::DimensionMismatch {
            actual: (actual.width, actual.height),
            golden: (golden.width, golden.height),
        };
    }

    let mut pixels = 0u64;
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (u32::MAX, u32::MAX, 0u32, 0u32);
    let width = actual.width.max(1);
    for (index, (ours, theirs)) in actual
        .rgba
        .chunks_exact(4)
        .zip(golden.rgba.chunks_exact(4))
        .enumerate()
    {
        let changed = ours
            .iter()
            .zip(theirs)
            .any(|(a, b)| a.abs_diff(*b) > CHANNEL_TOLERANCE);
        if !changed {
            continue;
        }
        pixels += 1;
        let x = (index as u64 % u64::from(width)) as u32;
        let y = (index as u64 / u64::from(width)) as u32;
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }

    if pixels == 0 {
        return Verdict::Identical;
    }
    Verdict::Differs {
        pixels,
        x: min_x,
        y: min_y,
        width: max_x - min_x + 1,
        height: max_y - min_y + 1,
    }
}

/// [`compare`] over encoded PNG bytes, decoded through the crate's own bounded codec.
///
/// # Errors
/// A sentence when either PNG cannot be decoded, naming which one — the comparison
/// never guesses at bytes it could not read.
pub fn compare_png(actual: &[u8], golden: &[u8]) -> Result<Verdict, String> {
    let actual = png::decode(actual).map_err(|reason| format!("the capture: {reason}"))?;
    let golden = png::decode(golden).map_err(|reason| format!("the golden image: {reason}"))?;
    Ok(compare(&actual, &golden))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(width: u32, height: u32, pixel: [u8; 4]) -> Image {
        Image {
            width,
            height,
            rgba: pixel
                .iter()
                .copied()
                .cycle()
                .take(width as usize * height as usize * 4)
                .collect(),
        }
    }

    #[test]
    fn identical_images_read_identical() {
        let paper = flat(4, 3, [255, 255, 255, 255]);
        let verdict = compare(&paper, &paper.clone());
        assert_eq!(verdict, Verdict::Identical);
        assert_eq!(verdict.to_string(), "identical");
    }

    #[test]
    fn one_changed_pixel_is_named_at_its_own_coordinates() {
        let golden = flat(4, 3, [255, 255, 255, 255]);
        let mut actual = golden.clone();
        // Pixel (2, 1): whole channels away, a real mark, not antialiasing drift.
        let at = (4 + 2) * 4; // row 1 of width 4, column 2
        actual.rgba[at..at + 3].copy_from_slice(&[0, 0, 0]);
        let verdict = compare(&actual, &golden);
        assert_eq!(
            verdict,
            Verdict::Differs {
                pixels: 1,
                x: 2,
                y: 1,
                width: 1,
                height: 1,
            }
        );
        assert_eq!(verdict.to_string(), "differs: 1 px in 2,1 1x1");
    }

    #[test]
    fn the_region_is_the_bounding_box_of_every_changed_pixel() {
        let golden = flat(6, 5, [200, 200, 200, 255]);
        let mut actual = golden.clone();
        for (x, y) in [(1u32, 1u32), (4, 3)] {
            let at = ((y * 6 + x) * 4) as usize;
            actual.rgba[at] = 0;
        }
        assert_eq!(
            compare(&actual, &golden),
            Verdict::Differs {
                pixels: 2,
                x: 1,
                y: 1,
                width: 4,
                height: 3,
            }
        );
    }

    #[test]
    fn different_dimensions_are_a_stated_verdict_not_an_error() {
        let verdict = compare(&flat(4, 3, [255; 4]), &flat(2, 2, [255; 4]));
        assert_eq!(
            verdict,
            Verdict::DimensionMismatch {
                actual: (4, 3),
                golden: (2, 2),
            }
        );
        assert_eq!(verdict.to_string(), "differs: dimensions 4x3 vs 2x2");
    }

    #[test]
    fn drift_at_the_tolerance_is_identical_and_one_shade_past_it_is_not() {
        let golden = flat(2, 2, [100, 100, 100, 255]);
        let within = flat(2, 2, [100 + CHANNEL_TOLERANCE, 100, 100, 255]);
        assert_eq!(compare(&within, &golden), Verdict::Identical);

        let past = flat(2, 2, [100 + CHANNEL_TOLERANCE + 1, 100, 100, 255]);
        assert!(matches!(
            compare(&past, &golden),
            Verdict::Differs { pixels: 4, .. }
        ));
    }

    #[test]
    fn an_alpha_change_counts_like_any_other_channel() {
        let golden = flat(1, 1, [10, 10, 10, 255]);
        let faded = flat(1, 1, [10, 10, 10, 100]);
        assert!(matches!(
            compare(&faded, &golden),
            Verdict::Differs { pixels: 1, .. }
        ));
    }

    #[test]
    fn encoded_pngs_compare_through_the_crates_own_codec() {
        let golden = flat(3, 3, [255, 255, 255, 255]);
        let mut actual = golden.clone();
        actual.rgba[0] = 0;
        let verdict =
            compare_png(&png::encode(&actual), &png::encode(&golden)).expect("both decode");
        assert_eq!(verdict.to_string(), "differs: 1 px in 0,0 1x1");

        let refused =
            compare_png(b"plainly not a png", &png::encode(&golden)).expect_err("bad bytes");
        assert!(refused.contains("the capture"), "{refused}");
    }
}
