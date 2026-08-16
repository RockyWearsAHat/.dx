//! A live DevTools session with the same browser the screenshots use.
//!
//! [`crate::capture`] drives Chromium as one-shot `--screenshot` invocations, which can
//! photograph a page but cannot touch it. This module opens the browser's own remote
//! debugging channel instead — the Chrome DevTools Protocol, spoken as JSON over a
//! WebSocket — so a caller can navigate, dispatch real keyboard and mouse events, ask the
//! page questions, and capture frame after frame from one living page. It is what
//! [`crate::play`] stands on.
//!
//! The WebSocket client is written here in full (RFC 6455: one HTTP upgrade, then masked
//! frames) rather than taken as a dependency: the peer is always a Chromium this process
//! just launched on loopback, which needs none of a general client's negotiation.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::base64;

/// How long to wait for the launched browser to announce its DevTools address.
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(15);

/// How long one protocol command may take before the session gives up on it.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(15);

/// How long a screenshot may take: its work scales with the page, and rasterizing plus
/// PNG-encoding a full-height document at capture density is honest minutes of work the
/// flat command timeout was reading as a dead browser.
const SCREENSHOT_TIMEOUT: Duration = Duration::from_secs(120);

/// How long to wait for a page's load event before proceeding without it.
const LOAD_TIMEOUT: Duration = Duration::from_secs(10);

/// Largest protocol message accepted, bounding what a frame's declared length can make
/// this process allocate. A full-page PNG arrives base64-encoded well under this.
const MAX_MESSAGE_BYTES: usize = 64 * 1024 * 1024;

/// Protocol events kept for [`Cdp::wait_for_event`]; older ones are dropped oldest-first.
const MAX_HELD_EVENTS: usize = 64;

/// A region of the page to capture, in CSS pixels from the document's top-left corner.
#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    /// Distance from the document's left edge.
    pub x: f64,
    /// Distance from the document's top edge.
    pub y: f64,
    /// Width of the region.
    pub width: f64,
    /// Height of the region.
    pub height: f64,
}

/// One live browser: the process, the DevTools socket, and the attached page session.
///
/// Dropping the session kills the browser it launched.
pub struct Cdp {
    child: Child,
    socket: TcpStream,
    next_id: u64,
    /// The attached page's session id; `None` until [`Cdp::open`] attaches one.
    session: Option<String>,
    /// The open page's target id, closed when [`Cdp::open`] replaces it — a session that
    /// opens page after page must not leave every one of them alive in the browser.
    target: Option<String>,
    /// Events received while waiting for command responses, oldest first.
    events: VecDeque<Value>,
}

impl Drop for Cdp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Cdp {
    /// Launch `browser` headless with remote debugging on a kernel-chosen port and connect.
    ///
    /// `profile_dir` must be a scratch directory this session may own: Chromium refuses
    /// remote debugging on a default profile, and an owned profile keeps the session away
    /// from the reader's real browsing state.
    ///
    /// `allow_host` is `None` for every reading capture (`dx_read`, `dx_play`, `dx png`):
    /// the browser resolves no hostname at all, ever — a capture is a read, and a document
    /// must not reach the network by being looked at. A `::view` naming an external
    /// stylesheet was both a phone-home channel and a hang: the render-blocking fetch never
    /// completed in this session, so the sandboxed frame never painted and shipped as empty
    /// paper. NXDOMAIN fails the fetch instantly; the page paints with what it carries.
    /// `file://` needs no resolution and is untouched either way.
    ///
    /// `doc-run`'s live-capture runner is the one caller that passes `Some(host)`: a
    /// document names a `target=` on purpose, so the resolver maps exactly that one
    /// hostname and still refuses every other. This alone does *not* scope an IP literal
    /// target (Chromium never consults the host resolver for one), which is what `proxy`
    /// is for: when set, every connection — hostname or literal alike — is forced through
    /// a local proxy that opens a socket only to the one destination it was told to allow,
    /// checked against the literal destination requested rather than anything DNS-shaped.
    /// `doc-run` is the proxy's own implementation and its authority on why a resolver rule
    /// alone was not enough.
    ///
    /// # Errors
    /// Returns a sentence naming what failed — launch, announcement, or handshake.
    pub fn launch(
        browser: &Path,
        profile_dir: &Path,
        width: u32,
        height: u32,
        allow_host: Option<&str>,
        proxy: Option<u16>,
    ) -> Result<Self, String> {
        let resolver_rules = match allow_host {
            Some(host) => format!("MAP {host} {host}, MAP * ~NOTFOUND"),
            None => "MAP * ~NOTFOUND".to_string(),
        };
        let mut command = Command::new(browser);
        command.args([
            "--headless",
            "--disable-gpu",
            "--no-sandbox",
            "--hide-scrollbars",
            "--no-first-run",
            "--disable-extensions",
            "--force-device-scale-factor=1",
            "--remote-debugging-port=0",
        ]);
        command.arg(format!("--host-resolver-rules={resolver_rules}"));
        if let Some(port) = proxy {
            command.arg(format!("--proxy-server=127.0.0.1:{port}"));
            // Chromium bypasses any configured proxy for loopback destinations by
            // default — exactly the case this proxy exists to scope. Negating it forces
            // every destination, loopback included, through the proxy.
            command.arg("--proxy-bypass-list=<-loopback>");
        }
        let mut child = command
            .arg(format!("--user-data-dir={}", profile_dir.display()))
            .arg(format!("--window-size={width},{height}"))
            .arg("about:blank")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("could not start the browser: {error}"))?;

        let Some(stderr) = child.stderr.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err("the browser gave no channel to announce DevTools on".to_string());
        };
        let url = match devtools_url(stderr) {
            Ok(url) => url,
            Err(reason) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "the browser did not open its DevTools channel: {reason}"
                ));
            }
        };

        match connect(&url) {
            Ok(socket) => Ok(Self {
                child,
                socket,
                next_id: 0,
                session: None,
                target: None,
                events: VecDeque::new(),
            }),
            Err(reason) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(reason)
            }
        }
    }

    /// Open `url` in a fresh page and attach this session to it.
    ///
    /// Waits for the page's load event (best-effort — a page that never fires one still
    /// becomes the session's page once [`LOAD_TIMEOUT`] passes).
    ///
    /// # Errors
    /// Returns a sentence when the browser cannot create or attach the page.
    pub fn open(&mut self, url: &str) -> Result<(), String> {
        self.attach()?;
        self.command("Page.navigate", json!({ "url": url }))?;
        self.wait_for_event("Page.loadEventFired", LOAD_TIMEOUT);
        Ok(())
    }

    /// Attach this session to a fresh `about:blank` page, closing the previous one.
    fn attach(&mut self) -> Result<(), String> {
        if let Some(previous) = self.target.take() {
            // Best-effort: a page that will not close must not stop the next one opening.
            let _ = self.raw_command("Target.closeTarget", json!({ "targetId": previous }));
            self.session = None;
        }
        let created = self.raw_command("Target.createTarget", json!({ "url": "about:blank" }))?;
        let target = created["targetId"]
            .as_str()
            .ok_or("the browser created no page")?
            .to_string();
        self.target = Some(target.clone());
        let attached = self.raw_command(
            "Target.attachToTarget",
            json!({ "targetId": target, "flatten": true }),
        )?;
        self.session = Some(
            attached["sessionId"]
                .as_str()
                .ok_or("the browser attached no session")?
                .to_string(),
        );
        self.command("Page.enable", json!({}))?;
        Ok(())
    }

    /// Open `url` and give the page `grace_ms` of real time to become itself before it
    /// is measured or photographed.
    ///
    /// # Errors
    /// Returns a sentence when the target cannot be opened.
    pub fn open_settled(&mut self, url: &str, grace_ms: u32) -> Result<(), String> {
        self.open(url)?;
        // Real time, deliberately: virtual time froze a `::view`'s sandboxed frame at
        // every stage it was tried (granted after load, the frame's tasks queue to an
        // expired timeline; granted before navigation, the capture hangs or ships
        // black). The pages this crate loads are local files — a settling grace of real
        // milliseconds is honest and short, and `stable_screenshot` holds the page to
        // its own definition of settled on top of it.
        std::thread::sleep(Duration::from_millis(u64::from(grace_ms)));
        Ok(())
    }

    /// Let the live page finish becoming itself after a change that needs more time —
    /// a scroll of a paginated capture, a viewport resize.
    ///
    /// Real milliseconds, deliberately: virtual time froze a `::view`'s sandboxed frame
    /// at every stage it was tried — granted after load, the frame's tasks queue to an
    /// already-expired timeline and it never paints; granted before navigation, the
    /// capture hangs on a paused renderer or ships black. The pages this crate loads
    /// are local files with no network, so a short real grace is honest, and
    /// `stable_screenshot` holds the page to its own definition of settled on top.
    pub fn settle(&mut self, grace_ms: u32) {
        std::thread::sleep(Duration::from_millis(u64::from(grace_ms)));
    }

    /// Send one protocol command to the attached page and return its `result`.
    ///
    /// # Errors
    /// Returns the protocol error's message when the browser refuses the command, or a
    /// sentence when the connection fails.
    pub fn command(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.command_within(method, params, COMMAND_TIMEOUT)
    }

    /// [`Self::command`] with its own deadline, for the commands whose work scales with
    /// the page: capturing a twelve-thousand-pixel document rasterizes and PNG-encodes
    /// tens of megapixels, which is minutes of honest work the flat command timeout was
    /// reading as a dead browser.
    fn command_within(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, String> {
        let session = self.session.clone();
        self.dispatch(method, params, session.as_deref(), timeout)
    }

    /// Evaluate a JavaScript expression on the page and return its value.
    ///
    /// # Errors
    /// Returns the thrown exception's description when the expression throws.
    pub fn evaluate(&mut self, expression: &str) -> Result<Value, String> {
        self.evaluate_with(expression, false)
    }

    /// [`Self::evaluate`], but awaits a returned promise before answering — for
    /// [`crate::capture`]'s state-loading scripts, which are routinely asynchronous
    /// (`await fetch(...)`, poll until a condition holds, wait for a login round trip).
    ///
    /// # Errors
    /// Returns the thrown exception's description when the expression throws, or the
    /// rejection's description when its promise rejects.
    pub fn evaluate_async(&mut self, expression: &str) -> Result<Value, String> {
        self.evaluate_with(expression, true)
    }

    /// The shared body of [`Self::evaluate`] and [`Self::evaluate_async`].
    fn evaluate_with(&mut self, expression: &str, await_promise: bool) -> Result<Value, String> {
        let result = self.command(
            "Runtime.evaluate",
            json!({
                "expression": expression,
                "returnByValue": true,
                "awaitPromise": await_promise,
            }),
        )?;
        if let Some(details) = result.get("exceptionDetails") {
            return Err(format!(
                "the page rejected the question: {}",
                details["exception"]["description"]
                    .as_str()
                    .unwrap_or("unknown error")
            ));
        }
        Ok(result["result"]["value"].clone())
    }

    /// Capture the page — or just `clip` of it — as PNG bytes.
    ///
    /// # Errors
    /// Returns a sentence when the browser produces no image.
    pub fn screenshot(&mut self, clip: Option<&Clip>) -> Result<Vec<u8>, String> {
        let mut params = json!({ "format": "png" });
        if let Some(clip) = clip {
            params["clip"] = json!({
                "x": clip.x, "y": clip.y,
                "width": clip.width, "height": clip.height,
                "scale": 1,
            });
            params["captureBeyondViewport"] = json!(true);
        }
        let result = self.command_within("Page.captureScreenshot", params, SCREENSHOT_TIMEOUT)?;
        base64::decode(
            result["data"]
                .as_str()
                .ok_or("the browser produced no image")?,
        )
    }

    /// Send a command carrying the given session (or none, for browser-level commands).
    fn dispatch(
        &mut self,
        method: &str,
        params: Value,
        session: Option<&str>,
        timeout: Duration,
    ) -> Result<Value, String> {
        self.next_id += 1;
        let id = self.next_id;
        let mut message = json!({ "id": id, "method": method, "params": params });
        if let Some(session) = session {
            message["sessionId"] = json!(session);
        }
        self.send_text(&message.to_string())?;

        let deadline = Instant::now() + timeout;
        loop {
            let text = self
                .receive_message(deadline)
                .map_err(|reason| format!("no answer to {method} from the browser: {reason}"))?;
            let value: Value = serde_json::from_str(&text)
                .map_err(|error| format!("the browser sent malformed JSON: {error}"))?;
            if value["id"].as_u64() == Some(id) {
                if let Some(error) = value.get("error") {
                    return Err(format!(
                        "{method} failed: {}",
                        error["message"].as_str().unwrap_or("unknown error")
                    ));
                }
                return Ok(value["result"].clone());
            }
            self.hold_event(value);
        }
    }

    /// A command with no session — the browser-level half of the protocol.
    fn raw_command(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.dispatch(method, params, None, COMMAND_TIMEOUT)
    }

    /// Wait until the named event arrives or the timeout passes; either way, proceed.
    ///
    /// Best-effort by design: the events this session waits for (a local file's load) are
    /// moments to synchronize on, not results — a page that never announces one is late,
    /// not failed.
    fn wait_for_event(&mut self, method: &str, timeout: Duration) {
        if self.take_event(method) {
            return;
        }
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let Ok(text) = self.receive_message(deadline) else {
                return;
            };
            let Ok(value) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            if value["method"].as_str() == Some(method) {
                return;
            }
            self.hold_event(value);
        }
    }

    /// Remove the first held event with this method, saying whether one was there.
    fn take_event(&mut self, method: &str) -> bool {
        let position = self
            .events
            .iter()
            .position(|event| event["method"].as_str() == Some(method));
        match position {
            Some(index) => {
                self.events.remove(index);
                true
            }
            None => false,
        }
    }

    /// Keep an out-of-turn message for a later [`Cdp::wait_for_event`], bounded.
    fn hold_event(&mut self, event: Value) {
        if self.events.len() >= MAX_HELD_EVENTS {
            self.events.pop_front();
        }
        self.events.push_back(event);
    }

    /// Send one masked WebSocket text frame.
    fn send_text(&mut self, text: &str) -> Result<(), String> {
        let frame = encode_frame(0x1, text.as_bytes());
        self.socket
            .write_all(&frame)
            .map_err(|error| format!("the DevTools connection dropped: {error}"))
    }

    /// Receive the next complete text message, answering pings along the way.
    fn receive_message(&mut self, deadline: Instant) -> Result<String, String> {
        let mut message: Vec<u8> = Vec::new();
        loop {
            let (fin, opcode, payload) = self.read_frame(deadline)?;
            match opcode {
                // Text, binary, or a continuation of either: one message, in pieces.
                0x0..=0x2 => {
                    if message.len() + payload.len() > MAX_MESSAGE_BYTES {
                        return Err("the browser sent an oversized message".to_string());
                    }
                    message.extend_from_slice(&payload);
                    if fin {
                        return String::from_utf8(message)
                            .map_err(|_| "the browser sent a non-UTF-8 message".to_string());
                    }
                }
                0x8 => return Err("the browser closed the connection".to_string()),
                0x9 => {
                    let pong = encode_frame(0xA, &payload);
                    self.socket
                        .write_all(&pong)
                        .map_err(|error| format!("the DevTools connection dropped: {error}"))?;
                }
                0xA => {} // A pong nobody asked for is legal and ignorable.
                other => return Err(format!("unsupported WebSocket opcode {other}")),
            }
        }
    }

    /// Read one frame — `(fin, opcode, payload)` — before `deadline`.
    fn read_frame(&mut self, deadline: Instant) -> Result<(bool, u8, Vec<u8>), String> {
        let mut header = [0u8; 2];
        self.read_exact_by(deadline, &mut header)?;
        let fin = header[0] & 0x80 != 0;
        let opcode = header[0] & 0x0F;
        let masked = header[1] & 0x80 != 0;

        let length = match header[1] & 0x7F {
            126 => {
                let mut extended = [0u8; 2];
                self.read_exact_by(deadline, &mut extended)?;
                usize::from(u16::from_be_bytes(extended))
            }
            127 => {
                let mut extended = [0u8; 8];
                self.read_exact_by(deadline, &mut extended)?;
                usize::try_from(u64::from_be_bytes(extended))
                    .map_err(|_| "the browser declared an absurd frame length".to_string())?
            }
            small => usize::from(small),
        };
        if length > MAX_MESSAGE_BYTES {
            return Err("the browser declared an oversized frame".to_string());
        }

        let mut mask = [0u8; 4];
        if masked {
            self.read_exact_by(deadline, &mut mask)?;
        }
        let mut payload = vec![0u8; length];
        self.read_exact_by(deadline, &mut payload)?;
        if masked {
            for (index, byte) in payload.iter_mut().enumerate() {
                *byte ^= mask[index % 4];
            }
        }
        Ok((fin, opcode, payload))
    }

    /// Fill `buffer` from the socket, retrying short reads until `deadline`.
    fn read_exact_by(&mut self, deadline: Instant, buffer: &mut [u8]) -> Result<(), String> {
        let mut filled = 0;
        while filled < buffer.len() {
            if Instant::now() >= deadline {
                return Err("timed out".to_string());
            }
            match self.socket.read(&mut buffer[filled..]) {
                Ok(0) => return Err("the browser closed the connection".to_string()),
                Ok(count) => filled += count,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        || error.kind() == std::io::ErrorKind::TimedOut =>
                {
                    continue;
                }
                Err(error) => return Err(format!("read failed: {error}")),
            }
        }
        Ok(())
    }
}

/// Encode one client WebSocket frame: FIN set, masked, as RFC 6455 requires of clients.
///
/// The mask is a constant. Masking exists to keep intermediaries from misreading frames
/// as other protocols; on a loopback socket to a browser this process launched there are
/// no intermediaries, and the spec only requires that a mask be present.
fn encode_frame(opcode: u8, payload: &[u8]) -> Vec<u8> {
    const MASK: [u8; 4] = [0x64, 0x78, 0x21, 0x3f];
    let mut frame = Vec::with_capacity(payload.len() + 14);
    frame.push(0x80 | opcode);
    if payload.len() < 126 {
        frame.push(0x80 | payload.len() as u8);
    } else if payload.len() <= usize::from(u16::MAX) {
        frame.push(0x80 | 126);
        frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    } else {
        frame.push(0x80 | 127);
        frame.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    }
    frame.extend_from_slice(&MASK);
    frame.extend(
        payload
            .iter()
            .enumerate()
            .map(|(index, byte)| byte ^ MASK[index % 4]),
    );
    frame
}

/// Read the browser's stderr until it announces `DevTools listening on ws://…`.
fn devtools_url(stderr: std::process::ChildStderr) -> Result<String, String> {
    use std::io::BufRead;
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let reader = std::io::BufReader::new(stderr);
        for line in reader.lines() {
            let Ok(line) = line else { break };
            if let Some(rest) = line.trim().strip_prefix("DevTools listening on ") {
                let _ = sender.send(rest.to_string());
                break;
            }
        }
        // Sender drops here; a browser that exits without announcing unblocks the wait.
    });
    receiver
        .recv_timeout(LAUNCH_TIMEOUT)
        .map_err(|_| "it never announced a listening address".to_string())
}

/// Split a `ws://host:port/path` URL into its address and path.
fn ws_address(url: &str) -> Result<(String, String), String> {
    let rest = url
        .strip_prefix("ws://")
        .ok_or_else(|| format!("not a ws:// URL: {url}"))?;
    let (address, path) = rest
        .split_once('/')
        .ok_or_else(|| format!("no path in {url}"))?;
    Ok((address.to_string(), format!("/{path}")))
}

/// Connect to the announced DevTools URL and complete the WebSocket handshake.
fn connect(url: &str) -> Result<TcpStream, String> {
    let (address, path) = ws_address(url)?;
    let mut socket = TcpStream::connect(&address)
        .map_err(|error| format!("could not reach the browser at {address}: {error}"))?;
    socket
        .set_read_timeout(Some(Duration::from_millis(250)))
        .map_err(|error| format!("could not configure the socket: {error}"))?;

    // Any 16-byte value serves as the key; its purpose is proving the peer speaks
    // WebSocket, which checking for the 101 below already does.
    let key = base64::encode(b"sixteen byte key");
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {address}\r\nUpgrade: websocket\r\n\
         Connection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
    );
    socket
        .write_all(request.as_bytes())
        .map_err(|error| format!("handshake failed: {error}"))?;

    let deadline = Instant::now() + COMMAND_TIMEOUT;
    let mut response = Vec::new();
    let mut byte = [0u8; 1];
    while !response.ends_with(b"\r\n\r\n") {
        if Instant::now() >= deadline || response.len() > 16 * 1024 {
            return Err("the browser did not complete the WebSocket handshake".to_string());
        }
        match socket.read(&mut byte) {
            Ok(1) => response.push(byte[0]),
            Ok(_) => return Err("the browser closed the connection mid-handshake".to_string()),
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.kind() == std::io::ErrorKind::TimedOut =>
            {
                continue;
            }
            Err(error) => return Err(format!("handshake failed: {error}")),
        }
    }
    if !response.starts_with(b"HTTP/1.1 101") {
        let head = String::from_utf8_lossy(&response);
        let first = head.lines().next().unwrap_or("no response");
        return Err(format!(
            "the browser refused the WebSocket upgrade: {first}"
        ));
    }
    Ok(socket)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_frame_is_masked_and_carries_its_length_inline() {
        let frame = encode_frame(0x1, b"hi");
        assert_eq!(frame[0], 0x81, "FIN + text opcode");
        assert_eq!(frame[1], 0x80 | 2, "mask bit + length");
        assert_eq!(frame.len(), 2 + 4 + 2, "header, mask, payload");
        // Unmasking the payload with the mask bytes restores the text.
        let mask = &frame[2..6];
        let restored: Vec<u8> = frame[6..]
            .iter()
            .enumerate()
            .map(|(index, byte)| byte ^ mask[index % 4])
            .collect();
        assert_eq!(restored, b"hi");
    }

    #[test]
    fn a_medium_frame_uses_the_two_byte_length_form() {
        let payload = vec![0u8; 300];
        let frame = encode_frame(0x1, &payload);
        assert_eq!(frame[1], 0x80 | 126);
        assert_eq!(u16::from_be_bytes([frame[2], frame[3]]), 300);
    }

    #[test]
    fn a_large_frame_uses_the_eight_byte_length_form() {
        let payload = vec![0u8; 70_000];
        let frame = encode_frame(0x1, &payload);
        assert_eq!(frame[1], 0x80 | 127);
        let mut length = [0u8; 8];
        length.copy_from_slice(&frame[2..10]);
        assert_eq!(u64::from_be_bytes(length), 70_000);
    }

    #[test]
    fn the_announced_url_splits_into_address_and_path() {
        let (address, path) =
            ws_address("ws://127.0.0.1:9222/devtools/browser/abc-123").expect("split");
        assert_eq!(address, "127.0.0.1:9222");
        assert_eq!(path, "/devtools/browser/abc-123");
    }

    #[test]
    fn a_non_websocket_url_is_refused_with_the_url_named() {
        let error = ws_address("http://127.0.0.1:9222/json").expect_err("should refuse");
        assert!(error.contains("http://127.0.0.1:9222/json"), "{error}");
    }
}
