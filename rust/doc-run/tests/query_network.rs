//! `lang=query`, attacked and proven the same way `capture_network.rs` proves `lang=capture`:
//! a real payload through the real `run_document`, checking both that a legitimate read
//! actually lands and that a query block reaches nothing beyond what it declared.
//!
//! `lang=query` is `report-26c0b880`'s answer — a declarative read for structured live
//! state (one field of a JSON file or a live service's response) that needs no
//! hand-written parsing script.

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use doc_core::resolve::{Nowhere, Provided};
use doc_run::{run_document, RunOptions};

/// A scratch document directory with a clean slate.
fn scene(label: &str) -> (PathBuf, RunOptions) {
    let root = std::env::temp_dir().join(format!("dx-query-attack-{label}"));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("scene");
    let options = RunOptions {
        document_dir: root.clone(),
        cache_root: root.join("cache"),
        default_timeout: Duration::from_secs(10),
        // Approved on purpose: a payload the review gate refused would leave every
        // assertion green without testing anything.
        approve: true,
        ..RunOptions::default()
    };
    (root, options)
}

/// Serve one fixed HTTP response to every connection, for as long as the test runs.
fn serve(status: &'static str, body: &'static str) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            thread::spawn(move || {
                let _ = respond(&mut stream, status, body);
            });
        }
    });
    port
}

fn respond(stream: &mut TcpStream, status: &str, body: &str) -> std::io::Result<()> {
    let mut buf = [0u8; 4096];
    let _ = stream.read(&mut buf);
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )?;
    stream.flush()
}

/// A server that records whether it was ever contacted — the boundary check needs a
/// "nothing else was reached" trace, the same shape `capture_network.rs` uses.
fn serve_recording(
    status: &'static str,
    body: &'static str,
) -> (u16, std::sync::Arc<std::sync::atomic::AtomicBool>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let reached = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = std::sync::Arc::clone(&reached);
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
            thread::spawn(move || {
                let _ = respond(&mut stream, status, body);
            });
        }
    });
    (port, reached)
}

#[test]
fn a_query_block_extracts_a_field_from_a_live_targets_json_response() {
    let (_root, options) = scene("target-happy-path");
    let port = serve("200 OK", r#"{"status":{"stepsDone":7}}"#);

    let source = format!(
        "::code id=steps lang=query run target=http://127.0.0.1:{port}/health \
         query=status.stepsDone timeout=10\n\n::end\n"
    );
    let report = run_document(&source, &options, &Nowhere).expect("acyclic run");
    let run = report.runs.first().expect("one run");
    assert_eq!(run.status, "ok", "{}", run.output);
    assert_eq!(run.output, "7");
}

#[test]
fn a_query_block_extracts_a_field_from_a_sibling_src_file() {
    let (_root, options) = scene("src-happy-path");
    let mut provided = Provided::new();
    provided.add_file("status.json", r#"{"stepsDone": 3, "items": ["a", "b"]}"#);

    let source =
        "::code id=steps lang=query run src=status.json query=items[1]\n\n::end\n".to_string();
    let report = run_document(&source, &options, &provided).expect("acyclic run");
    let run = report.runs.first().expect("one run");
    assert_eq!(run.status, "ok", "{}", run.output);
    assert_eq!(run.output, "b");
}

#[test]
fn a_query_block_never_reaches_a_host_other_than_its_own_target() {
    // The boundary: a query block names exactly one `target=`. Even though this module
    // makes only one deliberate GET (there is no author script that could try to fetch
    // anywhere else), the test still proves the *declared* scope is what gets reached —
    // no redirect, no second host — by giving the allowed target a body that itself looks
    // like another URL, and checking the forbidden server is never contacted.
    let (_root, options) = scene("no-cross-reach");
    let (forbidden_port, reached) = serve_recording("200 OK", "should never be contacted");
    let allowed_port = serve(
        "200 OK",
        r#"{"note":"http://127.0.0.1:1/should-not-be-followed"}"#,
    );

    let source = format!(
        "::code id=q lang=query run target=http://127.0.0.1:{allowed_port}/ query=note \
         timeout=10\n\n::end\n"
    );
    let report = run_document(&source, &options, &Nowhere).expect("acyclic run");
    let run = report.runs.first().expect("one run");
    assert_eq!(run.status, "ok", "{}", run.output);
    assert!(run.output.contains("127.0.0.1:1"), "{}", run.output);

    thread::sleep(Duration::from_millis(200));
    assert!(
        !reached.load(std::sync::atomic::Ordering::SeqCst),
        "a query block reached a server other than the one its `target=` declared \
         — the extracted text naming another URL must never cause a second fetch, since \
         there is no author code here to follow it"
    );
    let _ = forbidden_port;
}

#[test]
fn a_query_block_never_follows_a_redirect_to_another_host() {
    let (_root, options) = scene("no-redirect");
    let (forbidden_port, reached) = serve_recording("200 OK", r#"{"secret":true}"#);
    let redirecting_port = {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let _ = write!(
                    stream,
                    "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{forbidden_port}/\r\n\
                     Content-Length: 0\r\n\r\n"
                );
                let _ = stream.flush();
            }
        });
        port
    };

    let source = format!(
        "::code id=q lang=query run target=http://127.0.0.1:{redirecting_port}/ query=secret \
         timeout=10\n\n::end\n"
    );
    let report = run_document(&source, &options, &Nowhere).expect("acyclic run");
    let run = report.runs.first().expect("one run");
    assert_eq!(run.status, "error", "{}", run.output);
    assert!(run.output.contains("redirect"), "{}", run.output);

    thread::sleep(Duration::from_millis(200));
    assert!(
        !reached.load(std::sync::atomic::Ordering::SeqCst),
        "a query block followed a redirect to a host its `target=` never named"
    );
}

#[test]
fn a_query_block_with_an_https_target_is_blocked_before_any_socket_opens() {
    let (_root, options) = scene("https-blocked");
    let source =
        "::code id=q lang=query run target=https://example.com/health query=status\n\n::end\n"
            .to_string();
    let report = run_document(&source, &options, &Nowhere).expect("acyclic run");
    let run = report.runs.first().expect("one run");
    assert_eq!(run.status, "blocked", "{}", run.output);
    assert!(run.output.contains("http://"), "{}", run.output);
}

#[test]
fn an_unapproved_query_block_is_blocked_pending_review_like_any_other_runnable_block() {
    let root = std::env::temp_dir().join("dx-query-attack-unapproved");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("scene");
    let options = RunOptions {
        document_dir: root.clone(),
        cache_root: root.join("cache"),
        default_timeout: Duration::from_secs(10),
        approve: false,
        ..RunOptions::default()
    };
    let port = serve("200 OK", r#"{"status":"up"}"#);
    let source = format!(
        "::code id=q lang=query run target=http://127.0.0.1:{port}/ query=status\n\n::end\n"
    );
    let report = run_document(&source, &options, &Nowhere).expect("acyclic run");
    let run = report.runs.first().expect("one run");
    assert_eq!(run.status, "blocked", "{}", run.output);
    assert!(run.output.contains("pending review"), "{}", run.output);
}
