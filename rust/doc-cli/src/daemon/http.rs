//! A minimal HTTP/1.1 server: exactly enough for one local client, and no more.
//!
//! The daemon answers a browser shim over the loopback interface. That is a narrow enough
//! job that a dependency would cost more than it saves — there is no TLS to terminate, no
//! routing to do, no chunked body to reassemble, and no keep-alive to negotiate. One
//! request, one response, connection closed.
//!
//! The subset is deliberately strict rather than forgiving: a request this module does not
//! understand is answered with a status code, never guessed at. A parser that tries to
//! accept everything is how a local port becomes a liability.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::time::Instant;

/// Largest request head this server will read, in bytes.
///
/// A head longer than this is not a request the daemon has any answer for. The limit is
/// enforced *as the bytes are read* — see [`read_line_bounded`] — because a limit checked
/// after a line is complete never gets to say no: a peer that sends no newline at all grows
/// the buffer until the machine runs out of memory.
const MAX_HEAD: usize = 8 * 1024;

/// Largest request body this server will read, in bytes.
///
/// The largest thing any client sends is a repository's `.doc/repo.dxcp`, which is
/// compressed and chunk-deduplicated; 64 MiB is far above any real one and still bounded.
const MAX_BODY: usize = 64 * 1024 * 1024;

/// One request, parsed down to the parts the daemon acts on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// The method, uppercase as it arrived (`GET`, `POST`, `OPTIONS`).
    pub method: String,
    /// The path, without any query string.
    pub path: String,
    /// Header names lowercased, so a lookup never depends on how a client capitalized one.
    pub headers: BTreeMap<String, String>,
    /// The body, empty when the request declared no `Content-Length`.
    pub body: Vec<u8>,
}

impl Request {
    /// The value of the header `name`, which must be given in lowercase.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }
}

/// One response, before it is written to a socket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    /// The status code.
    pub status: u16,
    /// The `Content-Type` to send.
    pub content_type: &'static str,
    /// The body bytes.
    pub body: Vec<u8>,
}

impl Response {
    /// A JSON response with `status` and `body`.
    #[must_use]
    pub fn json(status: u16, body: String) -> Self {
        Self {
            status,
            content_type: "application/json; charset=utf-8",
            body: body.into_bytes(),
        }
    }

    /// A plain-text response, used for the statuses that carry no data.
    #[must_use]
    pub fn text(status: u16, body: &str) -> Self {
        Self {
            status,
            content_type: "text/plain; charset=utf-8",
            body: body.as_bytes().to_vec(),
        }
    }
}

/// The reason phrase for `status`, for the small set of statuses this server sends.
fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        413 => "Payload Too Large",
        _ => "Internal Server Error",
    }
}

/// How long one whole request may take to arrive, however it is paced.
///
/// A per-read timeout is not a request budget: a client that writes one byte just inside the
/// read timeout, forever, is never idle and so is never dropped. This is the deadline that
/// actually bounds a connection's lifetime.
pub const REQUEST_BUDGET: std::time::Duration = std::time::Duration::from_secs(10);

/// Read one request from `stream`, giving up at `deadline`.
///
/// # Errors
/// When the connection ends before a complete head arrives, when the head or body exceeds the
/// limits above, when the deadline passes, or when the request line is not `METHOD PATH
/// VERSION`.
pub fn read<R: Read>(stream: &mut BufReader<R>, deadline: Instant) -> Result<Request, String> {
    let mut head = String::new();
    loop {
        if Instant::now() >= deadline {
            return Err("the request took too long to arrive".to_string());
        }
        // Bounded as it is read, not after: `read_line` on a peer that never sends a newline
        // grows the string until the machine runs out of memory, and a limit checked once the
        // line is complete never gets the chance to say no.
        let remaining = MAX_HEAD.saturating_sub(head.len());
        if remaining == 0 {
            return Err(format!("the request head is longer than {MAX_HEAD} bytes"));
        }
        let line = read_line_bounded(stream, remaining)?;
        if line.is_empty() {
            return Err("the connection closed before the request was complete".to_string());
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        head.push_str(&line);
    }

    let mut lines = head.lines();
    let start = lines
        .next()
        .ok_or_else(|| "the request had no request line".to_string())?;
    let mut parts = start.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| format!("`{start}` is not a request line"))?
        .to_uppercase();
    let target = parts
        .next()
        .ok_or_else(|| format!("`{start}` is not a request line"))?;
    let path = target
        .split_once('?')
        .map_or(target, |(before, _)| before)
        .to_string();

    let mut headers = BTreeMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_lowercase(), value.trim().to_string());
        }
    }

    let length = match headers.get("content-length") {
        None => 0,
        Some(value) => value
            .parse::<usize>()
            .map_err(|_| format!("`{value}` is not a content length"))?,
    };
    if length > MAX_BODY {
        return Err(format!("the body is longer than {MAX_BODY} bytes"));
    }
    // Grown as the bytes arrive rather than reserved from the declared length: a caller that
    // claims 64 MiB and sends nothing should cost nothing, and several of them at once should
    // not reserve the machine's memory between them.
    let mut body = Vec::new();
    stream
        .take(length as u64)
        .read_to_end(&mut body)
        .map_err(|error| format!("reading the request body: {error}"))?;
    if body.len() != length {
        return Err(format!(
            "the request declared {length} bytes of body and sent {}",
            body.len()
        ));
    }
    if Instant::now() >= deadline {
        return Err("the request took too long to arrive".to_string());
    }

    Ok(Request {
        method,
        path,
        headers,
        body,
    })
}

/// Read one line, refusing to accumulate more than `limit` bytes.
///
/// Returns the line including its terminator, or an empty string at end of input.
fn read_line_bounded<R: Read>(stream: &mut BufReader<R>, limit: usize) -> Result<String, String> {
    let mut line = Vec::new();
    loop {
        let available = stream
            .fill_buf()
            .map_err(|error| format!("reading the request: {error}"))?;
        if available.is_empty() {
            break;
        }
        let take = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |at| at + 1);
        let ends_line = available.get(take - 1) == Some(&b'\n');
        if line.len() + take > limit {
            return Err(format!("the request head is longer than {MAX_HEAD} bytes"));
        }
        line.extend_from_slice(&available[..take]);
        stream.consume(take);
        if ends_line {
            break;
        }
    }
    String::from_utf8(line).map_err(|_| "the request head is not utf-8".to_string())
}

/// Whether `host` names this machine.
///
/// # Why this is the load-bearing check
/// A page cannot forge a `Host` header, and DNS rebinding — a name that resolves to the
/// attacker's server and then to `127.0.0.1`, so the browser treats a loopback service as
/// same-origin and lets the page read every response — still sends the *attacker's* name here.
/// Requiring loopback is therefore what actually stops it; the origin check below is the
/// second lock, not the first.
#[must_use]
pub fn is_loopback_host(host: &str) -> bool {
    let name = host
        .rsplit_once(':')
        .map_or(host, |(before, after)| {
            // Only a trailing *port* may be split off; `[::1]` has colons of its own.
            if after.chars().all(|ch| ch.is_ascii_digit()) && !after.is_empty() {
                before
            } else {
                host
            }
        })
        .trim();
    matches!(name, "127.0.0.1" | "localhost" | "[::1]" | "::1")
}

/// Whether `origin` may talk to the daemon.
///
/// Only the browser extension, which is the one web client there is. A page on any site — even
/// github.com — has no business here: the extension reaches the daemon from its own background
/// context, so its origin is the extension's, never the page's.
#[must_use]
pub fn is_allowed_origin(origin: &str) -> bool {
    [
        "chrome-extension://",
        "moz-extension://",
        "safari-web-extension://",
    ]
    .iter()
    .any(|scheme| origin.starts_with(scheme))
        && is_header_safe(origin)
}

/// Whether a value can be written into a header without changing the head's shape.
///
/// Anything reflected back to a caller is checked first: `head.lines()` leaves a bare `\r`
/// inside a value, and interpolating that into the response would let a caller append headers
/// of its own.
fn is_header_safe(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .chars()
            .all(|ch| ch.is_ascii_graphic() && ch != '\r' && ch != '\n')
}

/// Write `response` to `stream`, allowing `origin` to read it.
///
/// # Why every response carries permission headers
/// The client is a browser extension on a page served over https, reaching a loopback
/// service. Two separate browser rules govern that: the cross-origin check, answered by
/// echoing the caller's origin, and Chrome's private-network check, which additionally
/// requires the response to say that a public page may address a local server. Omitting
/// either makes the request fail before the daemon's answer is ever read.
///
/// `origin` is echoed only after [`is_allowed_origin`] has accepted it, so the value written
/// here is always one of a known set of extension origins. It is *not* echoed for anyone else:
/// the daemon holds packs the reader's browser uploaded, which can come from private
/// repositories, and granting an arbitrary site permission to read a response would hand those
/// documents to any page the reader happens to have open.
///
/// # Errors
/// When the socket cannot be written to.
pub fn write<W: Write>(
    stream: &mut W,
    response: &Response,
    origin: Option<&str>,
) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\n\
         content-type: {content_type}\r\n\
         content-length: {length}\r\n\
         {grant}\
         access-control-allow-methods: GET, POST, OPTIONS\r\n\
         access-control-allow-headers: content-type, x-dx-pack\r\n\
         access-control-allow-private-network: true\r\n\
         access-control-max-age: 600\r\n\
         vary: origin\r\n\
         cache-control: no-store\r\n\
         connection: close\r\n\
         \r\n",
        status = response.status,
        reason = reason(response.status),
        content_type = response.content_type,
        length = response.body.len(),
        // A caller the daemon did not recognize gets its answer with no
        // `access-control-allow-origin` at all. Writing the literal `null` here would not be
        // the refusal it reads as: a sandboxed-iframe page's origin *is* `null`, and a browser
        // will happily match the two. No header grants nothing to everyone.
        grant = origin
            .filter(|value| is_allowed_origin(value))
            .map(|value| format!("access-control-allow-origin: {value}\r\n"))
            .unwrap_or_default(),
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(&response.body)?;
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Read a request from a byte slice, as the socket path does.
    fn parse(raw: &str) -> Result<Request, String> {
        read(
            &mut BufReader::new(raw.as_bytes()),
            Instant::now() + REQUEST_BUDGET,
        )
    }

    #[test]
    fn a_request_line_and_headers_are_parsed_with_the_query_dropped() {
        let request = parse("GET /health?loud=1 HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
            .expect("a well-formed request parses");
        assert_eq!(request.method, "GET");
        assert_eq!(request.path, "/health");
        assert_eq!(request.header("host"), Some("127.0.0.1"));
        assert!(request.body.is_empty());
    }

    #[test]
    fn header_names_are_matched_however_the_client_capitalized_them() {
        let request = parse("GET / HTTP/1.1\r\nX-Dx-Pack: repo@main\r\n\r\n")
            .expect("a well-formed request parses");
        assert_eq!(request.header("x-dx-pack"), Some("repo@main"));
    }

    #[test]
    fn a_body_is_read_to_exactly_its_declared_length() {
        let request = parse("POST /pack HTTP/1.1\r\ncontent-length: 5\r\n\r\nhello")
            .expect("a well-formed request parses");
        assert_eq!(request.body, b"hello");
    }

    #[test]
    fn a_body_longer_than_the_limit_is_refused_rather_than_allocated() {
        let raw = format!(
            "POST /pack HTTP/1.1\r\ncontent-length: {}\r\n\r\n",
            MAX_BODY + 1
        );
        let error = parse(&raw).expect_err("an oversized body is refused");
        assert!(error.contains("longer than"), "{error}");
    }

    #[test]
    fn a_head_longer_than_the_limit_is_refused_rather_than_accumulated() {
        let filler = "x".repeat(MAX_HEAD);
        let raw = format!("GET / HTTP/1.1\r\nlong: {filler}\r\n\r\n");
        let error = parse(&raw).expect_err("an oversized head is refused");
        assert!(error.contains("longer than"), "{error}");
    }

    #[test]
    fn a_truncated_request_is_an_error_and_not_an_empty_one() {
        let error = parse("GET / HTTP/1.1\r\n").expect_err("a truncated request is refused");
        assert!(error.contains("closed"), "{error}");
    }

    /// The response head produced for `origin`.
    fn head_for(origin: Option<&str>) -> String {
        let mut out = Vec::new();
        write(&mut out, &Response::json(200, "{}".to_string()), origin)
            .expect("writing to a vector succeeds");
        String::from_utf8(out).expect("the head is utf-8")
    }

    #[test]
    fn the_extension_is_granted_the_response_and_the_private_network() {
        let text = head_for(Some("chrome-extension://abcdefghijklmnop"));
        assert!(text.starts_with("HTTP/1.1 200 OK\r\n"), "{text}");
        assert!(
            text.contains("access-control-allow-origin: chrome-extension://abcdefghijklmnop"),
            "{text}"
        );
        assert!(
            text.contains("access-control-allow-private-network: true"),
            "{text}"
        );
        assert!(text.ends_with("\r\n\r\n{}"), "{text}");
    }

    #[test]
    fn no_other_origin_is_ever_granted_permission_to_read_the_answer() {
        // The daemon holds packs the reader's browser uploaded, which can come from private
        // repositories. The header is omitted rather than set to `null`: a sandboxed frame's
        // origin *is* `null`, so echoing that literal would grant it the body.
        for origin in ["https://github.com", "https://evil.example", "null"] {
            let text = head_for(Some(origin));
            assert!(
                !text.contains("access-control-allow-origin"),
                "{origin} was granted: {text}"
            );
        }
        assert!(!head_for(None).contains("access-control-allow-origin"));
    }

    #[test]
    fn a_refused_response_carries_no_grant_a_foreign_page_could_use() {
        // The refusal itself must stay unreadable: a 403 that grants its origin would tell a
        // foreign page which requests the daemon distinguishes.
        let mut out = Vec::new();
        write(
            &mut out,
            &Response::json(403, "{}".to_string()),
            Some("https://evil.example"),
        )
        .expect("writing to a vector succeeds");
        let text = String::from_utf8(out).expect("the head is utf-8");
        assert!(text.starts_with("HTTP/1.1 403"), "{text}");
        assert!(!text.contains("access-control-allow-origin"), "{text}");
    }

    #[test]
    fn a_caller_cannot_append_headers_by_way_of_its_origin() {
        let text = head_for(Some("chrome-extension://abc\rX-Injected: yes"));
        assert!(!text.contains("X-Injected"), "{text}");
    }

    #[test]
    fn a_head_that_never_ends_is_refused_before_it_is_accumulated() {
        // `read_line` would grow this string until the machine ran out of memory; measured at
        // 1.7 GiB of resident memory from one connection before this was bounded.
        let endless = format!("GET / HTTP/1.1\r\nx: {}", "a".repeat(MAX_HEAD * 4));
        let error = parse(&endless).expect_err("an endless head is refused");
        assert!(error.contains("longer than"), "{error}");
    }

    #[test]
    fn a_body_shorter_than_it_claimed_is_an_error_rather_than_a_silent_truncation() {
        let error = parse("POST /pack HTTP/1.1\r\ncontent-length: 100\r\n\r\nshort")
            .expect_err("a short body is refused");
        assert!(error.contains("declared 100"), "{error}");
    }

    #[test]
    fn a_request_that_outruns_its_budget_is_dropped() {
        let raw = "GET /health HTTP/1.1\r\nhost: 127.0.0.1\r\n\r\n";
        let expired = Instant::now() - std::time::Duration::from_secs(1);
        let error = read(&mut BufReader::new(raw.as_bytes()), expired)
            .expect_err("an expired budget is refused");
        assert!(error.contains("too long"), "{error}");
    }
}
