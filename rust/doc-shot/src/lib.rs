//! `doc-shot` — render a `.dx` document to a picture.
//!
//! This is what lets an agent *look* at a document instead of only reading it. The same
//! HTML a person sees in their editor is loaded in a headless browser and captured as a
//! PNG, so a chart, a table, or a diagram arrives as an image rather than as a description
//! of an image.
//!
//! # How the full page fits in the frame
//! A headless browser screenshot is only as tall as its window, so capture happens in two
//! steps over one live [`cdp`] session: the page is asked its real content height, then the
//! viewport is sized exactly that tall and the picture taken. The result is the whole
//! document, never a cropped viewport — and a whole capture, however many pages it
//! produces, costs **one** browser launch (the one-shot `--screenshot` route it replaced
//! paid one per page).
//!
//! The measuring question is asked over the DevTools channel of the throwaway capture
//! copy. The stored document and everything [`doc_core::render`] produces stay script-free.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::all)]

pub mod base64;
pub mod browser;
pub mod cdp;
pub mod diff;
pub mod play;
pub mod png;

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use doc_core::model::Document;
use doc_core::render::{block_page, html, BlockPage, HtmlOptions, PageBounds, Theme};

/// Default capture width in CSS pixels.
pub const DEFAULT_WIDTH: u32 = 1200;

/// Default height of one page, in CSS pixels — roughly a sheet of paper at
/// [`DEFAULT_WIDTH`], so a paginated capture reads like the printed document.
pub const DEFAULT_PAGE_HEIGHT: u32 = 1550;

/// Most pages one paginated capture will produce.
///
/// A reader who needs the 30th page of something wants a section, not a flip-book.
/// [`capture_pages`] reports the pages it did not take rather than pretending the
/// document ended.
pub const MAX_PAGES: usize = 12;

/// Most pixels one image may carry before a vision model's ingestion scales it down
/// (~1.15 megapixels). [`ShotOptions::for_reading`] sizes pages under this, so every
/// pixel the image carries is a pixel the model actually sees. *Delivering* larger
/// only produces an image something else shrinks in transit, by whatever resample it
/// happens to use — which is why the extra density a reading capture rasterizes at
/// ([`ShotOptions::oversample`]) is averaged back down here, in linear light, before
/// the image leaves this crate.
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

/// Real milliseconds a capture lets a freshly opened page settle for
/// ([`cdp::Cdp::open_settled`]). Real, not virtual: virtual time froze a `::view`'s
/// sandboxed frame at every stage it was tried, and the local, network-free pages this
/// crate loads settle in real milliseconds — with `stable_screenshot` holding the page
/// to its own definition of settled on top.
const SETTLE_BUDGET_MS: u32 = 500;

/// Real milliseconds granted after each scroll of a paginated capture, so content the
/// window just reached — a sandboxed frame paints only inside the viewport — has time
/// to paint before the stability loop starts looking.
const SCROLL_SETTLE_BUDGET_MS: u32 = 250;

/// Shortest page captured, so a one-line document still produces a legible image.
const MIN_HEIGHT: u32 = 200;

/// Shortest a lone-block capture may come out. A trimmed block is content, not a page:
/// a one-line strip is the honest picture, and padding it to [`MIN_HEIGHT`] would ship
/// the sheet of blank paper the trim exists to remove. The floor only catches a failed
/// or degenerate measure.
const BLOCK_MIN_HEIGHT: u32 = 24;

/// Tallest page captured, so a runaway document cannot produce a gigantic image.
const MAX_HEIGHT: u32 = 12_000;

/// Tallest single screenshot a whole-document capture takes, in CSS pixels: a taller
/// page is photographed as scrolled strips this size and stitched (`png::stack`), so
/// every `::view` frame passes through the viewport it needs to paint.
const STRIP_HEIGHT: u32 = 2_000;

/// Largest CSS-pixel edge a self-sized page (a board at its natural size) may open a
/// window at, unless [`ShotOptions`] tightens it further.
const NATURAL_MAX_EDGE: u32 = 4000;

/// Most CSS pixels a self-sized page may cover, unless [`ShotOptions`] tightens it.
const NATURAL_MAX_PIXELS: u32 = 16_000_000;

/// The measuring question a paginated capture asks the live page: the `<body>` box's
/// height, then each flow block's box as `id:top:height;…` — the answer
/// [`parse_measure_answer`] reads. The height comes from the `<body>` box, not
/// `documentElement.scrollHeight`: the latter never reports less than the viewport, which
/// would pad every short document with a screenful of empty space. Only blocks **in the
/// page flow** are measured: a board renders a copy of every block it references inside
/// its own scaled canvas, each copy carrying the block's `data-block-id`, and measuring
/// those used to hand pagination boxes from inside the board's viewport — strip pages and
/// blank pages around every board, one block attributed to three pages. The board element
/// itself (it has the class *and* the id) stays measured — it is the flow.
const MEASURE_EXPRESSION: &str = "(function(){var b=document.body;\
var f=Array.prototype.filter.call(document.querySelectorAll('[data-block-id]'),\
function(e){var g=e.closest('.dx-board');return !g||g===e;});\
return String(Math.ceil(Math.max(b.scrollHeight,b.getBoundingClientRect().height)))+'|'+\
f.map(function(e){var x=e.getBoundingClientRect();\
return e.getAttribute('data-block-id')+':'+Math.max(0,Math.round(x.top+window.scrollY))\
+':'+Math.ceil(x.height);}).join(';');})()";

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
    /// Image pixels per CSS pixel *delivered*, clamped to 1–4. 2 makes an export match
    /// a high-density screen; 1 is right for a vision model, whose ingestion would only
    /// scale the extra pixels back out (see [`VISION_MAX_PIXELS`]).
    pub scale: u32,
    /// Extra density rasterized per delivered pixel, clamped so `scale × oversample`
    /// stays within the browser's sane range. The page is rasterized at
    /// `scale × oversample` and averaged back down to `scale` in linear light
    /// ([`png::downsample`]) before the capture leaves this crate — supersampling, so a
    /// hairline rule or a near-threshold mark survives the small image as gray ink
    /// instead of falling between the raster and vanishing. 1 delivers the browser's
    /// own rasterization untouched.
    pub oversample: u32,
    /// Longest CSS-pixel edge a self-sized page may reach. A board is captured at its
    /// natural canvas size — every node at the size its line states — and only shrunk,
    /// uniformly, when it would pass this. [`ShotOptions::for_reading`] sets the vision
    /// caps here so a board page arrives at the model unscaled.
    pub max_page_edge: u32,
    /// Most CSS pixels (width × height) a self-sized page may cover.
    pub max_page_pixels: u32,
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
            oversample: 1,
            max_page_edge: NATURAL_MAX_EDGE,
            max_page_pixels: NATURAL_MAX_PIXELS,
        }
    }
}

impl ShotOptions {
    /// Options for a read by a vision model: pages that arrive exactly as delivered.
    ///
    /// Each page stays within [`VISION_MAX_PIXELS`] and [`VISION_MAX_EDGE`], the two
    /// limits past which ingestion downscales an image, so zero compaction happens
    /// between this crate and the model. The pixels themselves are better than a
    /// budget-sized rasterization: the page is captured at twice the density
    /// (`oversample`) and averaged down in linear light, so fine ink the budget cannot
    /// afford a full pixel still arrives as gray instead of vanishing. Pages still
    /// break between blocks — a narrower, shorter page means more pages, never a
    /// sliced line. `width` overrides [`READ_WIDTH`]; the page height is derived so
    /// the pixel budget holds.
    pub fn for_reading(width: Option<u32>) -> Self {
        let width = width
            .unwrap_or(READ_WIDTH)
            .clamp(MIN_WIDTH, VISION_MAX_EDGE);
        Self {
            width,
            page_height: (VISION_MAX_PIXELS / width).min(VISION_MAX_EDGE),
            scale: 1,
            oversample: 2,
            max_page_edge: VISION_MAX_EDGE,
            max_page_pixels: VISION_MAX_PIXELS,
            ..Self::default()
        }
    }

    /// The bounds a self-sized page must fit, for [`doc_core::render::block_page`].
    fn page_bounds(&self) -> PageBounds {
        PageBounds {
            width: self.width,
            max_edge: self.max_page_edge.max(MIN_WIDTH),
            max_pixels: self.max_page_pixels.max(MIN_WIDTH * MIN_HEIGHT),
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
/// A `::board` is always a page of its own, photographed **independently** at its natural
/// canvas size ([`capture_block`]'s render) instead of as the column-fitted miniature the
/// flow carries — the fitted board is right for reading in context and unreadable as a
/// picture. The board's page slots in exactly where the board sits in the flow, so page
/// order is still reading order.
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
    let result = capture_all_pages(&browser, document, &page, &headings, options, &workspace);
    let _ = std::fs::remove_dir_all(&workspace);
    result
}

/// Render the single block called `id` and capture it as a PNG.
///
/// A `::board` is photographed at its **natural size** — every node at exactly the box its
/// line states, shrunk uniformly only past `options.max_page_edge`/`max_page_pixels`, never
/// enlarged — which is how a board is validated: what the picture shows is what the lines
/// say. Any other block (a board node's block included, hidden or not) is photographed in
/// the ordinary column, exactly as the page carries it.
pub fn capture_block(document: &Document, id: &str, options: &ShotOptions) -> Result<Shot, String> {
    let browser = browser::find().ok_or_else(browser::missing_message)?;
    let page = block_page(
        document,
        id,
        &block_html_options(options.theme),
        &options.page_bounds(),
    )
    .ok_or_else(|| format!("no block named `{id}`"))?;

    let workspace = scratch_workspace(&options.scratch_dir)?;
    let result = capture_block_page(&browser, &page, options, &workspace);
    let _ = std::fs::remove_dir_all(&workspace);
    result
}

/// Render every block named in `ids` and capture all of them from **one** browser session.
///
/// The batch form of [`capture_block`]: each shot is the same picture that function takes —
/// a `::board` at its natural size, anything else exactly as the page carries it — but a
/// single Chromium launch serves the whole list. The live [`cdp`] session loads each
/// block's page in turn and photographs it through the DevTools channel, so a visual loop
/// over N blocks pays one browser startup instead of N (the one-shot `--screenshot` route
/// pays one per pass). Shots come back in the order the ids were given, one per id.
///
/// An id naming no block fails the whole batch — before any browser starts — so a typo is
/// a sentence, never a half-written set of pictures.
pub fn capture_blocks(
    document: &Document,
    ids: &[&str],
    options: &ShotOptions,
) -> Result<Vec<Shot>, String> {
    let html_options = block_html_options(options.theme);
    let mut pages = Vec::with_capacity(ids.len());
    for id in ids {
        pages.push(
            block_page(document, id, &html_options, &options.page_bounds())
                .ok_or_else(|| format!("no block named `{id}`"))?,
        );
    }

    let browser = browser::find().ok_or_else(browser::missing_message)?;
    let workspace = scratch_workspace(&options.scratch_dir)?;
    let result = capture_block_pages(&browser, &pages, options, &workspace);
    let _ = std::fs::remove_dir_all(&workspace);
    result
}

/// Capture every rendered [`BlockPage`] from one live [`cdp::Cdp`] session.
///
/// Each page is opened in turn — [`cdp::Cdp::open`] waits for its load event, so nothing
/// here reads the previous block's page — then the viewport is sized to it: a page that
/// states its own height (a board) gets a viewport exactly that size, one that does not is
/// measured on the live page first. The capture rasterizes at `scale × oversample` and is
/// averaged back down in linear light, exactly like every other capture in this crate.
fn capture_block_pages(
    browser: &Path,
    pages: &[BlockPage],
    options: &ShotOptions,
    workspace: &Path,
) -> Result<Vec<Shot>, String> {
    let profile = workspace.join("profile");
    std::fs::create_dir_all(&profile)
        .map_err(|error| format!("could not create {}: {error}", profile.display()))?;
    let scale = scale(options);
    let oversample = oversample(options);

    let mut session =
        cdp::Cdp::launch(browser, &profile, options.width, MEASURE_HEIGHT, None, None)?;
    let mut shots = Vec::with_capacity(pages.len());
    for (index, page) in pages.iter().enumerate() {
        let page_file = workspace.join(format!("block-{index}.html"));
        write(&page_file, &page.html)?;
        session.open_settled(&file_url(&page_file), SETTLE_BUDGET_MS)?;
        let height = match page.height {
            Some(height) => height,
            None => {
                // Measure at scale 1, like the one-shot measuring pass: the height is
                // CSS pixels, and the capture applies its own density on top of them.
                set_viewport(&mut session, page.width, MEASURE_HEIGHT, 1)?;
                measure_body_height(&mut session).unwrap_or(MEASURE_HEIGHT)
            }
        };
        let height = height.clamp(BLOCK_MIN_HEIGHT, MAX_HEIGHT);
        set_viewport(&mut session, page.width, height, scale * oversample)?;
        let png = stable_screenshot(&mut session)?;
        let png = png::downsample(&png, oversample)
            .map_err(|reason| format!("could not resample the capture: {reason}"))?;
        shots.push(Shot {
            png,
            width: page.width * scale,
            height: height * scale,
        });
    }
    Ok(shots)
}

/// Size the live page's viewport in CSS pixels, rasterized at `density` pixels each.
fn set_viewport(
    session: &mut cdp::Cdp,
    width: u32,
    height: u32,
    density: u32,
) -> Result<(), String> {
    session
        .command(
            "Emulation.setDeviceMetricsOverride",
            serde_json::json!({
                "width": width,
                "height": height,
                "deviceScaleFactor": density,
                "mobile": false,
            }),
        )
        .map(|_| ())
}

/// The live page's content height in CSS pixels — the same `<body>` box the one-shot
/// measuring script reads (`documentElement.scrollHeight` never reports less than the
/// viewport, which would pad every short block with a screenful of empty space).
fn measure_body_height(session: &mut cdp::Cdp) -> Option<u32> {
    let value = session
        .evaluate(
            "Math.ceil(Math.max(document.body.scrollHeight, \
             document.body.getBoundingClientRect().height))",
        )
        .ok()?;
    let height = value.as_f64()?;
    if !height.is_finite() || height < 0.0 {
        return None;
    }
    Some(height as u32)
}

/// Capture an already-rendered [`BlockPage`] inside `workspace`: a batch of one.
fn capture_block_page(
    browser: &Path,
    page: &BlockPage,
    options: &ShotOptions,
    workspace: &Path,
) -> Result<Shot, String> {
    let mut shots = capture_block_pages(browser, std::slice::from_ref(page), options, workspace)?;
    shots
        .pop()
        .ok_or_else(|| "the capture produced no image".to_string())
}

/// The render options every capture in this crate photographs a block under: the caller's
/// theme, code open (a picture cannot be clicked).
fn block_html_options(theme: Theme) -> HtmlOptions {
    HtmlOptions {
        theme,
        collapse_code: false,
        ..HtmlOptions::default()
    }
}

/// One planned page of a paginated capture.
enum PlannedPage {
    /// A window over the document's flow.
    Flow(PageRange),
    /// A board photographed independently at its natural size.
    Board {
        /// The board block's id.
        id: String,
        /// Its standalone, self-sized page.
        page: BlockPage,
    },
}

/// Measure once, then capture each planned page — the whole run from **one** live
/// [`cdp`] session inside an already-created `workspace`.
fn capture_all_pages(
    browser: &Path,
    document: &Document,
    page: &str,
    headings: &[String],
    options: &ShotOptions,
    workspace: &Path,
) -> Result<Vec<Page>, String> {
    let profile = workspace.join("profile");
    std::fs::create_dir_all(&profile)
        .map_err(|error| format!("could not create {}: {error}", profile.display()))?;
    let mut session =
        cdp::Cdp::launch(browser, &profile, options.width, MEASURE_HEIGHT, None, None)?;

    let page_file = workspace.join("measure.html");
    write(&page_file, page)?;
    session.open_settled(&file_url(&page_file), SETTLE_BUDGET_MS)?;
    set_viewport(&mut session, options.width, MEASURE_HEIGHT, 1)?;
    let (total_height, boxes) = measure_page(&mut session);

    let plan = plan_pages(
        document,
        &boxes,
        total_height,
        page_height(options),
        headings,
        options,
    );
    let total = plan.len();
    let scale = scale(options);
    let oversample = oversample(options);

    // Every flow page is a window scrolled over the document already open in the
    // session — the document loads once, however many pages it breaks into. Scrolling,
    // not a clip past the viewport: a `::view`'s sandboxed frame only paints inside the
    // viewport, and `captureBeyondViewport` delivered nine of them as empty paper.
    // Boards follow in a second pass because each opens its own self-sized page, and
    // interleaving would make the session reload the document around every one of them.
    let mut pages: Vec<Option<Page>> = Vec::new();
    pages.resize_with(plan.len().min(options.max_pages), || None);
    for (index, planned) in plan.iter().take(options.max_pages).enumerate() {
        if let PlannedPage::Flow(range) = planned {
            let height = range.height.clamp(MIN_HEIGHT, MAX_HEIGHT);
            set_viewport(&mut session, options.width, height, scale * oversample)?;
            session.evaluate(&format!("window.scrollTo(0,{})", range.top))?;
            session.settle(SCROLL_SETTLE_BUDGET_MS);
            let shot = delivered(
                stable_screenshot(&mut session)?,
                options.width,
                height,
                options,
            )?;
            pages[index] = Some(Page {
                shot,
                number: index + 1,
                total,
                blocks: range.blocks.clone(),
            });
        }
    }
    for (index, planned) in plan.iter().take(options.max_pages).enumerate() {
        if let PlannedPage::Board { id, page: board } = planned {
            let board_file = workspace.join(format!("page-{index}.html"));
            write(&board_file, &board.html)?;
            session.open_settled(&file_url(&board_file), SETTLE_BUDGET_MS)?;
            let height = board
                .height
                .unwrap_or_else(|| page_height(options))
                .clamp(MIN_HEIGHT, MAX_HEIGHT);
            set_viewport(&mut session, board.width, height, scale * oversample)?;
            let shot = delivered(
                stable_screenshot(&mut session)?,
                board.width,
                height,
                options,
            )?;
            pages[index] = Some(Page {
                shot,
                number: index + 1,
                total,
                blocks: vec![id.clone()],
            });
        }
    }

    Ok(pages.into_iter().flatten().collect())
}

/// Photograph the page once it stops changing.
///
/// Virtual time ([`cdp::Cdp::settle`]) advances the page's own clocks, but the
/// compositor rasters in real time — a heavy `::view` frame can take a few hundred real
/// milliseconds to paint after its page is "loaded", and a capture that fires first
/// ships the frame as empty paper. Two consecutive captures that agree are the
/// definition of settled; a page that never stops moving is delivered as it last stood
/// rather than stalling the read.
fn stable_screenshot(session: &mut cdp::Cdp) -> Result<Vec<u8>, String> {
    /// Real time between looks — about two compositor frames.
    const BREATH: std::time::Duration = std::time::Duration::from_millis(120);
    /// Most looks taken before the page is delivered as it stands.
    const MOST_LOOKS: usize = 12;
    let mut last = session.screenshot(None)?;
    for _ in 0..MOST_LOOKS {
        std::thread::sleep(BREATH);
        let next = session.screenshot(None)?;
        if next == last {
            return Ok(next);
        }
        last = next;
    }
    Ok(last)
}

/// Average an oversampled capture back down and box it as the [`Shot`] it claims to be:
/// `width × scale` pixels, whatever density it was rasterized at.
fn delivered(png: Vec<u8>, width: u32, height: u32, options: &ShotOptions) -> Result<Shot, String> {
    let png = png::downsample(&png, oversample(options))
        .map_err(|reason| format!("could not resample the capture: {reason}"))?;
    Ok(Shot {
        png,
        width: width * scale(options),
        height: height * scale(options),
    })
}

/// Ask the live page [`MEASURE_EXPRESSION`] and read its answer.
///
/// A page that cannot be measured still captures — at [`MEASURE_HEIGHT`] with no block
/// boxes, so the breaks are less precise, never absent.
fn measure_page(session: &mut cdp::Cdp) -> (u32, Vec<BlockBox>) {
    session
        .evaluate(MEASURE_EXPRESSION)
        .ok()
        .as_ref()
        .and_then(serde_json::Value::as_str)
        .map_or((MEASURE_HEIGHT, Vec::new()), parse_measure_answer)
}

/// Parse [`MEASURE_EXPRESSION`]'s answer: the page height, `|`, then the block boxes.
fn parse_measure_answer(answer: &str) -> (u32, Vec<BlockBox>) {
    let (height, boxes) = answer.split_once('|').unwrap_or((answer, ""));
    (
        height.parse().unwrap_or(MEASURE_HEIGHT),
        parse_block_boxes(boxes),
    )
}

/// Divide the measured flow into pages, giving every board a page of its own.
///
/// The flow between boards paginates by [`packed_ranges`] — the most whole blocks that fit
/// a page. Where a board sits, its **independent** natural-size page takes the slot, and
/// the flow resumes below it; the fitted miniature the flow page carries there is never
/// photographed. Trailing space after the last block (the sheet's own bottom margin) makes
/// no page.
fn plan_pages(
    document: &Document,
    boxes: &[BlockBox],
    total_height: u32,
    height: u32,
    sticky: &[String],
    options: &ShotOptions,
) -> Vec<PlannedPage> {
    let board_ids: Vec<&str> = document
        .blocks
        .iter()
        .filter(|block| block.kind == "board" && !block.hidden)
        .map(|block| block.id.as_str())
        .collect();
    if boxes.is_empty() || board_ids.is_empty() {
        return page_ranges(boxes, total_height, height, sticky)
            .into_iter()
            .map(PlannedPage::Flow)
            .collect();
    }

    let html_options = block_html_options(options.theme);
    let mut plan = Vec::new();
    let mut run_start = 0;
    let mut run_top = 0u32;
    for (index, entry) in boxes.iter().enumerate() {
        if !board_ids.contains(&entry.id.as_str()) {
            continue;
        }
        let run = &boxes[run_start..index];
        if !run.is_empty() {
            let end = entry.top.max(run_top.saturating_add(1));
            plan.extend(
                packed_ranges(run, run_top, end, height, sticky)
                    .into_iter()
                    .map(PlannedPage::Flow),
            );
        }
        if let Some(page) = block_page(document, &entry.id, &html_options, &options.page_bounds()) {
            plan.push(PlannedPage::Board {
                id: entry.id.clone(),
                page,
            });
        }
        run_top = entry.top.saturating_add(entry.height);
        run_start = index + 1;
    }

    let run = &boxes[run_start..];
    if !run.is_empty() || plan.is_empty() {
        let end = total_height.max(run_top.saturating_add(1));
        plan.extend(
            packed_ranges(run, run_top, end, height, sticky)
                .into_iter()
                .map(PlannedPage::Flow),
        );
    }
    plan
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
    packed_ranges(boxes, 0, total_height.max(1), height, sticky)
}

/// [`page_ranges`] over one vertical span of the flow: pages start at `start` and the last
/// one ends at `end`. This is the packing rule applied between boards, which paginate
/// separately ([`plan_pages`]).
fn packed_ranges(
    boxes: &[BlockBox],
    start: u32,
    end: u32,
    height: u32,
    sticky: &[String],
) -> Vec<PageRange> {
    let end = end.max(start.saturating_add(1));
    if boxes.is_empty() {
        return fixed_ranges(start, end, height);
    }

    let ids = |range: std::ops::Range<usize>| -> Vec<String> {
        boxes[range].iter().map(|entry| entry.id.clone()).collect()
    };

    let mut ranges: Vec<PageRange> = Vec::new();
    let mut top = start;
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

    let tail = end.max(top + 1);
    ranges.push(PageRange {
        top,
        height: tail - top,
        blocks: ids(page_start..boxes.len()),
    });
    ranges
}

/// Divide the span `start..end` into equal pages, used when no block boxes were measured.
fn fixed_ranges(start: u32, end: u32, height: u32) -> Vec<PageRange> {
    let mut ranges = Vec::new();
    let mut top = start;
    while top < end {
        ranges.push(PageRange {
            top,
            height: height.min(end - top),
            blocks: Vec::new(),
        });
        top += height;
    }
    if ranges.is_empty() {
        ranges.push(PageRange {
            top: start,
            height: end.saturating_sub(start).max(1),
            blocks: Vec::new(),
        });
    }
    ranges
}

/// Measure `page` and photograph it whole, from one live [`cdp`] session inside an
/// already-created `workspace`.
fn capture_page(
    browser: &Path,
    page: &str,
    options: &ShotOptions,
    workspace: &Path,
) -> Result<Shot, String> {
    let profile = workspace.join("profile");
    std::fs::create_dir_all(&profile)
        .map_err(|error| format!("could not create {}: {error}", profile.display()))?;
    let mut session =
        cdp::Cdp::launch(browser, &profile, options.width, MEASURE_HEIGHT, None, None)?;

    let page_file = workspace.join("page.html");
    write(&page_file, page)?;
    session.open_settled(&file_url(&page_file), SETTLE_BUDGET_MS)?;
    set_viewport(&mut session, options.width, MEASURE_HEIGHT, 1)?;
    let height = measure_body_height(&mut session)
        .unwrap_or(MEASURE_HEIGHT)
        .clamp(MIN_HEIGHT, MAX_HEIGHT);
    let density = scale(options) * oversample(options);

    if height <= STRIP_HEIGHT {
        set_viewport(&mut session, options.width, height, density)?;
        return delivered(
            stable_screenshot(&mut session)?,
            options.width,
            height,
            options,
        );
    }

    // A tall page is photographed as scrolled strips and stitched: a `::view`'s
    // sandboxed frame only paints inside the viewport, so one full-height viewport
    // ships every deep frame as empty paper — the same reason the paged route scrolls
    // a window over the document instead of clipping past it.
    let mut strips = Vec::new();
    let mut top = 0;
    let mut viewport = 0;
    while top < height {
        let strip = STRIP_HEIGHT.min(height - top);
        if strip != viewport {
            set_viewport(&mut session, options.width, strip, density)?;
            viewport = strip;
        }
        session.evaluate(&format!("window.scrollTo(0,{top})"))?;
        session.settle(SCROLL_SETTLE_BUDGET_MS);
        strips.push(
            png::decode(&stable_screenshot(&mut session)?)
                .map_err(|reason| format!("could not read a captured strip: {reason}"))?,
        );
        top += strip;
    }
    let stitched =
        png::stack(&strips).map_err(|reason| format!("could not stitch the capture: {reason}"))?;
    delivered(stitched, options.width, height, options)
}

/// The device scale factor a capture delivers, never zero and never absurd.
fn scale(options: &ShotOptions) -> u32 {
    options.scale.clamp(1, MAX_SCALE)
}

/// The extra density a capture rasterizes at, clamped so the browser is never asked
/// for more than [`MAX_SCALE`] in total.
fn oversample(options: &ShotOptions) -> u32 {
    options.oversample.clamp(1, MAX_SCALE / scale(options))
}

/// Parse each block's measured box out of [`MEASURE_EXPRESSION`]'s `id:top:height;…`
/// list, in document order.
///
/// A malformed entry is skipped rather than failing the capture: a page that measured
/// imperfectly should still produce pictures, just with less precise breaks.
fn parse_block_boxes(raw: &str) -> Vec<BlockBox> {
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
                "delivered density past 1 would only be scaled back out in transit"
            );
            assert_eq!(
                options.oversample, 2,
                "the budget-sized page is averaged down from a denser rasterization"
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
    fn oversampling_never_asks_the_browser_past_its_sane_range() {
        // scale × oversample is what the browser is asked for; the product stays
        // within MAX_SCALE, giving up oversample before giving up delivered density.
        let reading = ShotOptions::for_reading(None);
        assert_eq!(scale(&reading) * oversample(&reading), 2);
        let dense_export = ShotOptions {
            scale: 4,
            oversample: 2,
            ..ShotOptions::default()
        };
        assert_eq!(oversample(&dense_export), 1, "no room above scale 4");
        let absurd = ShotOptions {
            scale: 1,
            oversample: 99,
            ..ShotOptions::default()
        };
        assert_eq!(oversample(&absurd), MAX_SCALE);
    }

    #[test]
    fn the_measure_answer_carries_the_height_then_the_boxes() {
        assert_eq!(
            parse_measure_answer("1840|intro:0:120;plan:140:300"),
            (1840, boxes(&[("intro", 0, 120), ("plan", 140, 300)]))
        );
        let (height, empty) = parse_measure_answer("640|");
        assert_eq!((height, empty.len()), (640, 0));
        let (fallback, none) = parse_measure_answer("not a number");
        assert_eq!((fallback, none.len()), (MEASURE_HEIGHT, 0));
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

    const BOARDED: &str = "::paragraph id=intro\nBefore.\n::end\n\n\
::board id=plan height=400\n- note x=0 y=0 w=400 h=200\n::end\n\n\
::paragraph id=outro\nAfter.\n::end\n\n\
::paragraph id=note hidden\nOn the board.\n::end\n";

    /// A board is a page of its own, photographed at its natural size, slotted exactly
    /// where the board sits in the flow — and the flow resumes below it with no strip
    /// page, no blank page, and no block attributed to a page it is not on.
    #[test]
    fn a_board_gets_its_own_natural_page_and_the_flow_resumes_below_it() {
        let document = parse(BOARDED);
        let measured = boxes(&[("intro", 0, 100), ("plan", 130, 400), ("outro", 560, 100)]);
        let plan = plan_pages(
            &document,
            &measured,
            700,
            1000,
            &[],
            &ShotOptions::default(),
        );

        assert_eq!(plan.len(), 3);
        let PlannedPage::Flow(before) = &plan[0] else {
            panic!("the flow before the board is a flow page");
        };
        assert_eq!(before.blocks, vec!["intro"]);
        assert_eq!((before.top, before.height), (0, 130));

        let PlannedPage::Board { id, page } = &plan[1] else {
            panic!("the board is its own page");
        };
        // The node is 400×200; the natural viewport adds the 24px fit margin around it —
        // not the 400px flow frame, and not the capture width.
        assert_eq!(id, "plan");
        assert_eq!((page.width, page.height), (448, Some(248)));

        let PlannedPage::Flow(after) = &plan[2] else {
            panic!("the flow after the board is a flow page");
        };
        assert_eq!(after.blocks, vec!["outro"]);
        assert_eq!((after.top, after.height), (530, 170));
    }

    /// A document that ends with a board ends with the board's page: the sheet's own
    /// bottom margin makes no blank trailing page.
    #[test]
    fn a_trailing_board_leaves_no_blank_page_after_it() {
        let document = parse(
            "::paragraph id=intro\nBefore.\n::end\n\n\
::board id=plan height=400\n- note x=0 y=0 w=400 h=200\n::end\n\n\
::paragraph id=note hidden\nOn the board.\n::end\n",
        );
        let measured = boxes(&[("intro", 0, 100), ("plan", 130, 400)]);
        let plan = plan_pages(
            &document,
            &measured,
            700,
            1000,
            &[],
            &ShotOptions::default(),
        );
        assert_eq!(plan.len(), 2);
        assert!(matches!(&plan[1], PlannedPage::Board { .. }));
    }

    /// A read's board page stays inside the vision budget the way every page does: shrunk
    /// before capture, so no pixel is scaled away in transit.
    #[test]
    fn a_reading_boards_page_fits_the_vision_budget() {
        let document = parse(
            "::board id=plan height=400\n- wide x=0 y=0 w=3000 h=200\n::end\n\n\
::paragraph id=wide hidden\nA very wide node.\n::end\n",
        );
        let measured = boxes(&[("plan", 0, 400)]);
        let options = ShotOptions::for_reading(None);
        let plan = plan_pages(&document, &measured, 400, 1337, &[], &options);
        let PlannedPage::Board { page, .. } = &plan[0] else {
            panic!("the board is its own page");
        };
        let height = page.height.expect("a board states its height");
        assert!(page.width <= VISION_MAX_EDGE, "{}", page.width);
        assert!(page.width * height <= VISION_MAX_PIXELS + page.width);
    }

    /// The measuring question only measures the flow: a block copied into a board node
    /// carries the same `data-block-id`, and measuring the copy was what cut strip pages
    /// and misattributed blocks around every board.
    #[test]
    fn the_measuring_question_skips_the_copies_inside_a_board() {
        assert!(MEASURE_EXPRESSION.contains(".dx-board"));
        assert!(MEASURE_EXPRESSION.contains("closest"));
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
        assert_eq!(
            parse_block_boxes("a:b:10:20;c:0:5"),
            boxes(&[("a:b", 10, 20), ("c", 0, 5)])
        );
    }

    #[test]
    fn a_malformed_box_entry_is_skipped_not_fatal() {
        assert_eq!(
            parse_block_boxes("good:0:10;broken;also-bad:x:y"),
            boxes(&[("good", 0, 10)])
        );
    }

    #[test]
    fn a_real_browser_captures_one_page_per_screenful() {
        let _turn = browser::ENV_LOCK.lock().expect("env lock");
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

    /// End to end against a real browser: a boarded document paginates with the board on
    /// its own natural-size page, and one block captures alone at either size rule.
    #[test]
    fn a_real_browser_captures_a_board_independently_and_one_block_alone() {
        let _turn = browser::ENV_LOCK.lock().expect("env lock");
        let Some(_) = browser::find() else {
            return; // No browser on this machine.
        };
        let document = parse(BOARDED);

        let pages = capture_pages(&document, &ShotOptions::default()).expect("pages");
        let board_page = pages
            .iter()
            .find(|page| page.blocks == ["plan"])
            .expect("the board must be a page of its own");
        assert_eq!(
            (board_page.shot.width, board_page.shot.height),
            (448, 248),
            "the board page is its natural size, not the capture width"
        );
        assert!(
            pages
                .iter()
                .all(|page| !page.blocks.iter().any(|b| b == "note")),
            "a board node's block belongs to no flow page: {:?}",
            pages.iter().map(|p| p.blocks.clone()).collect::<Vec<_>>()
        );

        let board = capture_block(&document, "plan", &ShotOptions::default()).expect("board");
        assert_eq!((board.width, board.height), (448, 248));
        // An ordinary block is trimmed to the page's own content measure — the picture is
        // the block, not the block plus the sheet's margins.
        let node = capture_block(&document, "note", &ShotOptions::default()).expect("node");
        assert_eq!(node.width, 680);
        assert!(
            node.height < 200,
            "a one-line block is a strip, not a sheet: {}px tall",
            node.height
        );

        let missing = capture_block(&document, "ghost", &ShotOptions::default());
        assert!(
            missing.is_err(),
            "a missing block is a sentence, not a shot"
        );
    }

    /// The batch form refuses an unknown id up front: no browser starts, and no
    /// half-written set of pictures is left behind a typo.
    #[test]
    fn a_batch_with_an_unknown_id_is_refused_before_any_browser_starts() {
        let document = parse(BOARDED);
        let error = capture_blocks(&document, &["intro", "ghost"], &ShotOptions::default())
            .expect_err("no such block");
        assert!(error.contains("ghost"), "{error}");
    }

    /// One live session serves the whole batch: every named block comes back as its own
    /// image, in the order asked — the board at its natural size, an ordinary block in
    /// the ordinary column — from a single Chromium launch.
    #[test]
    fn a_real_browser_captures_many_blocks_from_one_session() {
        let _turn = browser::ENV_LOCK.lock().expect("env lock");
        let Some(_) = browser::find() else {
            return; // No browser on this machine.
        };
        let document = parse(BOARDED);
        let shots = capture_blocks(
            &document,
            &["plan", "note", "intro"],
            &ShotOptions::default(),
        )
        .expect("batch");

        assert_eq!(shots.len(), 3, "one image per block");
        assert_eq!(
            (shots[0].width, shots[0].height),
            (448, 248),
            "the board keeps its natural size in a batch"
        );
        let board = png::decode(&shots[0].png).expect("the delivered PNG decodes");
        assert_eq!(
            (board.width, board.height),
            (448, 248),
            "the live session delivers the stated size, pixel for pixel"
        );
        assert_eq!(
            shots[1].width, 680,
            "an ordinary block is trimmed to the page's content measure"
        );
        for shot in &shots {
            assert_eq!(&shot.png[1..4], b"PNG");
        }
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
        let _turn = browser::ENV_LOCK.lock().expect("env lock");
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

    /// End to end against a real browser: a reading capture rasterizes at twice the
    /// density and arrives averaged back down to its stated size — which is also the
    /// proof that [`png`] speaks the browser's own PNG (its color type, its row
    /// filters), not only its own output.
    #[test]
    fn a_real_browser_reading_capture_arrives_at_its_stated_size() {
        let _turn = browser::ENV_LOCK.lock().expect("env lock");
        let Some(_) = browser::find() else {
            return; // No browser on this machine.
        };
        let document = parse(
            "::heading level=1 id=h\nFine ink\n::end\n\n::paragraph id=p\nBody line.\n::end\n",
        );
        let options = ShotOptions::for_reading(Some(400));
        assert_eq!(oversample(&options), 2);
        let shot = capture(&document, &options).expect("capture");
        let image = png::decode(&shot.png).expect("the delivered PNG decodes");
        assert_eq!(
            (image.width, image.height),
            (shot.width, shot.height),
            "the denser rasterization was averaged down before delivery"
        );
        assert_eq!(shot.width, 400);
    }
}
