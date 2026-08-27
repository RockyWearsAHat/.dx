//! The one runner that reaches a live target on purpose.
//!
//! Every other runner in [`crate::plan`] promises the block's own code sees no network at
//! all — [`crate::confine`]'s module doc is the authority on why. `lang=capture` is the
//! deliberate, narrow exception, and it does not weaken that promise: there is no
//! interpreter here, no subprocess running the block's own code, so there is nothing for a
//! filesystem/process boundary to contain in the first place. A capture block names one
//! `target=` URL and, optionally, a `setup=` command; dx's own trusted browser driver
//! ([`doc_shot::cdp::Cdp`]) is what reaches the network, and only ever to that one host —
//! [`doc_shot::cdp::Cdp::launch`]'s `allow_host` maps exactly `target`'s hostname and
//! refuses every other, the same way every other capture in this codebase refuses all of
//! them. The block's body is JavaScript, evaluated in the live page once it settles — the
//! way a capture is driven into a particular state (fill a form, call the app's own API,
//! wait for a condition) rather than only ever showing whatever the target renders on load.
//!
//! `setup=`, when a block states one, gets the same trust as every other runner's own
//! dependency-install phase ([`crate::plan`]'s module doc): unconfined, with the network,
//! because it exists to prepare the target — most often, to start the dev server the
//! capture is about to open — and that needs the network too. It is killed, best-effort,
//! the moment the capture is done, so a forgotten dev server never outlives the block that
//! started it.
//!
//! Approval still gates every capture exactly like every other runnable block: `target=`
//! and `setup=` are folded into the fingerprint and the approval material in `lib.rs`
//! (`approval_material`), so a reviewer approves *where this reaches and what it runs to
//! get there*, not just the script text — and editing either re-opens review the same way
//! editing the code would.

use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use doc_core::model::Block;
use doc_shot::{browser, cdp::Cdp};

use crate::live_actions;
use crate::live_proxy;
use crate::process::Capture;

/// Browser viewport when a capture block states no `width=`/`height=`.
const DEFAULT_WIDTH: u32 = 1280;
const DEFAULT_HEIGHT: u32 = 800;

/// Real time given the page to settle after navigation, before its script (if any) runs.
const SETTLE_MS: u32 = 300;

/// How long `setup=`, if given, gets to make its target's port accept a connection before
/// capture proceeds regardless — a target still unreachable after this is surfaced by the
/// navigation that follows failing, not by hanging here forever.
const SETUP_GRACE: Duration = Duration::from_secs(20);

/// Defines `__dxAction`, the helper every compiled [`live_actions`] verb calls: find the
/// one element `selector` names, scroll it into view, and run `fn` against it — refusing
/// with a clear sentence rather than a bare `null` deref when nothing matches. Defined
/// ahead of every capture script (`actions` or raw JavaScript alike), so raw JavaScript
/// may use it too.
const DX_ACTION_HELPER: &str = "async function __dxAction(selector, fn) {\n\
     const el = document.querySelector(selector);\n\
     if (!el) throw new Error('no element matches ' + JSON.stringify(selector));\n\
     el.scrollIntoView({block: 'center', inline: 'center'});\n\
     return await fn(el);\n\
     }";

/// Run a `lang=capture` block: launch dx's own browser, optionally prepare `target=` with
/// `setup=`, open it, evaluate the block's script, screenshot, and save the picture into
/// the first of `granted`'s directories.
pub(crate) fn execute(
    block: &Block,
    granted: &[PathBuf],
    document_dir: &Path,
    timeout: Duration,
) -> Capture {
    let Some(out_dir) = granted.first() else {
        return blocked(
            "a `capture` block needs `writes=` naming the folder its screenshot is saved \
             into — dx does not choose one for it",
        );
    };
    let target = block.target.trim();
    if target.is_empty() {
        return blocked("a `capture` block needs `target=<url>` naming what to open");
    }
    let authority = match target_authority(target) {
        Ok(authority) => authority,
        Err(reason) => return blocked(&reason),
    };
    let Some(browser_path) = browser::find() else {
        return blocked(&browser::missing_message());
    };

    let deadline = Instant::now() + timeout;
    let mut setup = None;
    if !block.setup.trim().is_empty() {
        match spawn_setup(&block.setup, document_dir) {
            Ok(child) => {
                wait_reachable(&authority, deadline.min(Instant::now() + SETUP_GRACE));
                setup = Some(child);
            }
            Err(reason) => return blocked(&reason),
        }
    }

    let result = capture(&browser_path, &authority, target, block, out_dir);

    if let Some(mut child) = setup {
        kill_tree(&mut child);
    }

    match result {
        Ok((path, note)) => Capture {
            output: format!("captured {target} to {}\n{note}", path.display()),
            exit: 0,
            timed_out: false,
        },
        Err(reason) => Capture {
            output: reason,
            exit: 1,
            timed_out: false,
        },
    }
}

/// The launch → open → script → screenshot → save sequence, isolated so [`execute`] can
/// still kill `setup` on every exit path, success or failure alike.
fn capture(
    browser_path: &Path,
    authority: &Authority,
    target: &str,
    block: &Block,
    out_dir: &Path,
) -> Result<(PathBuf, String), String> {
    let profile = std::env::temp_dir().join(format!(
        "dx-capture-{}-{}",
        std::process::id(),
        block.id.replace(['/', '\\'], "_")
    ));
    std::fs::create_dir_all(&profile)
        .map_err(|error| format!("could not prepare a browser profile: {error}"))?;
    // Compiled — and any mistake in it reported — before a browser is even launched.
    let script = if block.actions {
        let script = block.text.trim();
        if script.is_empty() {
            String::new()
        } else {
            live_actions::compile(script)
                .map_err(|reason| format!("the `actions` script is invalid: {reason}"))?
        }
    } else {
        block.text.trim().to_string()
    };
    let width = if block.width > 0 {
        block.width
    } else {
        DEFAULT_WIDTH
    };
    let height = if block.height > 0 {
        block.height
    } else {
        DEFAULT_HEIGHT
    };

    // Scoping network reach to exactly `target` needs both halves: the resolver rule
    // covers a hostname, the proxy covers an IP literal (Chromium never consults the
    // resolver for one) — see `Cdp::launch`'s and `live_proxy`'s module docs.
    let proxy = live_proxy::start(authority.host.clone(), authority.port)
        .map_err(|reason| format!("could not scope the capture's network reach: {reason}"))?;
    let mut cdp = Cdp::launch(
        browser_path,
        &profile,
        width,
        height,
        Some(&authority.host),
        Some(proxy.port),
    )
    .map_err(|reason| format!("could not open the capture browser: {reason}"))?;
    cdp.open_settled(target, SETTLE_MS)
        .map_err(|reason| format!("could not open {target}: {reason}"))?;

    let mut note = String::new();
    if !script.is_empty() {
        let wrapped = format!("(async () => {{\n{DX_ACTION_HELPER}\n{script}\n}})()");
        let value = cdp
            .evaluate_async(&wrapped)
            .map_err(|reason| format!("the capture script failed: {reason}"))?;
        if !value.is_null() {
            note = format!("script returned: {value}");
        }
    }

    let png = cdp
        .screenshot(None)
        .map_err(|reason| format!("could not screenshot {target}: {reason}"))?;
    let path = out_dir.join(format!("{}.png", block.id));
    std::fs::write(&path, &png)
        .map_err(|error| format!("could not save the screenshot: {error}"))?;
    let _ = std::fs::remove_dir_all(&profile);
    Ok((path, note))
}

/// The host and port a capture target names, for scoping the browser's DNS resolution
/// ([`Cdp::launch`]'s `allow_host`) and for polling readiness before opening it. Also the
/// authority [`crate::query`] scopes its own plain HTTP fetch to, for a `lang=query
/// target=` block — the same "exactly this host:port, nothing else" parsing either way.
#[derive(Debug)]
pub(crate) struct Authority {
    pub(crate) host: String,
    pub(crate) port: u16,
}

/// Parse `url`'s host and port.
///
/// This is not a general URL parser — it only has to be right for the http(s) targets a
/// capture block can name. A bracketed IPv6 literal is kept whole with its brackets; every
/// other authority is split on the last `:` to find a port, defaulting to 80 or 443.
pub(crate) fn target_authority(url: &str) -> Result<Authority, String> {
    let (scheme, rest) = if let Some(rest) = url.strip_prefix("https://") {
        ("https", rest)
    } else if let Some(rest) = url.strip_prefix("http://") {
        ("http", rest)
    } else {
        return Err(format!(
            "`target=` must be an http:// or https:// URL: {url}"
        ));
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let default_port = if scheme == "https" { 443 } else { 80 };
    let (host, port) = if authority.starts_with('[') {
        match authority.find(']') {
            Some(end) => {
                let host = authority[..=end].to_string();
                let port = authority[end + 1..]
                    .strip_prefix(':')
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(default_port);
                (host, port)
            }
            None => (authority.to_string(), default_port),
        }
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) => (host.to_string(), port.parse().unwrap_or(default_port)),
            None => (authority.to_string(), default_port),
        }
    };
    if host.is_empty() {
        return Err(format!("`target=` names no host: {url}"));
    }
    Ok(Authority { host, port })
}

/// Start `command` in `working_dir`, detached from this process's own stdio, with the
/// network available — the same trust `setup` phases already get in [`crate::plan`].
fn spawn_setup(command: &str, working_dir: &Path) -> Result<Child, String> {
    let mut spawned = shell_command(command);
    spawned
        .current_dir(working_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Its own process group, so killing it after capture can reach whatever it
        // spawned in turn (a dev server started through a package-manager wrapper
        // script) instead of leaving orphans behind.
        spawned.process_group(0);
    }
    spawned
        .spawn()
        .map_err(|error| format!("`setup=` could not start: {error}"))
}

/// Build the shell invocation for `command`, platform by platform.
fn shell_command(command: &str) -> Command {
    if cfg!(windows) {
        let mut spawned = Command::new("cmd");
        spawned.args(["/C", command]);
        spawned
    } else {
        let mut spawned = Command::new("sh");
        spawned.args(["-c", command]);
        spawned
    }
}

/// Poll `authority` until it accepts a TCP connection or `deadline` passes — a bounded
/// head start for `setup=` to bind its port. Best-effort: `open_settled`'s own failure is
/// what actually reports a target that never became reachable.
fn wait_reachable(authority: &Authority, deadline: Instant) {
    let address = (authority.host.as_str(), authority.port);
    while Instant::now() < deadline {
        if TcpStream::connect(address).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Best-effort kill of `child` and, on Unix, the whole process group it leads — a `setup=`
/// dev server started through a wrapper script otherwise survives its own launcher's death.
fn kill_tree(child: &mut Child) {
    #[cfg(unix)]
    {
        let _ = Command::new("kill")
            .arg("-9")
            .arg(format!("-{}", child.id()))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// A blocked capture, in the same shape [`crate::blocked`] gives every other runner.
fn blocked(message: &str) -> Capture {
    Capture {
        output: message.to_string(),
        exit: crate::BLOCKED_EXIT,
        timed_out: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_authority_reads_host_and_port() {
        let authority = target_authority("http://localhost:5173/app?x=1").expect("parse");
        assert_eq!(authority.host, "localhost");
        assert_eq!(authority.port, 5173);
    }

    #[test]
    fn target_authority_defaults_the_port_by_scheme() {
        assert_eq!(
            target_authority("http://example.com/").expect("parse").port,
            80
        );
        assert_eq!(
            target_authority("https://example.com").expect("parse").port,
            443
        );
    }

    #[test]
    fn target_authority_keeps_a_bracketed_ipv6_literal_whole() {
        let authority = target_authority("http://[::1]:9000/").expect("parse");
        assert_eq!(authority.host, "[::1]");
        assert_eq!(authority.port, 9000);
    }

    #[test]
    fn target_authority_refuses_a_url_with_no_scheme() {
        let error = target_authority("localhost:5173").expect_err("refused");
        assert!(error.contains("http://"), "{error}");
    }

    #[test]
    fn a_capture_block_with_no_writes_grant_is_blocked_not_run() {
        let block = Block {
            kind: "code".to_string(),
            id: "shot".to_string(),
            language: "capture".to_string(),
            run: true,
            target: "http://localhost:1/".to_string(),
            ..Block::default()
        };
        let capture = execute(&block, &[], Path::new("."), Duration::from_secs(5));
        assert_eq!(capture.exit, crate::BLOCKED_EXIT);
        assert!(capture.output.contains("writes="), "{}", capture.output);
    }

    #[test]
    fn a_capture_block_with_no_target_is_blocked_not_run() {
        let block = Block {
            kind: "code".to_string(),
            id: "shot".to_string(),
            language: "capture".to_string(),
            run: true,
            ..Block::default()
        };
        let capture = execute(
            &block,
            &[PathBuf::from(".")],
            Path::new("."),
            Duration::from_secs(5),
        );
        assert_eq!(capture.exit, crate::BLOCKED_EXIT);
        assert!(capture.output.contains("target="), "{}", capture.output);
    }
}
