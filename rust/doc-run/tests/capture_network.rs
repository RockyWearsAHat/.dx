//! `lang=capture`'s one narrow exception to "a block's own code never reaches the
//! network" (see `doc-run/src/live.rs`'s module doc), attacked the same way
//! `attacks.rs` attacks every other runner: a real payload, run through the real
//! `run_document`, asserting the reach that should be refused actually is.
//!
//! Skips outright when this machine has no Chromium-family browser — `doc_shot::browser`
//! already reports that plainly to a human; a CI box without one should not fail here.

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use doc_core::resolve::Nowhere;
use doc_run::{run_document, RunOptions};

/// Launching a real, headless Chromium concurrently from several tests in this file was
/// flaky on at least one machine — serialize the ones that do it, the same reason
/// `attacks.rs` serializes its sandbox-toggling test.
static BROWSER_LOCK: Mutex<()> = Mutex::new(());

/// A scratch document directory with a clean slate.
fn scene(label: &str) -> (PathBuf, RunOptions) {
    let root = std::env::temp_dir().join(format!("dx-capture-attack-{label}"));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("out")).expect("scene");
    let options = RunOptions {
        document_dir: root.clone(),
        cache_root: root.join("cache"),
        default_timeout: Duration::from_secs(30),
        approve: true,
        ..RunOptions::default()
    };
    (root, options)
}

/// Serve one HTTP response, in a loop, until the returned handle is dropped is not
/// enforced — the thread simply serves connections for as long as the test runs and dies
/// with the process. Good enough for a scratch listener nothing else will reuse.
fn serve(body: &'static str, headers: &'static str) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            thread::spawn(move || {
                let _ = respond(&mut stream, body, headers);
            });
        }
    });
    port
}

fn respond(stream: &mut TcpStream, body: &str, headers: &str) -> std::io::Result<()> {
    let mut buf = [0u8; 4096];
    // Drain the request line/headers without needing to parse them — every response here
    // is the same regardless of what was asked.
    let _ = stream.read(&mut buf);
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n{headers}\r\n\r\n{body}",
        body.len()
    )?;
    stream.flush()
}

#[test]
fn a_capture_scripts_fetch_to_a_different_ip_literal_port_is_refused() {
    // The realistic, load-bearing case: dev servers are overwhelmingly addressed by IP
    // literal (`http://127.0.0.1:5173/`), not hostname — and Chromium's host resolver is
    // not consulted for an IP literal at all, so `--host-resolver-rules` cannot gate this
    // the way it gates a hostname. If this ever passes for the wrong reason (the fetch
    // "failing" only because the port is closed rather than because it was refused), the
    // `REACHED` assertion below is the one that would actually catch a real regression —
    // this test exists specifically to keep that promise honest rather than assumed.
    let Some(_) = doc_shot::browser::find() else {
        eprintln!("skipping: no browser on this machine");
        return;
    };
    let _guard = BROWSER_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let (root, options) = scene("cross-port");

    let forbidden_port = serve(
        "should never be readable",
        "Access-Control-Allow-Origin: *\r\n",
    );
    let allowed_port = serve("<html><body>target</body></html>", "");

    let source = format!(
        "::code id=shot lang=capture run target=http://127.0.0.1:{allowed_port}/ writes=out timeout=25\n\
         try {{\n\
         \x20\x20const r = await fetch('http://127.0.0.1:{forbidden_port}/', {{ mode: 'cors' }});\n\
         \x20\x20return 'REACHED:' + r.status;\n\
         }} catch (e) {{\n\
         \x20\x20return 'BLOCKED:' + String(e);\n\
         }}\n\
         ::end\n"
    );
    let report = run_document(&source, &options, &Nowhere).expect("acyclic run");
    let run = report.runs.first().expect("one run");

    assert!(
        !run.output.contains("REACHED"),
        "a capture script reached a different port on the same IP literal target — \
         --host-resolver-rules does not gate IP literals, so this is a real gap, not a \
         flaky assertion: {}",
        run.output
    );
    assert!(root.join("out").join("shot.png").exists(), "{}", run.output);
}

#[test]
fn a_capture_scripts_fetch_to_a_different_hostname_is_refused() {
    let Some(_) = doc_shot::browser::find() else {
        eprintln!("skipping: no browser on this machine");
        return;
    };
    let _guard = BROWSER_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let (root, options) = scene("cross-host");

    let forbidden_port = serve(
        "should never be readable",
        "Access-Control-Allow-Origin: *\r\n",
    );
    let allowed_port = serve("<html><body>target</body></html>", "");

    // `dx-capture-test.invalid` is a hostname (RFC 2606 reserves `.invalid` so it can
    // never resolve for real anywhere), so this exercises the actual host-resolver-rules
    // path — `MAP * ~NOTFOUND` refuses it the instant it is looked up, with no real DNS
    // query and no dependence on this machine having internet access.
    let source = format!(
        "::code id=shot lang=capture run target=http://127.0.0.1:{allowed_port}/ writes=out timeout=25\n\
         try {{\n\
         \x20\x20const r = await fetch('http://dx-capture-test.invalid:{forbidden_port}/', {{ mode: 'cors' }});\n\
         \x20\x20return 'REACHED:' + r.status;\n\
         }} catch (e) {{\n\
         \x20\x20return 'BLOCKED:' + String(e);\n\
         }}\n\
         ::end\n"
    );
    let report = run_document(&source, &options, &Nowhere).expect("acyclic run");
    let run = report.runs.first().expect("one run");

    assert!(
        !run.output.contains("REACHED"),
        "a capture script reached a host name other than its own target: {}",
        run.output
    );
    assert!(root.join("out").join("shot.png").exists(), "{}", run.output);
}

#[test]
fn a_capture_reaches_exactly_the_target_it_names() {
    let Some(_) = doc_shot::browser::find() else {
        eprintln!("skipping: no browser on this machine");
        return;
    };
    let _guard = BROWSER_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let (root, options) = scene("same-host");
    let port = serve("<html><body>hello from the target</body></html>", "");

    let source = format!(
        "::code id=shot lang=capture run target=http://127.0.0.1:{port}/ writes=out timeout=25\n\
         return document.body.innerText;\n\
         ::end\n"
    );
    let report = run_document(&source, &options, &Nowhere).expect("acyclic run");
    let run = report.runs.first().expect("one run");

    assert_eq!(run.status, "ok", "{}", run.output);
    assert!(
        run.output.contains("hello from the target"),
        "{}",
        run.output
    );
    assert!(root.join("out").join("shot.png").exists(), "{}", run.output);
}

#[test]
fn an_actions_script_fills_a_form_and_clicks_a_real_button() {
    let Some(_) = doc_shot::browser::find() else {
        eprintln!("skipping: no browser on this machine");
        return;
    };
    let _guard = BROWSER_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let (root, options) = scene("actions-form");
    let page = "<html><body>\
                <input id=\"name\">\
                <button id=\"go\">Go</button>\
                <div id=\"result\"></div>\
                <script>\
                document.getElementById('go').addEventListener('click', () => {\
                document.getElementById('result').textContent = \
                'hello ' + document.getElementById('name').value;\
                });\
                </script>\
                </body></html>";
    let port = serve(page, "");

    let source = format!(
        "::code id=shot lang=capture run actions target=http://127.0.0.1:{port}/ writes=out timeout=25\n\
         type #name \"world\"; click #go; wait 100ms; \
         eval return document.querySelector('#result').textContent;\n\
         ::end\n"
    );
    let report = run_document(&source, &options, &Nowhere).expect("acyclic run");
    let run = report.runs.first().expect("one run");

    assert_eq!(run.status, "ok", "{}", run.output);
    assert!(run.output.contains("hello world"), "{}", run.output);
    assert!(root.join("out").join("shot.png").exists(), "{}", run.output);
}

#[test]
fn a_capture_block_is_gated_by_approval_like_any_other_runner() {
    let (root, mut options) = scene("unapproved");
    options.approve = false;
    let source = "::code id=shot lang=capture run target=http://127.0.0.1:1/ writes=out\n\
                  return 1;\n\
                  ::end\n";
    let report = run_document(source, &options, &Nowhere).expect("acyclic run");
    let run = report.runs.first().expect("one run");
    assert_eq!(run.status, "blocked", "{}", run.output);
    assert!(!root.join("out").join("shot.png").exists());
}
