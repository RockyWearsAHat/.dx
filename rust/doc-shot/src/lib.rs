//! `doc-shot` — render a `.dx` document to a picture.
//!
//! This is what lets an agent *look* at a document instead of only reading it. The same
//! HTML a person sees in their editor is loaded in a headless browser and captured as a
//! PNG, so a chart, a table, or a diagram arrives as an image rather than as a description
//! of an image.
//!
//! # How the full page fits in the frame
//! A headless browser screenshot is only as tall as its window, so capture runs twice.
//! The first pass loads the page with a one-line measuring script and reads the real
//! content height back out of the DOM; the second pass opens a window exactly that tall
//! and takes the picture. The result is the whole document, never a cropped viewport.
//!
//! The measuring script exists only in the throwaway capture copy. The stored document and
//! everything [`doc_core::render`] produces stay script-free.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::all)]

pub mod base64;
pub mod browser;
pub mod cdp;
pub mod play;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use doc_core::model::Document;
use doc_core::render::{html, HtmlOptions, Theme};

/// Default capture width in CSS pixels.
pub const DEFAULT_WIDTH: u32 = 1200;

/// Default height of one page, in CSS pixels — roughly a sheet of paper at
/// [`DEFAULT_WIDTH`], so a paginated capture reads like the printed document.
pub const DEFAULT_PAGE_HEIGHT: u32 = 1550;

/// Most pages one paginated capture will produce.
///
/// Each page costs a browser launch, and a reader who needs the 30th page of something
/// wants a section, not a flip-book. [`capture_pages`] reports the pages it did not take
/// rather than pretending the document ended.
pub const MAX_PAGES: usize = 12;

/// Most pixels one image may carry before a vision model's ingestion scales it down
/// (~1.15 megapixels). [`ShotOptions::for_reading`] sizes pages under this, so every
/// pixel captured is a pixel the model actually sees — capturing larger only produces
/// an image something else shrinks in transit.
pub const VISION_MAX_PIXELS: u32 = 1_150_000;

/// Longest edge a vision model accepts without scaling, in pixels.
pub const VISION_MAX_EDGE: u32 = 1568;

/// Default capture width for a read by a vision model, in CSS pixels: the rendered
/// content column (46rem at 17px ≈ 782px) plus its margins. Capturing wider spends the
/// fixed pixel budget of [`VISION_MAX_PIXELS`] on empty margin instead of on glyphs.
pub const READ_WIDTH: u32 = 860;

/// Narrowest capture that still lays the page out as a page.
const MIN_WIDTH: u32 = 320;

/// Highest device scale factor a capture may ask for.
const MAX_SCALE: u32 = 4;

/// Window height used for the measuring pass, before the real height is known.
const MEASURE_HEIGHT: u32 = 900;

/// Shortest page captured, so a one-line document still produces a legible image.
const MIN_HEIGHT: u32 = 200;

/// Tallest page captured, so a runaway document cannot produce a gigantic image.
const MAX_HEIGHT: u32 = 12_000;

/// Attribute the measuring script writes the content height into.
const HEIGHT_ATTRIBUTE: &str = "data-dx-height";

/// Attribute the measuring script writes each block's box into, as
/// `id:top:height;id:top:height…`. This is what lets a page break fall between blocks
/// instead of through the middle of a line.
const BLOCKS_ATTRIBUTE: &str = "data-dx-blocks";

/// How to capture a document.
#[derive(Debug, Clone)]
pub struct ShotOptions {
    /// Capture width in CSS pixels.
    pub width: u32,
    /// Palette to render with.
    pub theme: Theme,
    /// Directory for the temporary HTML the browser loads.
    pub scratch_dir: PathBuf,
    /// Height of one page for [`capture_pages`], in CSS pixels.
    pub page_height: u32,
    /// Most pages [`capture_pages`] will take before it stops and says so.
    pub max_pages: usize,
    /// Image pixels per CSS pixel, clamped to 1–4. 2 makes an export match a
    /// high-density screen; 1 is right for a vision model, which would only scale the
    /// extra pixels back out (see [`VISION_MAX_PIXELS`]).
    pub scale: u32,
}

impl Default for ShotOptions {
    fn default() -> Self {
        Self {
            width: DEFAULT_WIDTH,
            theme: Theme::Auto,
            scratch_dir: std::env::temp_dir(),
            page_height: DEFAULT_PAGE_HEIGHT,
            max_pages: MAX_PAGES,
            scale: 1,
        }
    }
}

impl ShotOptions {
    /// Options for a read by a vision model: pages that arrive exactly as captured.
    ///
    /// Each page stays within [`VISION_MAX_PIXELS`] and [`VISION_MAX_EDGE`], the two
    /// limits past which ingestion downscales an image, so zero compaction happens
    /// between the browser and the model. Pages still break between blocks — a narrower,
    /// shorter page means more pages, never a sliced line. `width` overrides
    /// [`READ_WIDTH`]; the page height is derived so the pixel budget holds.
    pub fn for_reading(width: Option<u32>) -> Self {
        let width = width
            .unwrap_or(READ_WIDTH)
            .clamp(MIN_WIDTH, VISION_MAX_EDGE);
        Self {
            width,
            page_height: (VISION_MAX_PIXELS / width).min(VISION_MAX_EDGE),
            scale: 1,
            ..Self::default()
        }
    }
}

/// A captured image.
#[derive(Debug, Clone)]
pub struct Shot {
    /// Raw PNG bytes.
    pub png: Vec<u8>,
    /// Captured width in pixels.
    pub width: u32,
    /// Captured height in pixels.
    pub height: u32,
}

/// One page of a paginated capture.
#[derive(Debug, Clone)]
pub struct Page {
    /// The image of this page.
    pub shot: Shot,
    /// 1-based page number.
    pub number: usize,
    /// How many pages the document has in total, including any not captured.
    pub total: usize,
    /// Ids of the blocks that begin on this page, in order — the handles a reader uses to
    /// ask for one part of the document instead of the next picture.
    pub blocks: Vec<String>,
}

/// Where one page starts and how tall it is, in CSS pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PageRange {
    /// Distance from the top of the document to the top of this page.
    top: u32,
    /// Height of this page.
    height: u32,
    /// Ids of the blocks beginning inside it.
    blocks: Vec<String>,
}

/// One block's box, as measured in the browser.
#[derive(Debug, Clone, PartialEq, Eq)]
struct BlockBox {
    /// The block's `id`.
    id: String,
    /// Distance from the top of the document to the top of the block.
    top: u32,
    /// The block's rendered height.
    height: u32,
}

/// The page every capture in this crate photographs — and every [`play`] session drives.
///
/// Code blocks are rendered **open**. On screen a block starts folded behind its label, which
/// a reader opens when they want the recipe; a picture cannot be opened, so folding one here
/// would photograph the label and throw the listing away — including in the images `dx_read`
/// hands an agent, whose whole way of reading a document is to look at it.
pub(crate) fn page_html(document: &Document, theme: Theme) -> String {
    html(
        document,
        &HtmlOptions {
            theme,
            collapse_code: false,
            ..HtmlOptions::default()
        },
    )
}

/// Render `document` and capture it as a PNG.
///
/// Returns a message naming what to install when no browser is available, so the caller
/// can fall back to text rather than failing outright.
pub fn capture(document: &Document, options: &ShotOptions) -> Result<Shot, String> {
    let browser = browser::find().ok_or_else(browser::missing_message)?;
    let page = page_html(document, options.theme);

    let workspace = scratch_workspace(&options.scratch_dir)?;
    let result = capture_page(&browser, &page, options, &workspace);
    let _ = std::fs::remove_dir_all(&workspace);
    result
}

/// Render `document` and capture it as a sequence of page images.
///
/// This is what an agent gets when it reads a document: the pages of the real rendered
/// page, in order, each one an image it can look at. Breaks fall **between blocks**, so a
/// sentence, a table, or a chart is never sliced across two pictures; a single block taller
/// than a page is the one case that must be cut, and it is cut at page height.
///
/// [`Page::total`] counts the document's real pages even when `options.max_pages` stopped
/// the capture early, so a caller can say what it did not show instead of implying the
/// document ended.
///
/// Complexity: one browser launch to measure, then one per captured page.
pub fn capture_pages(document: &Document, options: &ShotOptions) -> Result<Vec<Page>, String> {
    let browser = browser::find().ok_or_else(browser::missing_message)?;
    let page = page_html(document, options.theme);

    let headings: Vec<String> = document
        .blocks
        .iter()
        .filter(|block| block.kind == "heading")
        .map(|block| block.id.clone())
        .collect();

    let workspace = scratch_workspace(&options.scratch_dir)?;
    let result = capture_all_pages(&browser, &page, &headings, options, &workspace);
    let _ = std::fs::remove_dir_all(&workspace);
    result
}

/// Measure once, then capture each page range inside an already-created `workspace`.
fn capture_all_pages(
    browser: &Path,
    page: &str,
    headings: &[String],
    options: &ShotOptions,
    workspace: &Path,
) -> Result<Vec<Page>, String> {
    let measure_file = workspace.join("measure.html");
    write(&measure_file, &with_measuring_script(page))?;
    let dom = dump_dom(browser, &measure_file, options.width);
    let total_height = dom
        .as_deref()
        .and_then(read_height_attribute)
        .unwrap_or(MEASURE_HEIGHT);
    let boxes = dom.as_deref().map(read_block_boxes).unwrap_or_default();

    let ranges = page_ranges(&boxes, total_height, page_height(options), headings);
    let total = ranges.len();

    let page_file = workspace.join("page.html");
    let mut pages = Vec::new();
    for (index, range) in ranges.iter().take(options.max_pages).enumerate() {
        write(&page_file, &shifted_to(page, range.top))?;
        let shot = screenshot(
            browser,
            &page_file,
            &workspace.join(format!("page-{index}.png")),
            options.width,
            range.height,
            scale(options),
        )?;
        pages.push(Page {
            shot,
            number: index + 1,
            total,
            blocks: range.blocks.clone(),
        });
    }

    Ok(pages)
}

/// The page height to use, never zero (which would divide a document into infinite pages).
fn page_height(options: &ShotOptions) -> u32 {
    options.page_height.clamp(MIN_HEIGHT, MAX_HEIGHT)
}

/// Divide a document of `total_height` into pages of at most `height`, breaking between
/// blocks.
///
/// The rule is: fill a page with whole blocks; when the next block would overflow, the page
/// ends where that block begins. A block taller than a whole page cannot be kept intact, so
/// it is split at page height and its id is reported on each piece — the reader still sees
/// every pixel of it, and still knows what they are looking at.
///
/// Ids in `sticky` are blocks that must not be the last thing on a page — headings. A
/// heading stranded at the foot of one page while the text it titles begins on the next
/// reads as a mistake, so the break moves up to take the heading with it.
///
/// An unmeasured document (no boxes) falls back to fixed-height pages, which is the same
/// division a naive slicer would make.
///
/// Complexity: `O(n)` in the number of blocks — each block is placed once, and the walk
/// back over trailing headings only ever undoes placements from the page being closed.
fn page_ranges(
    boxes: &[BlockBox],
    total_height: u32,
    height: u32,
    sticky: &[String],
) -> Vec<PageRange> {
    let total_height = total_height.max(1);
    if boxes.is_empty() {
        return fixed_ranges(total_height, height);
    }

    let ids = |range: std::ops::Range<usize>| -> Vec<String> {
        boxes[range].iter().map(|entry| entry.id.clone()).collect()
    };

    let mut ranges: Vec<PageRange> = Vec::new();
    let mut top = 0;
    let mut page_start = 0;
    let mut index = 0;

    while index < boxes.len() {
        let entry = &boxes[index];
        let bottom = entry.top.saturating_add(entry.height);
        if bottom <= top + height {
            index += 1;
            continue;
        }

        // The block does not fit. Close the page before it — unless it is alone on the
        // page, in which case it is taller than any page and has to be cut.
        if index > page_start {
            let mut cut = index;
            while cut > page_start + 1 && sticky.iter().any(|id| *id == boxes[cut - 1].id) {
                cut -= 1;
            }
            let end = boxes[cut].top.max(top + 1);
            ranges.push(PageRange {
                top,
                height: end - top,
                blocks: ids(page_start..cut),
            });
            top = end;
            page_start = cut;
            continue;
        }

        while top + height < bottom {
            ranges.push(PageRange {
                top,
                height,
                blocks: vec![entry.id.clone()],
            });
            top += height;
        }
        index += 1;
    }

    let tail = total_height.max(top + 1);
    ranges.push(PageRange {
        top,
        height: tail - top,
        blocks: ids(page_start..boxes.len()),
    });
    ranges
}

/// Divide a height into equal pages, used when no block boxes were measured.
fn fixed_ranges(total_height: u32, height: u32) -> Vec<PageRange> {
    let mut ranges = Vec::new();
    let mut top = 0;
    while top < total_height {
        ranges.push(PageRange {
            top,
            height: height.min(total_height - top),
            blocks: Vec::new(),
        });
        top += height;
    }
    if ranges.is_empty() {
        ranges.push(PageRange {
            top: 0,
            height: total_height,
            blocks: Vec::new(),
        });
    }
    ranges
}

/// Run both browser passes against `page` inside an already-created `workspace`.
fn capture_page(
    browser: &Path,
    page: &str,
    options: &ShotOptions,
    workspace: &Path,
) -> Result<Shot, String> {
    let measure_file = workspace.join("measure.html");
    write(&measure_file, &with_measuring_script(page))?;
    let height = dump_dom(browser, &measure_file, options.width)
        .as_deref()
        .and_then(read_height_attribute)
        .unwrap_or(MEASURE_HEIGHT);

    let page_file = workspace.join("page.html");
    write(&page_file, page)?;
    screenshot(
        browser,
        &page_file,
        &workspace.join("shot.png"),
        options.width,
        height,
        scale(options),
    )
}

/// The device scale factor to capture at, never zero and never absurd.
fn scale(options: &ShotOptions) -> u32 {
    options.scale.clamp(1, MAX_SCALE)
}

/// Capture one window of `page_file` as a PNG, clamped to a sane height.
///
/// `--window-size` is CSS pixels and the image comes out `scale` times larger on each
/// axis, so the layout is identical at every scale — only the pixel density changes.
/// [`Shot`] reports the image's real pixel dimensions.
fn screenshot(
    browser: &Path,
    page_file: &Path,
    image_file: &Path,
    width: u32,
    height: u32,
    scale: u32,
) -> Result<Shot, String> {
    let height = height.clamp(MIN_HEIGHT, MAX_HEIGHT);
    let status = browser_command(browser)
        .arg(format!("--force-device-scale-factor={scale}"))
        .arg(format!("--screenshot={}", image_file.display()))
        .arg(format!("--window-size={width},{height}"))
        .arg(file_url(page_file))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| format!("could not start the browser: {error}"))?;

    let png = std::fs::read(image_file).map_err(|error| {
        format!(
            "the browser did not produce an image ({status}): {error}. \
             Try setting {} to a different browser.",
            browser::BROWSER_ENV
        )
    })?;

    Ok(Shot {
        png,
        width: width * scale,
        height: height * scale,
    })
}

/// Load the measuring copy and return the rendered DOM the browser dumped.
///
/// Measuring always runs at scale 1: block boxes and the page height are CSS pixels,
/// and the capture pass applies its own density on top of them.
fn dump_dom(browser: &Path, page_file: &Path, width: u32) -> Option<String> {
    let output = browser_command(browser)
        .arg("--force-device-scale-factor=1")
        .arg("--dump-dom")
        .arg(format!("--window-size={width},{MEASURE_HEIGHT}"))
        .arg(file_url(page_file))
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// A copy of `page` scrolled down to `top`, so the browser's window frames that page.
///
/// The shift is a style in the throwaway capture copy, not in the document: nothing here
/// changes the stored bytes or what [`doc_core::render`] produces.
fn shifted_to(page: &str, top: u32) -> String {
    if top == 0 {
        return page.to_string();
    }
    let style =
        format!("<style>html{{overflow:hidden}}body{{position:relative;top:-{top}px}}</style>");
    match page.rfind("</head>") {
        Some(index) => format!("{}{style}{}", &page[..index], &page[index..]),
        None => format!("{style}{page}"),
    }
}

/// Extract the measured height from a dumped DOM.
fn read_height_attribute(dom: &str) -> Option<u32> {
    read_attribute(dom, HEIGHT_ATTRIBUTE)?.parse().ok()
}

/// Extract each block's measured box from a dumped DOM, in document order.
///
/// A malformed entry is skipped rather than failing the capture: a page that measured
/// imperfectly should still produce pictures, just with less precise breaks.
fn read_block_boxes(dom: &str) -> Vec<BlockBox> {
    let Some(raw) = read_attribute(dom, BLOCKS_ATTRIBUTE) else {
        return Vec::new();
    };
    raw.split(';')
        .filter(|entry| !entry.is_empty())
        .filter_map(|entry| {
            let mut fields = entry.rsplitn(3, ':');
            let height = fields.next()?.parse().ok()?;
            let top = fields.next()?.parse().ok()?;
            let id = fields.next()?.to_string();
            Some(BlockBox { id, top, height })
        })
        .collect()
}

/// Read one attribute value off the dumped `<html>` element.
fn read_attribute<'a>(dom: &'a str, name: &str) -> Option<&'a str> {
    let marker = format!("{name}=\"");
    let start = dom.find(&marker)? + marker.len();
    let end = start + dom[start..].find('"')?;
    Some(&dom[start..end])
}

/// A browser command preloaded with the flags every pass needs.
fn browser_command(browser: &Path) -> Command {
    let mut command = Command::new(browser);
    command.args([
        "--headless",
        "--disable-gpu",
        "--no-sandbox",
        "--hide-scrollbars",
        "--no-first-run",
        "--disable-extensions",
        "--virtual-time-budget=4000",
    ]);
    command
}

/// Add the one-line script that records the page's real height for the measuring pass.
///
/// The height comes from the `<body>` box, not from `documentElement.scrollHeight`: the
/// latter never reports less than the viewport, which would pad every short document with
/// a screenful of empty space.
fn with_measuring_script(page: &str) -> String {
    let script = format!(
        "<script>var b=document.body,r=document.documentElement;\
         r.setAttribute('{HEIGHT_ATTRIBUTE}', \
         String(Math.ceil(Math.max(b.scrollHeight, b.getBoundingClientRect().height))));\
         r.setAttribute('{BLOCKS_ATTRIBUTE}', \
         Array.prototype.map.call(document.querySelectorAll('[data-block-id]'), function(e){{\
         var x=e.getBoundingClientRect();\
         return e.getAttribute('data-block-id')+':'+Math.max(0,Math.round(x.top+window.scrollY))\
         +':'+Math.ceil(x.height);}}).join(';'));</script>"
    );
    match page.rfind("</body>") {
        Some(index) => format!("{}{script}{}", &page[..index], &page[index..]),
        None => format!("{page}{script}"),
    }
}

/// Create a unique scratch directory for one capture or play session.
pub(crate) fn scratch_workspace(root: &Path) -> Result<PathBuf, String> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    let workspace = root.join(format!("dx-shot-{stamp}-{}", std::process::id()));
    std::fs::create_dir_all(&workspace)
        .map_err(|error| format!("could not create {}: {error}", workspace.display()))?;
    Ok(workspace)
}

/// Write a file, reporting the path on failure.
pub(crate) fn write(path: &Path, contents: &str) -> Result<(), String> {
    std::fs::write(path, contents)
        .map_err(|error| format!("could not write {}: {error}", path.display()))
}

/// A `file://` URL for a local path, which is what the browser needs to load it.
pub(crate) fn file_url(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    if text.starts_with('/') {
        format!("file://{text}")
    } else {
        format!("file:///{text}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use doc_core::format::parse;

    #[test]
    fn reading_pages_fit_a_vision_model_without_downscaling() {
        for width in [
            None,
            Some(320),
            Some(860),
            Some(1200),
            Some(1568),
            Some(9000),
        ] {
            let options = ShotOptions::for_reading(width);
            assert!(
                options.width * options.page_height <= VISION_MAX_PIXELS,
                "a {}x{} page would be downscaled in transit",
                options.width,
                options.page_height
            );
            assert!(options.width.max(options.page_height) <= VISION_MAX_EDGE);
            assert_eq!(
                options.scale, 1,
                "extra density would only be scaled back out"
            );
        }
    }

    #[test]
    fn a_scaled_capture_reports_image_pixels_not_css_pixels() {
        let options = ShotOptions {
            scale: 2,
            ..ShotOptions::default()
        };
        assert_eq!(scale(&options), 2);
        let absurd = ShotOptions {
            scale: 99,
            ..ShotOptions::default()
        };
        assert_eq!(scale(&absurd), MAX_SCALE);
    }

    #[test]
    fn the_measuring_script_goes_inside_the_body() {
        let page = "<html><body><p>x</p></body></html>";
        let measured = with_measuring_script(page);
        assert!(measured.contains(HEIGHT_ATTRIBUTE));
        assert!(measured.ends_with("</body></html>"));
        assert!(measured.find("<script>").unwrap() < measured.find("</body>").unwrap());
    }

    fn boxes(entries: &[(&str, u32, u32)]) -> Vec<BlockBox> {
        entries
            .iter()
            .map(|(id, top, height)| BlockBox {
                id: (*id).to_string(),
                top: *top,
                height: *height,
            })
            .collect()
    }

    #[test]
    fn a_document_shorter_than_a_page_is_one_page() {
        let ranges = page_ranges(&boxes(&[("a", 0, 100), ("b", 100, 80)]), 180, 1000, &[]);
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].blocks, vec!["a", "b"]);
        assert_eq!(ranges[0].height, 180);
    }

    #[test]
    fn a_page_break_falls_between_blocks_never_through_one() {
        // Page height 200: "a" and "b" fit; "c" would end at 260, so the page ends where
        // "c" begins — at 160, not at the 200px mark, which sits inside "c".
        let ranges = page_ranges(
            &boxes(&[("a", 0, 40), ("b", 40, 120), ("c", 160, 100)]),
            260,
            200,
            &[],
        );
        assert_eq!(ranges.len(), 2);
        assert_eq!(ranges[0].blocks, vec!["a", "b"]);
        assert_eq!((ranges[0].top, ranges[0].height), (0, 160));
        assert_eq!(ranges[1].blocks, vec!["c"]);
        // Every page joins the next with no gap and no overlap: nothing is lost, nothing
        // is shown twice.
        for pair in ranges.windows(2) {
            assert_eq!(pair[0].top + pair[0].height, pair[1].top);
        }
        assert_eq!(
            ranges.last().expect("last").top + ranges.last().expect("last").height,
            260
        );
    }

    #[test]
    fn a_block_taller_than_a_page_is_split_and_still_named_on_each_piece() {
        // The one case a break cannot avoid. The reader must still see all of it, and
        // still know which block they are looking at.
        let ranges = page_ranges(&boxes(&[("wide", 0, 250)]), 250, 100, &[]);
        assert_eq!(ranges.len(), 3);
        for range in &ranges {
            assert_eq!(range.blocks, vec!["wide"]);
        }
        assert_eq!(ranges.iter().map(|r| r.height).sum::<u32>(), 250);
    }

    #[test]
    fn a_heading_is_never_stranded_at_the_foot_of_a_page() {
        // "h2" fits on page one, but the text it titles does not. Leaving the heading
        // behind would print a title with nothing under it, so the break moves up.
        let ranges = page_ranges(
            &boxes(&[
                ("h1", 0, 30),
                ("p1", 30, 100),
                ("h2", 130, 30),
                ("p2", 160, 100),
            ]),
            260,
            180,
            &["h1".to_string(), "h2".to_string()],
        );
        assert_eq!(ranges[0].blocks, vec!["h1", "p1"]);
        assert_eq!(ranges[1].blocks, vec!["h2", "p2"]);
        assert_eq!((ranges[0].top, ranges[0].height), (0, 130));
    }

    #[test]
    fn a_heading_alone_on_a_page_still_gets_printed() {
        // The rule must not eat the only block on the page and loop forever.
        let ranges = page_ranges(
            &boxes(&[("h", 0, 300), ("p", 300, 50)]),
            350,
            200,
            &["h".to_string()],
        );
        assert!(ranges
            .iter()
            .any(|range| range.blocks.contains(&"h".to_string())));
        assert_eq!(ranges.iter().map(|r| r.height).sum::<u32>(), 350);
    }

    #[test]
    fn an_unmeasured_document_still_paginates_by_height() {
        let ranges = page_ranges(&[], 250, 100, &[]);
        assert_eq!(ranges.len(), 3);
        assert_eq!(ranges.iter().map(|r| r.height).sum::<u32>(), 250);
    }

    #[test]
    fn a_zero_height_page_cannot_produce_infinite_pages() {
        let options = ShotOptions {
            page_height: 0,
            ..ShotOptions::default()
        };
        assert_eq!(page_height(&options), MIN_HEIGHT);
    }

    #[test]
    fn block_boxes_survive_an_id_containing_a_colon() {
        let dom = "<html data-dx-blocks=\"a:b:10:20;c:0:5\"></html>";
        assert_eq!(
            read_block_boxes(dom),
            boxes(&[("a:b", 10, 20), ("c", 0, 5)])
        );
    }

    #[test]
    fn a_malformed_box_entry_is_skipped_not_fatal() {
        let dom = "<html data-dx-blocks=\"good:0:10;broken;also-bad:x:y\"></html>";
        assert_eq!(read_block_boxes(dom), boxes(&[("good", 0, 10)]));
    }

    #[test]
    fn the_shift_style_lands_in_the_head_and_only_when_needed() {
        let page = "<html><head><title>t</title></head><body>x</body></html>";
        assert_eq!(shifted_to(page, 0), page);
        let shifted = shifted_to(page, 1550);
        assert!(shifted.contains("top:-1550px"));
        assert!(shifted.find("<style>").unwrap() < shifted.find("</head>").unwrap());
    }

    #[test]
    fn a_real_browser_captures_one_page_per_screenful() {
        let Some(_) = browser::find() else {
            return; // No browser on this machine.
        };
        let long = (1..=40)
            .map(|n| format!("::paragraph id=p{n}\nParagraph number {n} of the document.\n::end\n"))
            .collect::<Vec<_>>()
            .join("\n");
        let pages = capture_pages(
            &parse(&long),
            &ShotOptions {
                page_height: 400,
                ..ShotOptions::default()
            },
        )
        .expect("capture");

        assert!(pages.len() > 1, "a long document must paginate");
        assert_eq!(pages[0].number, 1);
        assert_eq!(pages[0].total, pages.len().max(pages[0].total));
        for page in &pages {
            assert_eq!(&page.shot.png[1..4], b"PNG");
            assert!(!page.blocks.is_empty(), "a page must say what is on it");
        }
        // Every block of the document appears on exactly one page.
        let named: Vec<&String> = pages.iter().flat_map(|page| &page.blocks).collect();
        assert_eq!(named.len(), 40, "every block accounted for, none twice");
    }

    #[test]
    fn heights_are_read_back_out_of_a_dumped_dom() {
        assert_eq!(
            read_height_attribute("<html data-dx-height=\"1840\"><body></body></html>"),
            Some(1840)
        );
        assert_eq!(read_height_attribute("<html></html>"), None);
    }

    #[test]
    fn file_urls_are_well_formed_on_both_path_shapes() {
        assert_eq!(file_url(Path::new("/tmp/a.html")), "file:///tmp/a.html");
        assert_eq!(
            file_url(Path::new(r"C:\Users\a.html")),
            "file:///C:/Users/a.html"
        );
    }

    #[test]
    fn a_machine_with_no_browser_gets_an_explanation_not_a_failure_to_explain() {
        // Simulating "no browser" would mean editing PATH, which is process-wide and would
        // sabotage every test running beside this one. The reachable contract is the
        // message itself: `capture` returns exactly this when discovery comes up empty.
        let message = browser::missing_message();
        assert!(message.contains("install"));
        assert!(message.contains("Text rendering works"));
    }

    #[test]
    fn a_real_browser_captures_a_whole_document() {
        let Some(_) = browser::find() else {
            return; // No browser on this machine; the capture path is exercised elsewhere.
        };
        let document = parse(
            "::heading level=1 id=h\nCapture check\n::end\n\n::paragraph id=p\nBody line.\n::end\n",
        );
        let shot = capture(&document, &ShotOptions::default()).expect("capture");
        assert_eq!(&shot.png[1..4], b"PNG");
        assert_eq!(shot.width, DEFAULT_WIDTH);
        assert!(shot.height >= 1);
    }
}
