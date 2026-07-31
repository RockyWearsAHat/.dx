//! `dx serve` — the rendering service every surface talks to.
//!
//! # Why a service
//! A `.dx` file on disk is a pointer, so anything that shows a document has to resolve it,
//! and every place a person reads one is a different kind of program: a browser on
//! github.com, a phone, an editor, a file preview. Teaching each of them the format means
//! shipping the engine into each of them — a wasm build here, a native build there, each
//! with its own lifetime and its own cache, or in the browser's case no durable cache at
//! all, because the extension process it would live in is shut down after seconds of
//! idleness.
//!
//! So the engine runs once, here, in a process that stays alive and keeps what it decodes.
//! Every surface becomes a thin client: it locates a pointer and paints what comes back.
//! That is what makes `dx` a thing you install once per machine rather than once per program.
//!
//! # What it will and will not do
//! The daemon is a pure function from bytes the caller handed it to a rendering of them. It
//! reads no files, writes none, and runs nothing — `dx run` is the only thing that executes
//! a document, and it is not reachable from here.
//!
//! What it *holds*, though, is repository content the reader's browser gave it, which can be
//! private. So the port is not open to everything on the machine that finds it: every request
//! must name a loopback `Host` and, if it carries an `Origin` at all, must be the browser
//! extension. See [`refuse_unless_local`] for why those two checks and not others.
//!
//! # The shape of a session
//! A client names a pack rather than sending it. The first mention necessarily misses, the
//! daemon says which pack it needs, the client uploads it once, and every call after that —
//! every file in a pull request, every page of a repository — is answered from memory.
//!
//! ```text
//! POST /engine  {"call":"pack_document","args":[{"pack":"…/main/.doc/repo.dxcp"},"notes.dx"]}
//!   ← 409       {"needPack":"…/main/.doc/repo.dxcp"}
//! POST /pack    x-dx-pack: …/main/.doc/repo.dxcp     <the bytes, once>
//!   ← 200       {"paths":["notes.dx", …]}
//! POST /engine  {"call":"pack_document", …}          <and every call after it>
//!   ← 200       {"value":"::heading level=1\nHello\n"}
//! ```

pub mod engine;
pub mod http;
pub mod packs;

use std::io::{BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use packs::Packs;

/// The ports the daemon tries, in order.
///
/// A fixed, small range rather than one port or an ephemeral one: a client cannot be told
/// which port was chosen — a browser extension reads no files — so it probes the same short
/// list. One port would mean any other program holding it stops `dx` from starting at all.
pub const PORTS: [u16; 4] = [7333, 7334, 7335, 7336];

/// How long a client may be silent before its connection is dropped.
///
/// This bounds *idleness* only. What bounds a whole request is
/// [`http::REQUEST_BUDGET`] — a client that dribbles one byte per second is never idle, and
/// so is never dropped by this alone.
const TIMEOUT: Duration = Duration::from_secs(5);

/// How many connections may be in flight before further ones are closed unanswered.
const MAX_CONNECTIONS: usize = 64;

/// Bind the first free port and answer requests until the process is stopped.
///
/// When `port` is given that port is used and a failure to bind it is reported, because a
/// caller who named a port wants to know it was not honored.
///
/// # Errors
/// When no port in [`PORTS`] can be bound, or when a named `port` cannot be.
pub fn serve(port: Option<u16>, log: &mut impl Write) -> Result<(), String> {
    let listener = bind(port)?;
    let address = listener
        .local_addr()
        .map_err(|error| format!("the daemon could not report its address: {error}"))?;
    let _ = writeln!(
        log,
        "dx serve\n\n  listening  http://{address}\n  engine     doc-core {}\n\n\
         Reading is answered from memory; nothing here reads files or runs code.\n\
         Stop it with control-c.",
        env!("CARGO_PKG_VERSION")
    );

    let held = Arc::new(Mutex::new(Packs::new()));
    let live = Arc::new(AtomicUsize::new(0));
    for incoming in listener.incoming() {
        let stream = match incoming {
            Ok(stream) => stream,
            // One refused connection is not a reason to stop serving every other client.
            Err(_) => continue,
        };
        // Thread-per-connection with no ceiling is a local denial of service: anything on this
        // machine can open sockets faster than they retire and take the process down with it.
        // Past the cap a connection is simply closed, which a client retries.
        if live.load(Ordering::Relaxed) >= MAX_CONNECTIONS {
            drop(stream);
            continue;
        }
        live.fetch_add(1, Ordering::Relaxed);
        let held = Arc::clone(&held);
        let live = Arc::clone(&live);
        // A connection is handled on its own thread so one slow client cannot hold up the
        // reader waiting on the next page.
        std::thread::spawn(move || {
            converse(&stream, &held);
            live.fetch_sub(1, Ordering::Relaxed);
        });
    }
    Ok(())
}

/// Bind `port`, or the first free port in [`PORTS`] when none is named.
fn bind(port: Option<u16>) -> Result<TcpListener, String> {
    if port == Some(0) {
        return Err("port 0 would take a port no client can find.\n\
                    Run `dx serve` with no --port, or name one of the ports clients probe."
            .to_string());
    }
    let candidates: Vec<u16> = port.map_or_else(|| PORTS.to_vec(), |chosen| vec![chosen]);
    let mut refusal = None;
    for candidate in &candidates {
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, *candidate));
        match TcpListener::bind(address) {
            Ok(listener) => return Ok(listener),
            // Kept, not discarded: a port can fail to bind for reasons that are not "taken" —
            // permission, most often — and reporting the wrong cause sends the reader hunting
            // for a process that does not exist.
            Err(error) => refusal = Some(error),
        }
    }
    let cause = refusal.map_or_else(String::new, |error| format!(" ({error})"));
    Err(match port {
        Some(chosen) => format!(
            "port {chosen} could not be opened{cause}.\n\
             Run `dx serve` with no --port to take the first free one."
        ),
        None => format!(
            "no port dx serves on could be opened{cause} ({}).\n\
             A dx daemon is probably running already; stop it, or name a port with --port.",
            PORTS
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    })
}

/// Read one request from `stream`, answer it, and close.
fn converse(stream: &TcpStream, held: &Mutex<Packs>) {
    let _ = stream.set_read_timeout(Some(TIMEOUT));
    let _ = stream.set_write_timeout(Some(TIMEOUT));

    let mut reader = BufReader::new(stream);
    let response = match http::read(&mut reader, Instant::now() + http::REQUEST_BUDGET) {
        Ok(request) => {
            let origin = request.header("origin").map(str::to_string);
            (answer(&request, held), origin)
        }
        // A request that could not be parsed gets a status, not a guess.
        Err(message) => (http::Response::text(400, &message), None),
    };
    let (body, origin) = response;
    let mut writer = stream;
    let _ = http::write(&mut writer, &body, origin.as_deref());
}

/// The response to `request`.
///
/// Kept free of sockets so every route is testable as a function of a request.
#[must_use]
pub fn answer(request: &http::Request, held: &Mutex<Packs>) -> http::Response {
    if let Some(refusal) = refuse_unless_local(request) {
        return refusal;
    }

    // A browser asks permission before it will make the real request; answering it is the
    // whole of the preflight.
    if request.method == "OPTIONS" {
        return http::Response::text(204, "");
    }

    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/health") => health(held),
        ("POST", "/pack") => store_pack(request, held),
        ("POST", "/engine") => run_call(request, held),
        ("GET" | "POST", _) => http::Response::json(
            404,
            error_body(&format!("`{}` is not a dx daemon route", request.path)),
        ),
        _ => http::Response::json(
            405,
            error_body(&format!("the dx daemon does not answer {}", request.method)),
        ),
    }
}

/// Refuse a request that did not come from this machine, or from a client allowed to be here.
///
/// # What this is defending
/// The daemon holds packs the reader's browser handed it, and the whole point of the extension
/// is that those can come from **private repositories**. So "it only answers about bytes the
/// caller sent" was never the right claim — it answers about bytes *any* caller sent, and pack
/// keys are guessable (`…/<org>/<repo>/raw/main/.doc/repo.dxcp`). Two ways in had to close:
///
/// - **DNS rebinding.** A site whose name re-resolves to `127.0.0.1` is treated by the browser
///   as same-origin with the daemon, so no cross-origin rule applies at all. It cannot forge
///   `Host`, which still names the attacker, so requiring a loopback `Host` stops it.
/// - **An ordinary cross-origin `fetch`.** A `POST` of `text/plain` is a *simple* request: it
///   is sent with no preflight, so refusing it has to happen here rather than in a CORS header.
///
/// A request with no `Origin` at all is a native client — `curl`, an editor, the phone app —
/// and is allowed, because no page can make a browser omit that header.
fn refuse_unless_local(request: &http::Request) -> Option<http::Response> {
    let host = request.header("host").unwrap_or_default();
    if !http::is_loopback_host(host) {
        return Some(http::Response::json(
            403,
            error_body(
                "the dx daemon answers only on this machine. \
                 A request naming another host is a browser being told to treat it as one.",
            ),
        ));
    }
    match request.header("origin") {
        None => None,
        Some(origin) if http::is_allowed_origin(origin) => None,
        Some(_) => Some(http::Response::json(
            403,
            error_body(
                "that origin may not use the dx daemon. \
                 Only the dx browser extension talks to it.",
            ),
        )),
    }
}

/// `GET /health` — what a client probes to find the daemon among [`PORTS`].
fn health(held: &Mutex<Packs>) -> http::Response {
    let packs = held.lock().map_or(0, |packs| packs.len());
    http::Response::json(
        200,
        serde_json::json!({
            "name": "dx",
            "version": env!("CARGO_PKG_VERSION"),
            "packs": packs,
        })
        .to_string(),
    )
}

/// `POST /pack` — take a pack's bytes once, keyed by the `x-dx-pack` header.
fn store_pack(request: &http::Request, held: &Mutex<Packs>) -> http::Response {
    let Some(key) = request.header("x-dx-pack") else {
        return http::Response::json(
            400,
            error_body("a pack upload must name the pack in the x-dx-pack header"),
        );
    };
    let mut packs = match held.lock() {
        Ok(packs) => packs,
        Err(_) => return http::Response::json(500, error_body(HELD_POISONED)),
    };
    match packs.store(key, &request.body) {
        Ok(paths) => http::Response::json(200, serde_json::json!({ "paths": paths }).to_string()),
        Err(message) => http::Response::json(400, error_body(&message)),
    }
}

/// `POST /engine` — run one allowlisted call.
fn run_call(request: &http::Request, held: &Mutex<Packs>) -> http::Response {
    let parsed: Value = match serde_json::from_slice(&request.body) {
        Ok(parsed) => parsed,
        Err(error) => {
            return http::Response::json(400, error_body(&format!("that is not JSON: {error}")))
        }
    };
    let Some(name) = parsed.get("call").and_then(Value::as_str) else {
        return http::Response::json(400, error_body("a call must name a `call`"));
    };
    let args: Vec<Value> = parsed
        .get("args")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut packs = match held.lock() {
        Ok(packs) => packs,
        Err(_) => return http::Response::json(500, error_body(HELD_POISONED)),
    };
    match engine::call(&mut packs, name, &args) {
        Ok(engine::Outcome::Value(value)) => {
            http::Response::json(200, serde_json::json!({ "value": value }).to_string())
        }
        // Not a failure: the daemon fetches nothing, so it says which pack it needs and the
        // client hands it over once.
        Ok(engine::Outcome::NeedPack(key)) => {
            http::Response::json(409, serde_json::json!({ "needPack": key }).to_string())
        }
        Err(message) => http::Response::json(400, error_body(&message)),
    }
}

/// What a client is told when the held packs cannot be reached.
///
/// Only reachable if a thread panicked while holding the lock, which nothing here does; it is
/// answered rather than propagated so one client's failure cannot take the daemon down.
const HELD_POISONED: &str = "the dx daemon lost its packs; restart it with `dx serve`";

/// A JSON error body, the one shape every failure takes.
fn error_body(message: &str) -> String {
    serde_json::json!({ "error": message }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use doc_core::chunk::Pack as CorePack;

    /// A real encoded pack holding one document at `path`.
    fn pack_bytes(path: &str, source: &str) -> Vec<u8> {
        let document = doc_core::format::parse(source);
        let pack = CorePack::build([(path, &document)]);
        doc_core::chunk::encode_pack(&pack)
    }

    /// A request, built the way the socket path would have parsed one.
    ///
    /// A loopback `Host` is supplied unless the caller names one, because every request that
    /// reaches the daemon over a socket carries one and the access check depends on it.
    fn request(method: &str, path: &str, headers: &[(&str, &str)], body: Vec<u8>) -> http::Request {
        let mut headers: BTreeMap<String, String> = headers
            .iter()
            .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
            .collect();
        headers
            .entry("host".to_string())
            .or_insert_with(|| "127.0.0.1:7333".to_string());
        http::Request {
            method: method.to_string(),
            path: path.to_string(),
            headers,
            body,
        }
    }

    /// The JSON a response carries.
    fn body_of(response: &http::Response) -> Value {
        serde_json::from_slice(&response.body).expect("every route answers with JSON")
    }

    /// A call request for `name` with `args`.
    fn engine_call(name: &str, args: Value) -> http::Request {
        let body = serde_json::json!({ "call": name, "args": args }).to_string();
        request("POST", "/engine", &[], body.into_bytes())
    }

    #[test]
    fn health_names_the_daemon_so_a_client_can_find_it_among_the_ports() {
        let held = Mutex::new(Packs::new());
        let response = answer(&request("GET", "/health", &[], Vec::new()), &held);
        assert_eq!(response.status, 200);
        assert_eq!(body_of(&response)["name"], "dx");
    }

    #[test]
    fn a_preflight_is_answered_so_the_real_request_is_allowed_to_happen() {
        let held = Mutex::new(Packs::new());
        let response = answer(&request("OPTIONS", "/engine", &[], Vec::new()), &held);
        assert_eq!(response.status, 204);
    }

    #[test]
    fn the_first_mention_of_a_pack_asks_for_it_and_the_call_after_the_upload_answers() {
        let held = Mutex::new(Packs::new());
        let key = "https://github.com/a/b/raw/main/.doc/repo.dxcp";
        let call = engine_call(
            "pack_document",
            serde_json::json!([{ "pack": key }, "notes.dx"]),
        );

        let asked = answer(&call, &held);
        assert_eq!(asked.status, 409);
        assert_eq!(body_of(&asked)["needPack"], key);

        let upload = request(
            "POST",
            "/pack",
            &[("x-dx-pack", key)],
            pack_bytes("notes.dx", "::heading level=1\nHello\n"),
        );
        let stored = answer(&upload, &held);
        assert_eq!(stored.status, 200);
        assert_eq!(body_of(&stored)["paths"][0], "notes.dx");

        let answered = answer(&call, &held);
        assert_eq!(answered.status, 200);
        let value = body_of(&answered)["value"]
            .as_str()
            .unwrap_or("")
            .to_string();
        assert!(value.contains("Hello"), "{value}");
    }

    #[test]
    fn an_upload_without_the_header_says_which_header_it_wanted() {
        let held = Mutex::new(Packs::new());
        let response = answer(
            &request(
                "POST",
                "/pack",
                &[],
                pack_bytes("notes.dx", "::text\nOne\n"),
            ),
            &held,
        );
        assert_eq!(response.status, 400);
        let message = body_of(&response)["error"]
            .as_str()
            .unwrap_or("")
            .to_string();
        assert!(message.contains("x-dx-pack"), "{message}");
    }

    #[test]
    fn a_call_the_engine_does_not_answer_is_refused_with_its_name() {
        let held = Mutex::new(Packs::new());
        let response = answer(&engine_call("read_file", serde_json::json!([])), &held);
        assert_eq!(response.status, 400);
        let message = body_of(&response)["error"]
            .as_str()
            .unwrap_or("")
            .to_string();
        assert!(message.contains("read_file"), "{message}");
    }

    #[test]
    fn a_body_that_is_not_json_is_a_sentence_rather_than_a_crash() {
        let held = Mutex::new(Packs::new());
        let response = answer(
            &request("POST", "/engine", &[], b"not json".to_vec()),
            &held,
        );
        assert_eq!(response.status, 400);
    }

    #[test]
    fn an_unknown_route_is_answered_rather_than_hung() {
        let held = Mutex::new(Packs::new());
        let response = answer(&request("GET", "/etc/passwd", &[], Vec::new()), &held);
        assert_eq!(response.status, 404);
    }

    #[test]
    fn a_method_the_daemon_does_not_answer_is_refused() {
        let held = Mutex::new(Packs::new());
        let response = answer(&request("DELETE", "/pack", &[], Vec::new()), &held);
        assert_eq!(response.status, 405);
    }

    #[test]
    fn stylesheet_and_render_are_answered_without_any_pack_at_all() {
        let held = Mutex::new(Packs::new());
        let css = answer(&engine_call("stylesheet", serde_json::json!([])), &held);
        assert_eq!(css.status, 200);
        assert!(!body_of(&css)["value"].as_str().unwrap_or("").is_empty());

        let html = answer(
            &engine_call(
                "render_html",
                serde_json::json!(["::heading level=1\nHello\n", "auto", true, false]),
            ),
            &held,
        );
        assert_eq!(html.status, 200);
        let rendered = body_of(&html)["value"].as_str().unwrap_or("").to_string();
        assert!(rendered.contains("Hello"), "{rendered}");
    }

    /// A pack key naming a private repository, as the extension would have uploaded it.
    const PRIVATE: &str = "https://github.com/acme/secrets/raw/main/.doc/repo.dxcp";

    /// A daemon holding a private repository's pack, which is what an attacker is after.
    fn holding_a_private_pack() -> Mutex<Packs> {
        let held = Mutex::new(Packs::new());
        let upload = request(
            "POST",
            "/pack",
            &[("x-dx-pack", PRIVATE)],
            pack_bytes("notes.dx", "::text\nthe merger closes friday\n"),
        );
        assert_eq!(answer(&upload, &held).status, 200);
        held
    }

    /// The call an attacker would make for a guessed pack key.
    fn read_private() -> http::Request {
        let body = serde_json::json!({
            "call": "pack_document",
            "args": [{ "pack": PRIVATE }, "notes.dx"],
        })
        .to_string();
        request("POST", "/engine", &[], body.into_bytes())
    }

    #[test]
    fn a_rebound_name_cannot_reach_the_daemon_even_though_the_browser_thinks_it_is_local() {
        // DNS rebinding: the browser treats the daemon as same-origin, so no cross-origin rule
        // applies at all. The one thing the page cannot forge is Host, which still names it.
        let held = holding_a_private_pack();
        let mut attack = read_private();
        attack
            .headers
            .insert("host".to_string(), "evil.example".to_string());

        let response = answer(&attack, &held);
        assert_eq!(response.status, 403);
        let body = String::from_utf8_lossy(&response.body);
        assert!(!body.contains("merger"), "the document leaked: {body}");
    }

    #[test]
    fn a_page_on_another_site_cannot_read_a_private_repository_out_of_the_daemon() {
        // `POST` of `text/plain` is a CORS-*simple* request: it arrives with no preflight, so
        // refusing it has to happen here and not in a response header.
        let held = holding_a_private_pack();
        let mut attack = read_private();
        attack
            .headers
            .insert("origin".to_string(), "https://evil.example".to_string());

        let response = answer(&attack, &held);
        assert_eq!(response.status, 403);
        let body = String::from_utf8_lossy(&response.body);
        assert!(!body.contains("merger"), "the document leaked: {body}");
    }

    #[test]
    fn github_com_itself_is_not_allowed_either() {
        // The extension reaches the daemon from its own background context, never from the
        // page, so a request claiming to be github.com is not the extension.
        let held = holding_a_private_pack();
        let mut attack = read_private();
        attack
            .headers
            .insert("origin".to_string(), "https://github.com".to_string());
        assert_eq!(answer(&attack, &held).status, 403);
    }

    #[test]
    fn the_extension_and_a_native_client_are_both_still_served() {
        let held = holding_a_private_pack();

        let mut extension = read_private();
        extension.headers.insert(
            "origin".to_string(),
            "chrome-extension://abcdefghijklmnop".to_string(),
        );
        assert_eq!(answer(&extension, &held).status, 200);

        // No `Origin` at all is a native client — curl, an editor, the phone app. No page can
        // make a browser omit that header, so this cannot be a site in disguise.
        assert_eq!(answer(&read_private(), &held).status, 200);
    }

    #[test]
    fn a_host_header_is_required_before_anything_is_answered() {
        let held = Mutex::new(Packs::new());
        let mut nameless = request("GET", "/health", &[], Vec::new());
        nameless.headers.remove("host");
        assert_eq!(answer(&nameless, &held).status, 403);
    }

    #[test]
    fn loopback_is_recognized_however_it_is_spelled_and_nothing_else_is() {
        for host in [
            "127.0.0.1",
            "127.0.0.1:7333",
            "localhost:7334",
            "[::1]:7333",
        ] {
            assert!(http::is_loopback_host(host), "{host} is this machine");
        }
        for host in [
            "evil.example",
            "evil.example:7333",
            "127.0.0.1.evil.example",
            "notlocalhost",
            "",
        ] {
            assert!(!http::is_loopback_host(host), "{host} is not this machine");
        }
    }

    #[test]
    fn only_an_extension_origin_is_granted_and_never_one_carrying_header_bytes() {
        assert!(http::is_allowed_origin("chrome-extension://abc"));
        assert!(http::is_allowed_origin("moz-extension://abc"));
        for origin in [
            "https://github.com",
            "https://evil.example",
            "null",
            // A bare CR survives header splitting; reflecting it would let a caller append
            // headers of its own to the response.
            "chrome-extension://abc\rX-Injected: yes",
            "chrome-extension://abc\nX-Injected: yes",
        ] {
            assert!(!http::is_allowed_origin(origin), "{origin} was granted");
        }
    }

    #[test]
    fn port_zero_is_refused_because_no_client_could_find_it() {
        let error = bind(Some(0)).expect_err("port 0 is refused");
        assert!(error.contains("no client can find"), "{error}");
    }

    #[test]
    fn a_named_port_that_is_taken_is_reported_rather_than_silently_moved() {
        let held = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .expect("binding an ephemeral port succeeds");
        let taken = held
            .local_addr()
            .expect("the listener has an address")
            .port();
        let error = bind(Some(taken)).expect_err("a taken port is refused");
        assert!(error.contains(&taken.to_string()), "{error}");
    }

    #[test]
    fn an_unnamed_port_is_taken_from_the_range_clients_probe() {
        let Ok(listener) = bind(None) else {
            // Every port in the range is in use on this machine, which the caller is told
            // about; there is nothing to assert.
            return;
        };
        let port = listener.local_addr().expect("bound").port();
        assert!(
            PORTS.contains(&port),
            "bound {port}, which no client probes"
        );
    }
}
