//! `play` — drive the rendered page the way a reader would, and keep every frame.
//!
//! A screenshot answers *what does this document look like*; play answers *what does it do
//! when touched*. The document is rendered once — the same script-free HTML every capture
//! in this crate photographs — loaded in a live [`crate::cdp`] session, and a small script
//! of inputs is performed against it: wait, key, click, scroll, hover. Every step of the
//! way a PNG frame is captured and stamped with when it was taken and which action
//! produced it, so a caller can review the behaviour frame by frame.
//!
//! Nothing the document carries executes. The page has no scripts to run
//! ([`doc_core::render`] emits none and strips the author's), the inputs are synthetic
//! events against static markup, and no file is read or written by the page. Play is a
//! read with hands.
//!
//! # The script
//! Statements are separated by `;` or newlines:
//!
//! ```text
//! wait 500ms; key Space; click steps; scroll 200; scroll steps 200; hover intro
//! ```
//!
//! A target is a block id (as `dx outline` reports them) or an `x,y` pixel coordinate.
//! During a `wait`, frames keep arriving at the configured rate; every other action
//! captures one frame the moment it lands, annotated with the action's own words.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use doc_core::model::Document;
use doc_core::render::Theme;
use serde_json::{json, Value};

use crate::cdp::{Cdp, Clip};

/// Default frames per second during waits.
pub const DEFAULT_FPS: u32 = 10;

/// Fastest capture rate offered. Above this the capture itself outruns the clock.
pub const MAX_FPS: u32 = 30;

/// Default viewport height for a play session, in CSS pixels.
pub const DEFAULT_HEIGHT: u32 = 900;

/// Most actions one script may carry.
pub const MAX_ACTIONS: usize = 100;

/// Longest total waiting one script may request, in milliseconds.
pub const MAX_PLAY_MS: u64 = 30_000;

/// Most frames one play will produce. A session that wants more must lower its fps or
/// shorten its waits — the limit is stated up front rather than truncating silently.
pub const MAX_FRAMES: usize = 300;

/// How to run a play session.
#[derive(Debug, Clone)]
pub struct PlayOptions {
    /// Viewport width in CSS pixels.
    pub width: u32,
    /// Viewport height in CSS pixels.
    pub height: u32,
    /// Palette to render with.
    pub theme: Theme,
    /// Frames per second captured during waits, clamped to `1..=`[`MAX_FPS`].
    pub fps: u32,
    /// Clip every frame to this block's box instead of the whole viewport.
    pub node: Option<String>,
    /// Directory for the temporary HTML and browser profile.
    pub scratch_dir: PathBuf,
}

impl Default for PlayOptions {
    fn default() -> Self {
        Self {
            width: crate::DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            theme: Theme::Auto,
            fps: DEFAULT_FPS,
            node: None,
            scratch_dir: std::env::temp_dir(),
        }
    }
}

/// One captured frame: the picture, when it was taken, and what had just happened.
#[derive(Debug, Clone)]
pub struct PlayFrame {
    /// Raw PNG bytes.
    pub png: Vec<u8>,
    /// Milliseconds since the session's first frame.
    pub at_ms: u64,
    /// The action this frame shows landing — `None` for a frame inside a wait.
    pub note: Option<String>,
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
}

/// Where an input lands: a block of the document, or a raw viewport coordinate.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Target {
    /// A block id; resolved to the block's on-page box when the action runs.
    Block(String),
    /// Viewport CSS pixels.
    Point(u32, u32),
}

/// One statement of the script.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Action {
    /// Let time pass, capturing frames at the session's rate.
    Wait(u64),
    /// Press and release one key.
    Key(String),
    /// Click the target's centre.
    Click(Target),
    /// Turn the wheel over the target (or the viewport centre); positive scrolls down.
    Scroll(Option<Target>, i32),
    /// Move the pointer onto the target.
    Hover(Target),
}

/// Perform `script` against the rendered `document` and return every captured frame.
///
/// # Errors
/// Returns a sentence when the script does not parse, exceeds the stated limits, names a
/// block the page does not have, or when no Chromium-family browser is installed.
pub fn play(
    document: &Document,
    script: &str,
    options: &PlayOptions,
) -> Result<Vec<PlayFrame>, String> {
    let actions = parse_script(script)?;
    let fps = options.fps.clamp(1, MAX_FPS);
    let interval = Duration::from_millis(1000 / u64::from(fps));
    check_budget(&actions, interval)?;

    let browser = crate::browser::find().ok_or_else(crate::browser::missing_message)?;
    let page = crate::page_html(document, options.theme);
    let workspace = crate::scratch_workspace(&options.scratch_dir)?;
    let result = drive(&browser, &page, &actions, options, interval, &workspace);
    let _ = std::fs::remove_dir_all(&workspace);
    result
}

/// Parse the script into actions, refusing the first statement that makes no sense.
fn parse_script(script: &str) -> Result<Vec<Action>, String> {
    let mut actions = Vec::new();
    for statement in script.split([';', '\n']) {
        let statement = statement.trim();
        if statement.is_empty() {
            continue;
        }
        actions.push(parse_statement(statement)?);
        if actions.len() > MAX_ACTIONS {
            return Err(format!("a script may carry at most {MAX_ACTIONS} actions"));
        }
    }
    if actions.is_empty() {
        return Err(
            "the script is empty — say what to do, e.g. \"wait 500ms; key Space; scroll 200\""
                .to_string(),
        );
    }
    Ok(actions)
}

/// Parse one `verb argument…` statement.
fn parse_statement(statement: &str) -> Result<Action, String> {
    let mut words = statement.split_whitespace();
    let verb = words.next().unwrap_or_default();
    let arguments: Vec<&str> = words.collect();
    match (verb, arguments.as_slice()) {
        ("wait", [duration]) => Ok(Action::Wait(parse_duration(duration)?)),
        ("key", [key]) => Ok(Action::Key((*key).to_string())),
        ("click", [target]) => Ok(Action::Click(parse_target(target))),
        ("hover", [target]) => Ok(Action::Hover(parse_target(target))),
        ("scroll", [amount]) if amount.parse::<i32>().is_ok() => Ok(Action::Scroll(
            None,
            amount.parse().expect("checked by the guard"),
        )),
        ("scroll", [target, amount]) => Ok(Action::Scroll(
            Some(parse_target(target)),
            amount
                .parse()
                .map_err(|_| format!("`{statement}`: `{amount}` is not a pixel amount"))?,
        )),
        _ => Err(format!(
            "cannot read `{statement}` — statements are `wait 500ms`, `key Space`, \
             `click <block-id|x,y>`, `scroll [target] <pixels>`, `hover <block-id|x,y>`, \
             separated by `;`"
        )),
    }
}

/// Parse `500ms`, `2s`, or a bare number of milliseconds.
fn parse_duration(text: &str) -> Result<u64, String> {
    let (digits, scale) = match text.strip_suffix("ms") {
        Some(digits) => (digits, 1),
        None => match text.strip_suffix('s') {
            Some(digits) => (digits, 1000),
            None => (text, 1),
        },
    };
    digits
        .parse::<u64>()
        .map(|value| value * scale)
        .map_err(|_| format!("`{text}` is not a duration — write `500ms` or `2s`"))
}

/// Parse a target word: `x,y` when it is two numbers, a block id otherwise.
fn parse_target(word: &str) -> Target {
    if let Some((x, y)) = word.split_once(',') {
        if let (Ok(x), Ok(y)) = (x.parse(), y.parse()) {
            return Target::Point(x, y);
        }
    }
    Target::Block(word.to_string())
}

/// Refuse a script whose waits or frame count exceed the stated limits.
fn check_budget(actions: &[Action], interval: Duration) -> Result<(), String> {
    let waiting: u64 = actions
        .iter()
        .map(|action| match action {
            Action::Wait(ms) => *ms,
            _ => 0,
        })
        .sum();
    if waiting > MAX_PLAY_MS {
        return Err(format!(
            "the script waits {waiting}ms in total; the most a play may take is {MAX_PLAY_MS}ms"
        ));
    }

    let interval_ms = interval.as_millis().max(1) as u64;
    let wait_frames: u64 = actions
        .iter()
        .map(|action| match action {
            Action::Wait(ms) => ms.div_ceil(interval_ms),
            _ => 1,
        })
        .sum();
    let planned = wait_frames as usize + 2; // the opening and closing frames
    if planned > MAX_FRAMES {
        return Err(format!(
            "this script would capture about {planned} frames; the most a play may keep is \
             {MAX_FRAMES} — lower --fps or shorten the waits"
        ));
    }
    Ok(())
}

/// The words a frame is annotated with when this action lands.
fn describe(action: &Action) -> String {
    let target = |target: &Target| match target {
        Target::Block(id) => id.clone(),
        Target::Point(x, y) => format!("{x},{y}"),
    };
    match action {
        Action::Wait(ms) => format!("wait {ms}ms"),
        Action::Key(key) => format!("key {key}"),
        Action::Click(at) => format!("click {}", target(at)),
        Action::Hover(at) => format!("hover {}", target(at)),
        Action::Scroll(None, amount) => format!("scroll {amount}"),
        Action::Scroll(Some(at), amount) => format!("scroll {} {amount}", target(at)),
    }
}

/// One key's DevTools identity.
struct KeySpec {
    key: String,
    code: &'static str,
    text: Option<String>,
    virtual_code: u32,
}

/// Map a script key name onto the event fields DevTools expects.
///
/// Named keys are matched case-insensitively; any single visible ASCII character is
/// itself a key.
fn key_spec(name: &str) -> Result<KeySpec, String> {
    let named = |key: &str, code, text: Option<&str>, virtual_code| KeySpec {
        key: key.to_string(),
        code,
        text: text.map(str::to_string),
        virtual_code,
    };
    match name.to_ascii_lowercase().as_str() {
        "space" => Ok(named(" ", "Space", Some(" "), 32)),
        "enter" | "return" => Ok(named("Enter", "Enter", Some("\r"), 13)),
        "tab" => Ok(named("Tab", "Tab", Some("\t"), 9)),
        "escape" | "esc" => Ok(named("Escape", "Escape", None, 27)),
        "backspace" => Ok(named("Backspace", "Backspace", None, 8)),
        "delete" => Ok(named("Delete", "Delete", None, 46)),
        "arrowup" | "up" => Ok(named("ArrowUp", "ArrowUp", None, 38)),
        "arrowdown" | "down" => Ok(named("ArrowDown", "ArrowDown", None, 40)),
        "arrowleft" | "left" => Ok(named("ArrowLeft", "ArrowLeft", None, 37)),
        "arrowright" | "right" => Ok(named("ArrowRight", "ArrowRight", None, 39)),
        "pageup" => Ok(named("PageUp", "PageUp", None, 33)),
        "pagedown" => Ok(named("PageDown", "PageDown", None, 34)),
        "home" => Ok(named("Home", "Home", None, 36)),
        "end" => Ok(named("End", "End", None, 35)),
        single if single.len() == 1 && name.len() == 1 => {
            let character = name.chars().next().expect("one character");
            if !character.is_ascii_graphic() {
                return Err(format!("`{name}` is not a key this script can press"));
            }
            Ok(KeySpec {
                key: name.to_string(),
                code: "",
                text: Some(name.to_string()),
                virtual_code: u32::from(character.to_ascii_uppercase() as u8),
            })
        }
        _ => Err(format!(
            "`{name}` is not a key this script knows — use Space, Enter, Tab, Escape, \
             Backspace, Delete, the arrows, PageUp/PageDown, Home, End, or one character"
        )),
    }
}

/// Run the session: load the page, walk the script, capture as it goes.
fn drive(
    browser: &std::path::Path,
    page: &str,
    actions: &[Action],
    options: &PlayOptions,
    interval: Duration,
    workspace: &std::path::Path,
) -> Result<Vec<PlayFrame>, String> {
    let page_file = workspace.join("play.html");
    crate::write(&page_file, page)?;
    let profile = workspace.join("profile");
    std::fs::create_dir_all(&profile)
        .map_err(|error| format!("could not create {}: {error}", profile.display()))?;

    let mut cdp = Cdp::launch(browser, &profile, options.width, options.height)?;
    cdp.open(&crate::file_url(&page_file))?;

    let clip = match options.node.as_deref() {
        Some(id) => Some(node_clip(&mut cdp, id)?),
        None => None,
    };

    let start = Instant::now();
    let mut frames = Vec::new();
    capture(&mut cdp, clip.as_ref(), start, Some("start"), &mut frames)?;

    for action in actions {
        match action {
            Action::Wait(ms) => {
                let end = Instant::now() + Duration::from_millis(*ms);
                while Instant::now() < end {
                    let nap = interval.min(end - Instant::now());
                    std::thread::sleep(nap);
                    capture(&mut cdp, clip.as_ref(), start, None, &mut frames)?;
                }
            }
            other => {
                perform(&mut cdp, other, options)?;
                capture(
                    &mut cdp,
                    clip.as_ref(),
                    start,
                    Some(&describe(other)),
                    &mut frames,
                )?;
            }
        }
    }

    capture(&mut cdp, clip.as_ref(), start, Some("end"), &mut frames)?;
    Ok(frames)
}

/// Dispatch one non-wait action as real input events.
fn perform(cdp: &mut Cdp, action: &Action, options: &PlayOptions) -> Result<(), String> {
    match action {
        Action::Wait(_) => Ok(()),
        Action::Key(name) => {
            let spec = key_spec(name)?;
            let mut down = json!({
                "type": if spec.text.is_some() { "keyDown" } else { "rawKeyDown" },
                "key": spec.key,
                "code": spec.code,
                "windowsVirtualKeyCode": spec.virtual_code,
                "nativeVirtualKeyCode": spec.virtual_code,
            });
            if let Some(text) = &spec.text {
                down["text"] = json!(text);
            }
            cdp.command("Input.dispatchKeyEvent", down)?;
            cdp.command(
                "Input.dispatchKeyEvent",
                json!({
                    "type": "keyUp",
                    "key": spec.key,
                    "code": spec.code,
                    "windowsVirtualKeyCode": spec.virtual_code,
                    "nativeVirtualKeyCode": spec.virtual_code,
                }),
            )?;
            Ok(())
        }
        Action::Click(target) => {
            let (x, y) = locate(cdp, target)?;
            for kind in ["mousePressed", "mouseReleased"] {
                cdp.command(
                    "Input.dispatchMouseEvent",
                    json!({
                        "type": kind, "x": x, "y": y,
                        "button": "left", "clickCount": 1,
                    }),
                )?;
            }
            Ok(())
        }
        Action::Hover(target) => {
            let (x, y) = locate(cdp, target)?;
            cdp.command(
                "Input.dispatchMouseEvent",
                json!({ "type": "mouseMoved", "x": x, "y": y }),
            )?;
            Ok(())
        }
        Action::Scroll(target, amount) => {
            let (x, y) = match target {
                Some(target) => locate(cdp, target)?,
                None => (
                    f64::from(options.width) / 2.0,
                    f64::from(options.height) / 2.0,
                ),
            };
            cdp.command(
                "Input.dispatchMouseEvent",
                json!({
                    "type": "mouseWheel", "x": x, "y": y,
                    "deltaX": 0, "deltaY": amount,
                }),
            )?;
            Ok(())
        }
    }
}

/// The JavaScript that finds a block's box, scrolled into view first so input can land.
///
/// Blocks are matched by walking `[data-block-id]` elements rather than by building a
/// selector, so an id needs no escaping to be found.
fn block_rect_js(id: &str) -> String {
    let id_json = serde_json::to_string(id).unwrap_or_else(|_| "\"\"".to_string());
    format!(
        "(function(){{var id={id_json};var found=null;\
         var all=document.querySelectorAll('[data-block-id]');\
         for(var i=0;i<all.length;i++){{\
         if(all[i].getAttribute('data-block-id')===id){{found=all[i];break;}}}}\
         if(!found)return null;\
         found.scrollIntoView({{block:'nearest'}});\
         var r=found.getBoundingClientRect();\
         return {{cx:r.left+r.width/2,cy:r.top+r.height/2,\
         x:r.left+window.scrollX,y:r.top+window.scrollY,w:r.width,h:r.height}};}})()"
    )
}

/// Resolve a target to viewport coordinates, naming the block when it is not on the page.
fn locate(cdp: &mut Cdp, target: &Target) -> Result<(f64, f64), String> {
    match target {
        Target::Point(x, y) => Ok((f64::from(*x), f64::from(*y))),
        Target::Block(id) => {
            let value = cdp.evaluate(&block_rect_js(id))?;
            if value.is_null() {
                return Err(format!("no block named `{id}` on the page"));
            }
            Ok((field(&value, "cx")?, field(&value, "cy")?))
        }
    }
}

/// The clip covering one block's box, in page coordinates.
fn node_clip(cdp: &mut Cdp, id: &str) -> Result<Clip, String> {
    let value = cdp.evaluate(&block_rect_js(id))?;
    if value.is_null() {
        return Err(format!("no block named `{id}` on the page"));
    }
    Ok(Clip {
        x: field(&value, "x")?,
        y: field(&value, "y")?,
        width: field(&value, "w")?.max(1.0),
        height: field(&value, "h")?.max(1.0),
    })
}

/// One numeric field of a measured box.
fn field(value: &Value, name: &str) -> Result<f64, String> {
    value[name]
        .as_f64()
        .ok_or_else(|| format!("the page's measurement carried no `{name}`"))
}

/// Capture one frame and stamp it with the moment and the note.
fn capture(
    cdp: &mut Cdp,
    clip: Option<&Clip>,
    start: Instant,
    note: Option<&str>,
    frames: &mut Vec<PlayFrame>,
) -> Result<(), String> {
    let png = cdp.screenshot(clip)?;
    let (width, height) = png_dimensions(&png).unwrap_or((0, 0));
    frames.push(PlayFrame {
        png,
        at_ms: start.elapsed().as_millis() as u64,
        note: note.map(str::to_string),
        width,
        height,
    });
    Ok(())
}

/// Read a PNG's pixel dimensions out of its IHDR header.
fn png_dimensions(png: &[u8]) -> Option<(u32, u32)> {
    if png.len() < 24 || &png[1..4] != b"PNG" {
        return None;
    }
    let width = u32::from_be_bytes(png[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(png[20..24].try_into().ok()?);
    Some((width, height))
}

#[cfg(test)]
mod tests {
    use super::*;
    use doc_core::format::parse;

    #[test]
    fn a_script_parses_into_its_actions_in_order() {
        let actions = parse_script("wait 500ms; key Space; click steps; scroll 200;\nhover 10,20")
            .expect("parse");
        assert_eq!(
            actions,
            vec![
                Action::Wait(500),
                Action::Key("Space".to_string()),
                Action::Click(Target::Block("steps".to_string())),
                Action::Scroll(None, 200),
                Action::Hover(Target::Point(10, 20)),
            ]
        );
    }

    #[test]
    fn a_scroll_may_name_the_node_whose_overflow_it_turns() {
        let actions = parse_script("scroll steps 120; scroll -80").expect("parse");
        assert_eq!(
            actions,
            vec![
                Action::Scroll(Some(Target::Block("steps".to_string())), 120),
                Action::Scroll(None, -80),
            ]
        );
    }

    #[test]
    fn durations_read_naturally_in_either_unit() {
        assert_eq!(parse_duration("500ms").expect("ms"), 500);
        assert_eq!(parse_duration("2s").expect("s"), 2000);
        assert_eq!(parse_duration("750").expect("bare"), 750);
        assert!(parse_duration("soon").is_err());
    }

    #[test]
    fn a_statement_that_makes_no_sense_is_named_with_the_vocabulary() {
        let error = parse_script("wait 500ms; jump high").expect_err("should refuse");
        assert!(error.contains("jump high"), "{error}");
        assert!(error.contains("wait 500ms"), "{error}");
    }

    #[test]
    fn an_empty_script_is_refused_with_an_example() {
        let error = parse_script("  ;;  \n ").expect_err("should refuse");
        assert!(error.contains("key Space"), "{error}");
    }

    #[test]
    fn the_stated_limits_are_enforced_before_any_browser_starts() {
        let long = vec!["key a"; MAX_ACTIONS + 1].join(";");
        assert!(parse_script(&long)
            .expect_err("too many")
            .contains("at most"));

        let patient = parse_script("wait 31s").expect("parse");
        let interval = Duration::from_millis(100);
        assert!(check_budget(&patient, interval)
            .expect_err("too long")
            .contains("30000ms"));

        let dense = parse_script("wait 30s").expect("parse");
        let too_fast = Duration::from_millis(33);
        let error = check_budget(&dense, too_fast).expect_err("too many frames");
        assert!(error.contains("--fps"), "{error}");
        assert!(check_budget(&dense, Duration::from_millis(200)).is_ok());
    }

    #[test]
    fn named_keys_and_characters_map_to_real_key_events() {
        let space = key_spec("Space").expect("space");
        assert_eq!(space.key, " ");
        assert_eq!(space.virtual_code, 32);
        assert_eq!(space.text.as_deref(), Some(" "));

        let escape = key_spec("esc").expect("escape");
        assert_eq!(escape.key, "Escape");
        assert!(escape.text.is_none());

        let letter = key_spec("j").expect("letter");
        assert_eq!(letter.virtual_code, u32::from(b'J'));
        assert_eq!(letter.text.as_deref(), Some("j"));

        assert!(key_spec("Hyperspace").is_err());
    }

    #[test]
    fn every_action_describes_itself_in_its_own_words() {
        for (script, note) in [
            ("key Space", "key Space"),
            ("click steps", "click steps"),
            ("click 12,34", "click 12,34"),
            ("scroll 200", "scroll 200"),
            ("scroll steps -60", "scroll steps -60"),
            ("hover intro", "hover intro"),
        ] {
            let actions = parse_script(script).expect(script);
            assert_eq!(describe(&actions[0]), note);
        }
    }

    #[test]
    fn png_dimensions_come_out_of_the_header() {
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        png.extend_from_slice(&[0, 0, 0, 13]);
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&640u32.to_be_bytes());
        png.extend_from_slice(&480u32.to_be_bytes());
        assert_eq!(png_dimensions(&png), Some((640, 480)));
        assert_eq!(png_dimensions(b"not a png"), None);
    }

    #[test]
    fn a_block_id_needing_escapes_still_finds_its_element() {
        let script = block_rect_js("weird\"id'");
        assert!(script.contains("\\\"id'"), "{script}");
        assert!(script.contains("data-block-id"), "{script}");
    }

    /// The full pipeline against a real browser: load, wait, scroll, key, capture.
    /// Skipped silently on a machine with no Chromium, like every capture test.
    #[test]
    fn a_real_page_is_driven_and_returns_annotated_frames() {
        let _turn = crate::browser::ENV_LOCK.lock().expect("env lock");
        if crate::browser::find().is_none() {
            return;
        }
        let long = (1..=30)
            .map(|n| format!("::paragraph id=p{n}\nParagraph number {n}.\n::end\n"))
            .collect::<Vec<_>>()
            .join("\n");
        let document = parse(&long);
        let frames = play(
            &document,
            "wait 250ms; scroll 400; key PageDown; click p1",
            &PlayOptions {
                fps: 10,
                ..PlayOptions::default()
            },
        )
        .expect("play");

        assert!(frames.len() >= 5, "start, waits, three actions, end");
        for frame in &frames {
            assert_eq!(&frame.png[1..4], b"PNG");
            assert!(frame.width > 0 && frame.height > 0);
        }
        let times: Vec<u64> = frames.iter().map(|frame| frame.at_ms).collect();
        assert!(times.windows(2).all(|pair| pair[0] <= pair[1]));
        let notes: Vec<&str> = frames.iter().filter_map(|f| f.note.as_deref()).collect();
        assert_eq!(notes.first(), Some(&"start"));
        assert_eq!(notes.last(), Some(&"end"));
        assert!(notes.contains(&"scroll 400"), "{notes:?}");
        assert!(notes.contains(&"key PageDown"), "{notes:?}");
        assert!(notes.contains(&"click p1"), "{notes:?}");
    }

    /// `node` clips every frame to one block's box — the numbers an agent computes from.
    #[test]
    fn a_node_clip_frames_only_that_block() {
        let _turn = crate::browser::ENV_LOCK.lock().expect("env lock");
        if crate::browser::find().is_none() {
            return;
        }
        let document = parse(
            "::paragraph id=intro\nA one-line paragraph.\n::end\n\n\
             ::paragraph id=next\nAnother.\n::end\n",
        );
        let options = PlayOptions {
            node: Some("intro".to_string()),
            ..PlayOptions::default()
        };
        let frames = play(&document, "wait 100ms", &options).expect("play");
        let first = frames.first().expect("a frame");
        assert!(
            first.width < options.width && first.height < DEFAULT_HEIGHT,
            "a clipped frame ({}x{}) must be smaller than the viewport",
            first.width,
            first.height
        );
        let missing = play(
            &document,
            "wait 100ms",
            &PlayOptions {
                node: Some("ghost".to_string()),
                ..PlayOptions::default()
            },
        );
        assert!(missing.expect_err("no such block").contains("ghost"));
    }
}
