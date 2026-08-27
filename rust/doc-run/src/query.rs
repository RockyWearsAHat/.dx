//! `lang=query` — read one field out of a sibling file or a live service, and record it as
//! `::output`. This is a *read*, not a *run*: it exists so an author reaching for "show this
//! document one field of live project state" (`stepsDone` out of a status file, `status` out
//! of a health endpoint) does not have to hand-write a parsing script the way `lang=bash`
//! blocks that shell out to `node -e '...'` do today — see `docs/dx-format-contract.dx` and
//! the report this block type answers (`report-26c0b880`). There is no author-supplied code
//! here at all, only two declarative attributes, so there is nothing for [`crate::confine`]
//! to sandbox beyond the one declared `src=`/`target=` reach itself — the same reasoning
//! [`crate::live`]'s module doc gives for `lang=capture` not going through `confine`/`plan`.
//!
//! `src=<path>` is resolved before this module ever runs: `resolve::hydrate` fills
//! `block.text` with the named sibling file's current text exactly the way it already does
//! for `::code src=`, so [`execute`] only ever sees text already sitting in the block.
//! `target=<url>` is this module's own job — a direct HTTP GET to exactly the host:port the
//! block names, no redirect ever followed (a redirect elsewhere would silently widen what
//! the block reaches past what review saw), and `http://` only: reaching an `https://`
//! target live is what `lang=capture`'s browser is for. This is a smaller boundary than
//! `live_proxy`'s CONNECT/absolute-form proxy needs to be, because there is no external
//! process (a whole browser) to route through it — this module *is* the one client, so
//! parsing the authority once ([`crate::live::target_authority`], shared with `lang=capture`)
//! and never opening a socket anywhere else is the whole of the scoping.
//!
//! `query=<dot.path>` (`a.b.c`, or `a.b[2].c` — a fixed grammar, never author code) is the
//! one field extracted from the JSON read and rendered as the block's `::output`. Approval
//! still gates every query block exactly like every other runnable block:
//! `lib.rs::approval_material` folds `target=` and `query=` into the fingerprint, so a
//! reviewer approves *where this reaches and what it extracts*, not just the body text —
//! editing either re-opens review the same way editing a capture's `target=` does.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use doc_core::model::Block;
use serde_json::Value;

use crate::live::target_authority;
use crate::process::Capture;

/// Response body byte cap. A query block reads one field, not a file — bounding it is
/// "bound anything sized by untrusted input" applied to a target the block does not fully
/// control.
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

/// Run a `lang=query` block: read `block.text` (already hydrated from `src=`, if named) or
/// fetch `block.target`, parse it as JSON, extract `block.query`'s dot-path, and record the
/// result as the block's output text.
pub(crate) fn execute(block: &Block, timeout: Duration) -> Capture {
    let query = block.query.trim();
    if query.is_empty() {
        return blocked(
            "a `query` block needs `query=<dot.path>` naming the field to extract, e.g. \
             `query=status.stepsDone`",
        );
    }
    let has_target = !block.target.trim().is_empty();
    let has_src = !block.src.trim().is_empty();
    if has_target && has_src {
        return blocked(
            "a `query` block names only one of `src=` or `target=` — a sibling file or a \
             live service, never both",
        );
    }

    let (text, origin) = if has_target {
        let target = block.target.trim();
        match fetch(target, timeout) {
            Ok(body) => (body, target.to_string()),
            Err(FetchError::Blocked(reason)) => return blocked(&reason),
            Err(FetchError::Failed(reason)) => {
                return Capture {
                    output: reason,
                    exit: 1,
                    timed_out: false,
                }
            }
        }
    } else if has_src {
        (block.text.clone(), block.src.trim().to_string())
    } else {
        return blocked(
            "a `query` block needs `src=<path>` naming a sibling file or `target=<url>` \
             naming a live service to read",
        );
    };

    let value: Value = match serde_json::from_str(text.trim()) {
        Ok(value) => value,
        Err(error) => {
            return Capture {
                output: format!("{origin} is not valid JSON: {error}"),
                exit: 1,
                timed_out: false,
            }
        }
    };

    match extract(&value, query) {
        Ok(found) => Capture {
            output: format_value(found),
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

/// A blocked query, in the same shape [`crate::blocked`] gives every other runner: a
/// configuration problem this block cannot even attempt, not a failure of the attempt.
fn blocked(message: &str) -> Capture {
    Capture {
        output: message.to_string(),
        exit: crate::BLOCKED_EXIT,
        timed_out: false,
    }
}

/// Why a `target=` fetch did not produce text: [`FetchError::Blocked`] for something the
/// block's own declaration rules out before any socket opens (an unsupported scheme, an
/// unparseable authority), [`FetchError::Failed`] for the attempt itself going wrong
/// (unreachable, timed out, a non-2xx status, a redirect this module refuses to follow).
enum FetchError {
    Blocked(String),
    Failed(String),
}

/// Fetch `target` with a direct, unproxied HTTP/1.1 GET, scoped to exactly the host:port it
/// names — the same authority parsing [`crate::live`] uses for a capture's browser, so a
/// query block's `target=` and a capture block's `target=` are read identically.
fn fetch(target: &str, timeout: Duration) -> Result<String, FetchError> {
    if !target.starts_with("http://") {
        return Err(FetchError::Blocked(format!(
            "a `query` block's `target=` must be an http:// URL naming a live service — an \
             https:// target is reached through a `lang=capture` block's browser instead: \
             {target}"
        )));
    }
    let authority = target_authority(target).map_err(FetchError::Blocked)?;
    let path = request_path(target);

    let address = format!("{}:{}", authority.host, authority.port);
    let socket_addr = address
        .to_socket_addrs()
        .map_err(|error| FetchError::Failed(format!("could not resolve {target}: {error}")))?
        .next()
        .ok_or_else(|| FetchError::Failed(format!("could not resolve {target}")))?;

    let mut stream = TcpStream::connect_timeout(&socket_addr, timeout)
        .map_err(|error| FetchError::Failed(format!("could not reach {target}: {error}")))?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|error| FetchError::Failed(format!("could not configure the fetch: {error}")))?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|error| FetchError::Failed(format!("could not configure the fetch: {error}")))?;

    let host_header = if authority.port == 80 || authority.port == 443 {
        authority.host.clone()
    } else {
        format!("{}:{}", authority.host, authority.port)
    };
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {host_header}\r\nAccept: application/json\r\n\
         User-Agent: dx-query/1\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|error| FetchError::Failed(format!("could not send the request: {error}")))?;

    let raw = read_bounded(&mut stream)
        .map_err(|error| FetchError::Failed(format!("could not read the response: {error}")))?;
    let (status, headers, mut body) = parse_response(&raw).ok_or_else(|| {
        FetchError::Failed(format!("{target} did not return a valid HTTP response"))
    })?;
    if is_chunked(&headers) {
        body = decode_chunked(&body).ok_or_else(|| {
            FetchError::Failed(format!("{target}'s chunked response could not be decoded"))
        })?;
    }
    if (300..400).contains(&status) {
        let location = header_value(&headers, "location").unwrap_or("(no Location header)");
        return Err(FetchError::Failed(format!(
            "{target} redirected ({status}) to {location} — a query block never follows a \
             redirect; name the final address in `target=` directly"
        )));
    }
    if !(200..300).contains(&status) {
        return Err(FetchError::Failed(format!(
            "{target} returned status {status}"
        )));
    }
    String::from_utf8(body).map_err(|error| {
        FetchError::Failed(format!("{target}'s response was not valid UTF-8: {error}"))
    })
}

/// `http://host:port/path?query` → `/path?query` (empty path becomes `/`).
fn request_path(target: &str) -> String {
    let rest = target.strip_prefix("http://").unwrap_or(target);
    match rest.find('/') {
        Some(index) => rest[index..].to_string(),
        None => "/".to_string(),
    }
}

/// Read a response off `stream` until it closes, bounded to [`MAX_RESPONSE_BYTES`] — a
/// query's response is one field's worth of JSON, not an arbitrary download.
fn read_bounded(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..read]);
        if buf.len() > MAX_RESPONSE_BYTES {
            return Err(std::io::Error::other(format!(
                "the response exceeded dx's {MAX_RESPONSE_BYTES}-byte limit for a query block"
            )));
        }
    }
    Ok(buf)
}

/// A response's status code, its lower-cased headers, and its body.
type ParsedResponse = (u16, Vec<(String, String)>, Vec<u8>);

/// Split a raw HTTP/1.1 response into its status code, lower-cased headers, and body.
fn parse_response(raw: &[u8]) -> Option<ParsedResponse> {
    let split = find_subslice(raw, b"\r\n\r\n")?;
    let head = std::str::from_utf8(&raw[..split]).ok()?;
    let body = raw[split + 4..].to_vec();
    let mut lines = head.split("\r\n");
    let status_line = lines.next()?;
    let mut parts = status_line.split_whitespace();
    let _version = parts.next()?;
    let status: u16 = parts.next()?.parse().ok()?;
    let mut headers = Vec::new();
    for line in lines {
        if let Some((key, value)) = line.split_once(':') {
            headers.push((key.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
    }
    Some((status, headers, body))
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

fn is_chunked(headers: &[(String, String)]) -> bool {
    header_value(headers, "transfer-encoding")
        .map(|value| value.to_ascii_lowercase().contains("chunked"))
        .unwrap_or(false)
}

/// Decode an HTTP chunked-transfer body into its plain bytes, bounded the same way an
/// ordinary response is.
fn decode_chunked(body: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut rest = body;
    loop {
        let line_end = find_subslice(rest, b"\r\n")?;
        let size_line = std::str::from_utf8(&rest[..line_end]).ok()?.trim();
        let size_str = size_line.split(';').next().unwrap_or(size_line);
        let size = usize::from_str_radix(size_str, 16).ok()?;
        rest = &rest[line_end + 2..];
        if size == 0 {
            break;
        }
        if rest.len() < size {
            return None;
        }
        out.extend_from_slice(&rest[..size]);
        rest = &rest[size..];
        if rest.len() < 2 {
            break;
        }
        rest = &rest[2..];
        if out.len() > MAX_RESPONSE_BYTES {
            return None;
        }
    }
    Some(out)
}

/// Walk `value` along `path`'s dot-separated segments (`a.b.c`), each an object key or,
/// when the segment parses as a number, an array index (`a.b[2].c` is normalized to
/// `a.b.2.c` before this runs). A fixed grammar, never author code: there is no operator,
/// no wildcard, no comparison — only "go to the named field, or the numbered item."
fn extract<'a>(value: &'a Value, path: &str) -> Result<&'a Value, String> {
    let normalized = path.replace('[', ".").replace(']', "");
    let mut current = value;
    let mut walked: Vec<&str> = Vec::new();
    for segment in normalized
        .split('.')
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let next = match current {
            Value::Object(map) => map.get(segment).ok_or_else(|| {
                format!(
                    "no field named `{segment}` at `{}` — the value there is an object \
                     with keys: {}",
                    display_path(&walked),
                    join_keys(map)
                )
            })?,
            Value::Array(list) => {
                let index: usize = segment.parse().map_err(|_| {
                    format!(
                        "`{segment}` is not a valid array index at `{}`",
                        display_path(&walked)
                    )
                })?;
                list.get(index).ok_or_else(|| {
                    format!(
                        "index {index} is out of range at `{}` — the array has {} item(s)",
                        display_path(&walked),
                        list.len()
                    )
                })?
            }
            other => {
                return Err(format!(
                    "`{}` cannot be indexed by `{segment}` — it is {}, not an object or \
                     array",
                    display_path(&walked),
                    kind_name(other)
                ))
            }
        };
        walked.push(segment);
        current = next;
    }
    Ok(current)
}

fn display_path(walked: &[&str]) -> String {
    if walked.is_empty() {
        "the root".to_string()
    } else {
        walked.join(".")
    }
}

fn join_keys(map: &serde_json::Map<String, Value>) -> String {
    let mut keys: Vec<&str> = map.keys().map(String::as_str).collect();
    keys.sort_unstable();
    keys.join(", ")
}

fn kind_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// Render an extracted value as the block's output text: a scalar prints bare (a string
/// with no quotes, a number or boolean in its plain form), and an object or array — the
/// query landed on a substructure, not a leaf — pretty-prints as JSON.
fn format_value(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => "null".to_string(),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(number) => number.to_string(),
        other => serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(text: &str) -> Value {
        serde_json::from_str(text).expect("valid JSON fixture")
    }

    #[test]
    fn extracts_a_nested_field_by_dot_path() {
        let value = json(r#"{"status":{"stepsDone":7,"ok":true}}"#);
        assert_eq!(extract(&value, "status.stepsDone").unwrap(), &json("7"));
        assert_eq!(extract(&value, "status.ok").unwrap(), &json("true"));
    }

    #[test]
    fn extracts_an_array_index_with_bracket_or_dot_syntax() {
        let value = json(r#"{"items":[{"name":"a"},{"name":"b"}]}"#);
        assert_eq!(
            extract(&value, "items[1].name").unwrap(),
            &Value::String("b".to_string())
        );
        assert_eq!(
            extract(&value, "items.1.name").unwrap(),
            &Value::String("b".to_string())
        );
    }

    #[test]
    fn a_missing_field_names_the_available_keys() {
        let value = json(r#"{"a":1,"b":2}"#);
        let error = extract(&value, "c").unwrap_err();
        assert!(error.contains('c'), "{error}");
        assert!(error.contains('a') && error.contains('b'), "{error}");
    }

    #[test]
    fn an_out_of_range_index_says_the_arrays_length() {
        let value = json(r#"{"items":[1,2]}"#);
        let error = extract(&value, "items.9").unwrap_err();
        assert!(error.contains('2'), "{error}");
    }

    #[test]
    fn indexing_a_scalar_is_a_named_error() {
        let value = json(r#"{"a":1}"#);
        let error = extract(&value, "a.b").unwrap_err();
        assert!(error.contains("number"), "{error}");
    }

    #[test]
    fn a_scalar_formats_bare_and_a_substructure_pretty_prints() {
        assert_eq!(format_value(&json("7")), "7");
        assert_eq!(format_value(&json("true")), "true");
        assert_eq!(format_value(&json("\"hi\"")), "hi");
        assert_eq!(format_value(&json("null")), "null");
        assert!(format_value(&json(r#"{"x":1}"#)).contains('\n'));
    }

    #[test]
    fn a_block_with_no_query_attribute_is_blocked_not_run() {
        let block = Block {
            kind: "code".to_string(),
            id: "q".to_string(),
            language: "query".to_string(),
            run: true,
            src: "status.json".to_string(),
            ..Block::default()
        };
        let capture = execute(&block, Duration::from_secs(5));
        assert_eq!(capture.exit, crate::BLOCKED_EXIT);
        assert!(capture.output.contains("query="), "{}", capture.output);
    }

    #[test]
    fn a_block_naming_neither_src_nor_target_is_blocked_not_run() {
        let block = Block {
            kind: "code".to_string(),
            id: "q".to_string(),
            language: "query".to_string(),
            run: true,
            query: "a".to_string(),
            ..Block::default()
        };
        let capture = execute(&block, Duration::from_secs(5));
        assert_eq!(capture.exit, crate::BLOCKED_EXIT);
        assert!(capture.output.contains("src="), "{}", capture.output);
        assert!(capture.output.contains("target="), "{}", capture.output);
    }

    #[test]
    fn a_block_naming_both_src_and_target_is_blocked_not_run() {
        let block = Block {
            kind: "code".to_string(),
            id: "q".to_string(),
            language: "query".to_string(),
            run: true,
            query: "a".to_string(),
            src: "status.json".to_string(),
            target: "http://127.0.0.1:1/".to_string(),
            ..Block::default()
        };
        let capture = execute(&block, Duration::from_secs(5));
        assert_eq!(capture.exit, crate::BLOCKED_EXIT);
        assert!(capture.output.contains("only one"), "{}", capture.output);
    }

    #[test]
    fn a_hydrated_src_block_extracts_its_field() {
        let block = Block {
            kind: "code".to_string(),
            id: "q".to_string(),
            language: "query".to_string(),
            run: true,
            query: "stepsDone".to_string(),
            src: "status.json".to_string(),
            text: r#"{"stepsDone": 3}"#.to_string(),
            ..Block::default()
        };
        let capture = execute(&block, Duration::from_secs(5));
        assert_eq!(capture.exit, 0);
        assert_eq!(capture.output, "3");
    }

    #[test]
    fn invalid_json_from_src_is_a_failure_not_a_block() {
        let block = Block {
            kind: "code".to_string(),
            id: "q".to_string(),
            language: "query".to_string(),
            run: true,
            query: "a".to_string(),
            src: "status.json".to_string(),
            text: "not json".to_string(),
            ..Block::default()
        };
        let capture = execute(&block, Duration::from_secs(5));
        assert_eq!(capture.exit, 1);
        assert!(
            capture.output.contains("not valid JSON"),
            "{}",
            capture.output
        );
    }

    #[test]
    fn an_https_target_is_blocked_not_attempted() {
        let block = Block {
            kind: "code".to_string(),
            id: "q".to_string(),
            language: "query".to_string(),
            run: true,
            query: "a".to_string(),
            target: "https://example.com/".to_string(),
            ..Block::default()
        };
        let capture = execute(&block, Duration::from_secs(5));
        assert_eq!(capture.exit, crate::BLOCKED_EXIT);
        assert!(capture.output.contains("http://"), "{}", capture.output);
    }

    #[test]
    fn request_path_defaults_to_root_and_keeps_the_query_string() {
        assert_eq!(request_path("http://127.0.0.1:9/health?x=1"), "/health?x=1");
        assert_eq!(request_path("http://127.0.0.1:9"), "/");
    }

    #[test]
    fn chunked_bodies_decode_to_their_plain_bytes() {
        let chunked = b"4\r\ndata\r\n0\r\n\r\n";
        assert_eq!(decode_chunked(chunked).unwrap(), b"data".to_vec());
    }
}
