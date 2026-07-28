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

pub mod browser;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use doc_core::model::Document;
use doc_core::render::{html, HtmlOptions, Theme};

/// Default capture width in CSS pixels.
pub const DEFAULT_WIDTH: u32 = 1200;

/// Window height used for the measuring pass, before the real height is known.
const MEASURE_HEIGHT: u32 = 900;

/// Shortest page captured, so a one-line document still produces a legible image.
const MIN_HEIGHT: u32 = 200;

/// Tallest page captured, so a runaway document cannot produce a gigantic image.
const MAX_HEIGHT: u32 = 12_000;

/// Attribute the measuring script writes the content height into.
const HEIGHT_ATTRIBUTE: &str = "data-dx-height";

/// How to capture a document.
#[derive(Debug, Clone)]
pub struct ShotOptions {
    /// Capture width in CSS pixels.
    pub width: u32,
    /// Palette to render with.
    pub theme: Theme,
    /// Apply the document's own `::style` blocks.
    pub document_css: bool,
    /// Directory for the temporary HTML the browser loads.
    pub scratch_dir: PathBuf,
}

impl Default for ShotOptions {
    fn default() -> Self {
        Self {
            width: DEFAULT_WIDTH,
            theme: Theme::Auto,
            document_css: false,
            scratch_dir: std::env::temp_dir(),
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

/// Render `document` and capture it as a PNG.
///
/// Returns a message naming what to install when no browser is available, so the caller
/// can fall back to text rather than failing outright.
pub fn capture(document: &Document, options: &ShotOptions) -> Result<Shot, String> {
    let browser = browser::find().ok_or_else(browser::missing_message)?;
    let page = html(
        document,
        &HtmlOptions {
            theme: options.theme,
            document_css: options.document_css,
            ..HtmlOptions::default()
        },
    );

    let workspace = scratch_workspace(&options.scratch_dir)?;
    let result = capture_page(&browser, &page, options, &workspace);
    let _ = std::fs::remove_dir_all(&workspace);
    result
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
    let height = measure_height(browser, &measure_file, options.width).unwrap_or(MEASURE_HEIGHT);

    let page_file = workspace.join("page.html");
    write(&page_file, page)?;
    let image_file = workspace.join("shot.png");
    let height = height.clamp(MIN_HEIGHT, MAX_HEIGHT);

    let status = browser_command(browser)
        .arg(format!("--screenshot={}", image_file.display()))
        .arg(format!("--window-size={},{height}", options.width))
        .arg(file_url(&page_file))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| format!("could not start the browser: {error}"))?;

    let png = std::fs::read(&image_file).map_err(|error| {
        format!(
            "the browser did not produce an image ({status}): {error}. \
             Try setting {} to a different browser.",
            browser::BROWSER_ENV
        )
    })?;

    Ok(Shot {
        png,
        width: options.width,
        height,
    })
}

/// Load the measuring copy and read the content height back out of the dumped DOM.
fn measure_height(browser: &Path, page_file: &Path, width: u32) -> Option<u32> {
    let output = browser_command(browser)
        .arg("--dump-dom")
        .arg(format!("--window-size={width},{MEASURE_HEIGHT}"))
        .arg(file_url(page_file))
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;

    let dom = String::from_utf8_lossy(&output.stdout);
    read_height_attribute(&dom)
}

/// Extract the measured height from a dumped DOM.
fn read_height_attribute(dom: &str) -> Option<u32> {
    let marker = format!("{HEIGHT_ATTRIBUTE}=\"");
    let start = dom.find(&marker)? + marker.len();
    let end = start + dom[start..].find('"')?;
    dom[start..end].parse().ok()
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
        "--force-device-scale-factor=1",
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
        "<script>var b=document.body;document.documentElement.setAttribute('{HEIGHT_ATTRIBUTE}', \
         String(Math.ceil(Math.max(b.scrollHeight, b.getBoundingClientRect().height))));</script>"
    );
    match page.rfind("</body>") {
        Some(index) => format!("{}{script}{}", &page[..index], &page[index..]),
        None => format!("{page}{script}"),
    }
}

/// Create a unique scratch directory for one capture.
fn scratch_workspace(root: &Path) -> Result<PathBuf, String> {
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
fn write(path: &Path, contents: &str) -> Result<(), String> {
    std::fs::write(path, contents)
        .map_err(|error| format!("could not write {}: {error}", path.display()))
}

/// A `file://` URL for a local path, which is what the browser needs to load it.
fn file_url(path: &Path) -> String {
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
    fn the_measuring_script_goes_inside_the_body() {
        let page = "<html><body><p>x</p></body></html>";
        let measured = with_measuring_script(page);
        assert!(measured.contains(HEIGHT_ATTRIBUTE));
        assert!(measured.ends_with("</body></html>"));
        assert!(measured.find("<script>").unwrap() < measured.find("</body>").unwrap());
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
