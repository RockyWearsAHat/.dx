//! Rasterize `packaging/icon.svg` into the PNG sizes browsers and stores ask for, and into
//! the macOS `.icns` that `DX.app` and every `.dx` file wear.
//!
//! The reasoning — why this rasterizes rather than driving a headless browser, why the icon
//! is not optional, why the render is supersampled and checked for ink — is recorded once,
//! in `packaging/packaging.dx#icon-source-and-build`; this file is the mechanism. It reuses
//! [`doc_shot::png`], the same encoder a document capture writes through, so the icon never
//! needs a second PNG writer to agree with the first.
//!
//! ```text
//! cargo run --release -p doc-shot --bin build-icons
//!   -> editor/github/icons/dx-{16,32,48,128}.png
//! cargo run --release -p doc-shot --bin build-icons -- --icns <path>
//!   -> <path>, the .icns DX.app and every .dx file wear
//! ```
//!
//! A release tool, not part of the shipped `dx` binary — a document renderer has no
//! business carrying a rasterizer around on a reader's machine.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use doc_shot::png::{encode, Image};

/// One filled rectangle from the source SVG, in document order — later rectangles are
/// painted over earlier ones, exactly as the SVG says.
#[derive(Debug)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    color: [u8; 3],
}

/// Sizes the browser extensions and the Chrome/Firefox stores ask for.
const SIZES: [u32; 4] = [16, 32, 48, 128];

/// What an `.icns` holds, as (pixels, iconset name). Finder picks from these for the app,
/// for every `.dx` file on disk, and for the icon that flies out of the Dock — so the
/// large sizes are not decoration, they are the ones a person actually looks at.
const ICNS_SIZES: [(u32, &str); 10] = [
    (16, "icon_16x16"),
    (32, "icon_16x16@2x"),
    (32, "icon_32x32"),
    (64, "icon_32x32@2x"),
    (128, "icon_128x128"),
    (256, "icon_128x128@2x"),
    (256, "icon_256x256"),
    (512, "icon_256x256@2x"),
    (512, "icon_512x512"),
    (1024, "icon_512x512@2x"),
];

/// Each pixel is sampled this many times per axis, then averaged, at sizes up to 128 —
/// where a rectangle edge lands on a fractional pixel and would otherwise alternate
/// between hard-on and hard-off. Dropped above 128: the source is a 128-unit square, so
/// every larger size is an exact integer scale and every edge already lands on a pixel
/// boundary.
const SUPERSAMPLE: u32 = 4;

fn main() {
    if let Err(message) = run() {
        eprintln!("{message}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let root = repo_root();
    let svg = fs::read_to_string(root.join("packaging/icon.svg"))
        .map_err(|error| format!("could not read packaging/icon.svg: {error}"))?;
    let (side, rects) = parse(&svg)?;

    match env::args().skip(1).collect::<Vec<_>>().as_slice() {
        [] => {
            let out = root.join("editor/github/icons");
            fs::create_dir_all(&out).map_err(|error| error.to_string())?;
            for &size in &SIZES {
                let path = out.join(format!("dx-{size}.png"));
                let opaque = write(side, &rects, size, &path)?;
                let bytes = fs::metadata(&path)
                    .map_err(|error| error.to_string())?
                    .len();
                println!("  {size:3}px  {bytes:6} bytes  {opaque} opaque pixels");
            }
            println!("wrote {}", out.display());
            Ok(())
        }
        [flag, target] if flag == "--icns" => icns(side, &rects, Path::new(target)),
        [flag] if flag == "--icns" => Err("--icns needs a file to write".to_string()),
        other => Err(format!("unrecognized arguments: {}", other.join(" "))),
    }
}

/// The repository root, two directories above this crate's own manifest — the same way
/// the script this replaces located itself relative to its own file.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("rust/doc-shot sits two directories under the repository root")
        .to_path_buf()
}

/// The SVG's `viewBox` side length and every filled rectangle it declares, in order.
///
/// Anything this cannot draw is an error naming what it found: a shape left unrendered
/// with no complaint is exactly the defect `packaging/packaging.dx` describes this
/// refusing.
fn parse(svg: &str) -> Result<(f64, Vec<Rect>), String> {
    let stripped = strip_comments(svg);
    let side = view_box_side(&stripped)?;

    let declared = count_tag(&stripped, "rect");
    let rects = find_rects(&stripped);
    let other_shapes: usize = ["path", "circle", "ellipse", "polygon", "line", "text", "g"]
        .iter()
        .map(|tag| count_tag(&stripped, tag))
        .sum();

    if rects.len() != declared || other_shapes > 0 {
        return Err(format!(
            "icon.svg has shapes this renderer does not draw ({declared} rects declared, \
             {} understood, {other_shapes} other shapes). Keep the icon to filled <rect> \
             elements, or teach build-icons.rs the rest.",
            rects.len()
        ));
    }
    Ok((side, rects))
}

/// The source with every `<!-- … -->` removed — prose naming a `<rect>` is not a shape,
/// and counting it as one is how this once refused to build its own icon.
fn strip_comments(svg: &str) -> String {
    let mut out = String::with_capacity(svg.len());
    let mut rest = svg;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        rest = match rest[start..].find("-->") {
            Some(end) => &rest[start + end + 3..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}

fn view_box_side(svg: &str) -> Result<f64, String> {
    let marker = "viewBox=\"0 0 ";
    let start = svg.find(marker).ok_or("icon.svg has no viewBox")? + marker.len();
    let rest = &svg[start..];
    let end = rest.find('"').ok_or("icon.svg's viewBox is not closed")?;
    let mut sides = rest[..end].split_whitespace();
    let width = next_number(&mut sides, "viewBox width")?;
    let height = next_number(&mut sides, "viewBox height")?;
    if width != height {
        return Err("icon.svg must be square".to_string());
    }
    Ok(width)
}

fn next_number<'a>(values: &mut impl Iterator<Item = &'a str>, what: &str) -> Result<f64, String> {
    values
        .next()
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| format!("icon.svg's {what} is not a number"))
}

/// How many times `<name` opens a tag — counting the declaration, not whether this
/// renderer went on to understand it.
fn count_tag(svg: &str, name: &str) -> usize {
    let needle = format!("<{name}");
    svg.match_indices(&needle)
        .filter(|(index, _)| {
            svg[index + needle.len()..]
                .chars()
                .next()
                .is_none_or(|next| !next.is_alphanumeric())
        })
        .count()
}

/// Every well-formed `<rect x="…" y="…" width="…" height="…" fill="#RRGGBB"/>`, in
/// document order. A rectangle whose attributes do not parse is simply absent from the
/// result, which is what makes the declared/understood mismatch in [`parse`] the signal.
fn find_rects(svg: &str) -> Vec<Rect> {
    let mut rects = Vec::new();
    let mut rest = svg;
    while let Some(start) = rest.find("<rect") {
        let Some(end) = rest[start..].find("/>") else {
            break;
        };
        if let Some(rect) = parse_rect(&rest[start..start + end]) {
            rects.push(rect);
        }
        rest = &rest[start + end + 2..];
    }
    rects
}

fn parse_rect(tag: &str) -> Option<Rect> {
    Some(Rect {
        x: attr(tag, "x")?.parse().ok()?,
        y: attr(tag, "y")?.parse().ok()?,
        w: attr(tag, "width")?.parse().ok()?,
        h: attr(tag, "height")?.parse().ok()?,
        color: hex_color(attr(tag, "fill")?)?,
    })
}

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let marker = format!("{name}=\"");
    let start = tag.find(&marker)? + marker.len();
    let end = tag[start..].find('"')?;
    Some(&tag[start..start + end])
}

fn hex_color(value: &str) -> Option<[u8; 3]> {
    let hex = value.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    Some([
        u8::from_str_radix(&hex[0..2], 16).ok()?,
        u8::from_str_radix(&hex[2..4], 16).ok()?,
        u8::from_str_radix(&hex[4..6], 16).ok()?,
    ])
}

/// The icon at `size × size`, as raw RGBA — [`encode`] does its own filtering, so this
/// hands back pixels, never PNG scanlines.
fn render(side: f64, rects: &[Rect], size: u32) -> Vec<u8> {
    let samples = if size <= 128 { SUPERSAMPLE } else { 1 };
    let scale = f64::from(size * samples) / side;
    let span = size * samples;

    let mut canvas = vec![0u8; (span * span * 4) as usize];
    for rect in rects {
        let left = (rect.x * scale).round().max(0.0) as u32;
        let right = ((rect.x + rect.w) * scale).round().min(f64::from(span)) as u32;
        let top = (rect.y * scale).round().max(0.0) as u32;
        let bottom = ((rect.y + rect.h) * scale).round().min(f64::from(span)) as u32;
        for row in top..bottom {
            for column in left..right {
                let base = ((row * span + column) * 4) as usize;
                canvas[base] = rect.color[0];
                canvas[base + 1] = rect.color[1];
                canvas[base + 2] = rect.color[2];
                canvas[base + 3] = 255;
            }
        }
    }

    if samples == 1 {
        return canvas;
    }

    // Average each samples×samples block back down to one pixel, alpha included so an
    // edge fades rather than jags.
    let mut out = vec![0u8; (size * size * 4) as usize];
    for row in 0..size {
        for column in 0..size {
            let mut totals = [0u32; 4];
            for dy in 0..samples {
                let source_row = row * samples + dy;
                for dx in 0..samples {
                    let source_column = column * samples + dx;
                    let base = ((source_row * span + source_column) * 4) as usize;
                    for (channel, total) in totals.iter_mut().enumerate() {
                        *total += u32::from(canvas[base + channel]);
                    }
                }
            }
            let count = samples * samples;
            let base = ((row * size + column) * 4) as usize;
            for (channel, total) in totals.iter().enumerate() {
                out[base + channel] = (total / count) as u8;
            }
        }
    }
    out
}

/// Draw the icon at `size` into `path` and return how many pixels carry ink — the check
/// that matters, and the one a byte count cannot make. A blank render is the exact defect
/// `packaging/packaging.dx#icon-source-and-build` records headless Chrome producing.
fn write(side: f64, rects: &[Rect], size: u32, path: &Path) -> Result<usize, String> {
    let rgba = render(side, rects, size);
    let opaque = rgba
        .iter()
        .skip(3)
        .step_by(4)
        .filter(|&&alpha| alpha > 0)
        .count();
    if opaque < (size * size / 4) as usize {
        return Err(format!(
            "the {size}px icon came out (nearly) empty — {opaque} opaque pixels"
        ));
    }
    let bytes = encode(&Image {
        width: size,
        height: size,
        rgba,
    });
    fs::write(path, &bytes)
        .map_err(|error| format!("could not write {}: {error}", path.display()))?;
    Ok(opaque)
}

/// Write the macOS icon file `DX.app` and every `.dx` document wear.
///
/// `iconutil` is the only thing that writes the format, and it takes a directory of
/// conventionally named PNGs — so the directory is a scratch one, removed once the
/// `.icns` is written or the conversion fails.
fn icns(side: f64, rects: &[Rect], target: &Path) -> Result<(), String> {
    let work = env::temp_dir().join(format!("dx-iconset-{}", std::process::id()));
    let iconset = work.join("dx.iconset");
    fs::create_dir_all(&iconset).map_err(|error| error.to_string())?;

    let result = (|| -> Result<(), String> {
        for &(size, name) in &ICNS_SIZES {
            write(side, rects, size, &iconset.join(format!("{name}.png")))?;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let status = Command::new("iconutil")
            .args(["--convert", "icns", "--output"])
            .arg(target)
            .arg(&iconset)
            .status()
            .map_err(|error| format!("could not run iconutil: {error}"))?;
        if !status.success() {
            return Err("iconutil failed".to_string());
        }
        Ok(())
    })();

    let _ = fs::remove_dir_all(&work);
    result?;

    let bytes = fs::metadata(target)
        .map_err(|error| error.to_string())?
        .len();
    println!("  icns   {bytes:6} bytes  {}", target.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The repository's real icon — parsing it is what would have caught the shape this
    /// renderer refused to draw once already.
    const ICON_SVG: &str = include_str!("../../../../packaging/icon.svg");

    #[test]
    fn a_rect_named_only_in_prose_is_not_counted_as_a_shape() {
        let svg = r##"<svg viewBox="0 0 10 10">
            <!-- a <rect> mentioned here should not count -->
            <rect x="0" y="0" width="10" height="10" fill="#000000"/>
        </svg>"##;
        let (side, rects) = parse(svg).expect("comment-only mention is not a shape");
        assert_eq!(side, 10.0);
        assert_eq!(rects.len(), 1);
    }

    #[test]
    fn a_shape_this_renderer_cannot_draw_fails_loudly() {
        let svg = r##"<svg viewBox="0 0 10 10">
            <rect x="0" y="0" width="10" height="10" fill="#000000"/>
            <circle cx="5" cy="5" r="5"/>
        </svg>"##;
        let error = parse(svg).expect_err("a circle is not a rect");
        assert!(error.contains("1 other shapes"), "{error}");
    }

    #[test]
    fn a_non_square_view_box_is_refused() {
        let svg = r#"<svg viewBox="0 0 10 20"></svg>"#;
        let error = parse(svg).expect_err("10x20 is not square");
        assert!(error.contains("square"), "{error}");
    }

    #[test]
    fn rects_come_back_in_document_order_with_their_colors() {
        let svg = r##"<svg viewBox="0 0 10 10">
            <rect x="1" y="2" width="3" height="4" fill="#ff0000"/>
            <rect x="5" y="6" width="7" height="8" fill="#00ff00"/>
        </svg>"##;
        let (_, rects) = parse(svg).expect("two plain rects");
        assert_eq!(rects.len(), 2);
        assert_eq!((rects[0].x, rects[0].y), (1.0, 2.0));
        assert_eq!(rects[0].color, [0xff, 0x00, 0x00]);
        assert_eq!((rects[1].x, rects[1].y), (5.0, 6.0));
        assert_eq!(rects[1].color, [0x00, 0xff, 0x00]);
    }

    #[test]
    fn a_blank_render_is_refused_not_shipped() {
        // No rects at all: every pixel stays transparent, which is exactly the defect
        // this check exists to catch before it reaches a browser's toolbar.
        let dir = std::env::temp_dir().join(format!("dx-icon-test-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("scratch dir");
        let path = dir.join("blank.png");
        let error = write(10.0, &[], 16, &path).expect_err("a blank icon is refused");
        assert!(error.contains("empty"), "{error}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_committed_icon_parses_and_renders_with_ink_at_every_shipped_size() {
        let (side, rects) = parse(ICON_SVG).expect("the repository's own icon.svg");
        assert_eq!(side, 128.0);
        assert!(!rects.is_empty());
        for &size in &SIZES {
            let rgba = render(side, &rects, size);
            let opaque = rgba.iter().skip(3).step_by(4).filter(|&&a| a > 0).count();
            assert!(
                opaque >= (size * size / 4) as usize,
                "{size}px came out nearly blank: {opaque} opaque pixels"
            );
        }
    }
}
