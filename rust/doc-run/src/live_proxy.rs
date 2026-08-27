//! The proxy that makes "scoped to exactly the one host a capture names" true for an IP
//! literal, not only a hostname.
//!
//! [`doc_shot::cdp::Cdp::launch`]'s `allow_host` maps a *hostname* through
//! `--host-resolver-rules` and refuses every other — but Chromium does not consult the host
//! resolver for a numeric IP literal at all, so a `target=http://127.0.0.1:5173/` (the
//! overwhelmingly common shape a real dev server is named) would reach *any* port on
//! `127.0.0.1`, resolver rule or not. `doc-run/tests/capture_network.rs`'s
//! `a_capture_scripts_fetch_to_a_different_ip_literal_port_is_refused` is the test that
//! caught this the first time it was tried.
//!
//! A forward proxy closes it properly, because it checks the *actual destination requested*
//! rather than anything DNS-shaped: [`live::capture`] launches the browser with
//! `--proxy-server` pointed here, and this refuses to open a socket to anywhere except the
//! one `host:port` the capture named — hostname or IP literal alike, since the comparison
//! never goes through resolution at all.
//!
//! This is not a general-purpose proxy — it is written the same way [`crate::live`]'s
//! caller is its only caller, the same reasoning [`doc_shot::cdp`]'s own hand-rolled
//! WebSocket client gives for skipping a general client's negotiation: the one peer is
//! always the Chromium this process just launched, headed to exactly one destination.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

/// A running scoped proxy. Dropping this stops it within one poll tick.
pub(crate) struct Proxy {
    pub port: u16,
    stop: Arc<AtomicBool>,
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// Start a proxy on a kernel-chosen loopback port that allows a connection to exactly
/// `allowed_host:allowed_port` and refuses every other destination it is asked to reach,
/// whether asked for by hostname (an HTTP absolute-form request or a `CONNECT` for HTTPS)
/// or by IP literal.
///
/// # Errors
/// Returns a sentence when the listener cannot be bound.
pub(crate) fn start(allowed_host: String, allowed_port: u16) -> Result<Proxy, String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|error| format!("could not open the capture proxy: {error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("could not configure the capture proxy: {error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| format!("could not read the capture proxy's port: {error}"))?
        .port();

    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = Arc::clone(&stop);
    thread::spawn(move || {
        while !worker_stop.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((stream, _)) => {
                    let host = allowed_host.clone();
                    thread::spawn(move || {
                        let _ = handle(stream, &host, allowed_port);
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(20));
                }
                Err(_) => break,
            }
        }
    });

    Ok(Proxy { port, stop })
}

/// Read one proxy request off `client` and either tunnel it to the allowed destination or
/// refuse it — never opening a socket anywhere else, regardless of what the request asked.
fn handle(mut client: TcpStream, allowed_host: &str, allowed_port: u16) -> std::io::Result<()> {
    client.set_read_timeout(Some(Duration::from_secs(10)))?;
    let request = read_request_line(&mut client)?;
    let Some((method, target, headers)) = request else {
        return Ok(());
    };

    let destination = match method.as_str() {
        // `CONNECT host:port HTTP/1.1` — HTTPS and anything else Chromium tunnels.
        "CONNECT" => target.clone(),
        // `GET http://host:port/path HTTP/1.1` (and POST/HEAD/etc.) — plain HTTP proxied
        // in absolute-form, as every browser sends it to an explicit proxy.
        _ => match parse_absolute_form(&target) {
            Some(destination) => destination,
            None => {
                refuse(&mut client, 400, "Bad Request")?;
                return Ok(());
            }
        },
    };

    let Some((host, port)) = split_host_port(&destination) else {
        refuse(&mut client, 400, "Bad Request")?;
        return Ok(());
    };
    if !host.eq_ignore_ascii_case(allowed_host) || port != allowed_port {
        refuse(&mut client, 403, "Forbidden")?;
        return Ok(());
    }

    let Ok(mut upstream) = TcpStream::connect((host.as_str(), port)) else {
        refuse(&mut client, 502, "Bad Gateway")?;
        return Ok(());
    };

    if method == "CONNECT" {
        client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")?;
    } else {
        // Re-issue the request to the origin in origin-form — an origin server does not
        // expect the absolute-URI a proxy is sent.
        let origin_form = request_target_from_absolute(&target).unwrap_or(target.clone());
        upstream.write_all(format!("{method} {origin_form} HTTP/1.1\r\n").as_bytes())?;
        upstream.write_all(&headers)?;
        // Any already-buffered body bytes still need to reach the origin — relay drains
        // and forwards everything else below.
    }

    relay(client, upstream)
}

/// Copy bytes both directions until either side closes — the tunnel body, for both a
/// `CONNECT` (raw TLS) and a re-issued plain HTTP request (headers/body, then the
/// response).
fn relay(client: TcpStream, upstream: TcpStream) -> std::io::Result<()> {
    let client_read = client.try_clone()?;
    let upstream_write = upstream.try_clone()?;
    let to_upstream = thread::spawn(move || {
        let mut client_read = client_read;
        let mut upstream_write = upstream_write;
        let r = std::io::copy(&mut client_read, &mut upstream_write);
        if r.is_ok() {
            let _ = upstream_write.shutdown(std::net::Shutdown::Write);
        }
    });
    let mut upstream_read = upstream;
    let mut client_write = client;
    let _ = std::io::copy(&mut upstream_read, &mut client_write);
    let _ = client_write.shutdown(std::net::Shutdown::Write);
    let _ = to_upstream.join();
    Ok(())
}

/// Read one request line (`METHOD target HTTP/version`) plus the header block that follows
/// it. Returns `(method, target, headers)` — uppercased method, and the raw header bytes
/// (everything after the request line's trailing `\r\n`, including the final blank line) for
/// [`handle`] to forward verbatim to the origin.
fn read_request_line(client: &mut TcpStream) -> std::io::Result<Option<(String, String, Vec<u8>)>> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    // A request line plus headers is small; reading byte-by-byte to find `\r\n\r\n` keeps
    // this proxy's parser tiny and avoids buffering a body it does not need to inspect.
    loop {
        if client.read(&mut byte)? == 0 {
            return Ok(None);
        }
        buf.push(byte[0]);
        if buf.len() >= 4 && &buf[buf.len() - 4..] == b"\r\n\r\n" {
            break;
        }
        if buf.len() > 64 * 1024 {
            return Ok(None);
        }
    }
    let Some(split_point) = buf.windows(2).position(|w| w == b"\r\n") else {
        return Ok(None);
    };
    let first_line = String::from_utf8_lossy(&buf[..split_point]);
    let mut parts = first_line.split_whitespace();
    let (Some(method), Some(target)) = (parts.next(), parts.next()) else {
        return Ok(None);
    };
    let headers = buf[split_point + 2..].to_vec();
    Ok(Some((
        method.to_ascii_uppercase(),
        target.to_string(),
        headers,
    )))
}

/// `http://host:port/path` → `host:port`.
fn parse_absolute_form(target: &str) -> Option<String> {
    let rest = target
        .strip_prefix("http://")
        .or_else(|| target.strip_prefix("https://"))?;
    let authority = rest.split(['/', '?', '#']).next()?;
    Some(authority.to_string())
}

/// `http://host:port/path?query` → `/path?query` (empty path becomes `/`).
fn request_target_from_absolute(target: &str) -> Option<String> {
    let rest = target
        .strip_prefix("http://")
        .or_else(|| target.strip_prefix("https://"))?;
    let slash = rest.find('/');
    Some(match slash {
        Some(index) => rest[index..].to_string(),
        None => "/".to_string(),
    })
}

/// `host:port` (or `[ipv6]:port`) → `(host, port)`.
fn split_host_port(destination: &str) -> Option<(String, u16)> {
    if let Some(rest) = destination.strip_prefix('[') {
        let (host, rest) = rest.split_once(']')?;
        let port = rest.strip_prefix(':')?.parse().ok()?;
        return Some((format!("[{host}]"), port));
    }
    let (host, port) = destination.rsplit_once(':')?;
    Some((host.to_string(), port.parse().ok()?))
}

/// Send a bare status line refusing the request; the client sees a proxy error, not the
/// destination's own response, because there is no destination — nothing was ever opened.
fn refuse(client: &mut TcpStream, status: u16, reason: &str) -> std::io::Result<()> {
    client.write_all(format!("HTTP/1.1 {status} {reason}\r\nConnection: close\r\n\r\n").as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_absolute_form_and_strips_it_to_origin_form() {
        assert_eq!(
            parse_absolute_form("http://127.0.0.1:9/path?x=1"),
            Some("127.0.0.1:9".to_string())
        );
        assert_eq!(
            request_target_from_absolute("http://127.0.0.1:9/path?x=1"),
            Some("/path?x=1".to_string())
        );
        assert_eq!(
            request_target_from_absolute("http://127.0.0.1:9"),
            Some("/".to_string())
        );
    }

    #[test]
    fn splits_host_and_port_including_a_bracketed_ipv6_literal() {
        assert_eq!(
            split_host_port("127.0.0.1:9"),
            Some(("127.0.0.1".to_string(), 9))
        );
        assert_eq!(split_host_port("[::1]:9"), Some(("[::1]".to_string(), 9)));
        assert_eq!(split_host_port("no-port"), None);
    }

    #[test]
    fn a_connection_to_the_allowed_destination_is_relayed() {
        let target = TcpListener::bind("127.0.0.1:0").expect("bind target");
        let target_port = target.local_addr().expect("addr").port();
        thread::spawn(move || {
            if let Ok((mut stream, _)) = target.accept() {
                let mut buf = [0u8; 256];
                let _ = stream.read(&mut buf);
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok");
            }
        });

        let proxy = start("127.0.0.1".to_string(), target_port).expect("start proxy");
        let mut client = TcpStream::connect(("127.0.0.1", proxy.port)).expect("connect proxy");
        client
            .write_all(
                format!(
                    "GET http://127.0.0.1:{target_port}/ HTTP/1.1\r\nHost: 127.0.0.1:{target_port}\r\n\r\n"
                )
                .as_bytes(),
            )
            .expect("write");
        let mut response = String::new();
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("timeout");
        let _ = client.read_to_string(&mut response);
        assert!(response.contains("200"), "{response}");
        assert!(response.ends_with("ok"), "{response}");
    }

    #[test]
    fn a_connection_to_anywhere_else_is_refused_before_a_socket_opens() {
        let forbidden = TcpListener::bind("127.0.0.1:0").expect("bind forbidden");
        let forbidden_port = forbidden.local_addr().expect("addr").port();
        let reached = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&reached);
        thread::spawn(move || {
            if forbidden.accept().is_ok() {
                flag.store(true, Ordering::SeqCst);
            }
        });

        // The proxy allows only port `allowed_port` (nothing is listening there — it must
        // never be dialed either way for this test to prove anything).
        let proxy = start("127.0.0.1".to_string(), 1).expect("start proxy");
        let mut client = TcpStream::connect(("127.0.0.1", proxy.port)).expect("connect proxy");
        client
            .write_all(
                format!(
                    "GET http://127.0.0.1:{forbidden_port}/ HTTP/1.1\r\nHost: 127.0.0.1:{forbidden_port}\r\n\r\n"
                )
                .as_bytes(),
            )
            .expect("write");
        let mut response = String::new();
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("timeout");
        let _ = client.read_to_string(&mut response);
        assert!(response.contains("403"), "{response}");

        thread::sleep(Duration::from_millis(200));
        assert!(
            !reached.load(Ordering::SeqCst),
            "the proxy opened a socket to a destination it was not asked to allow"
        );
    }
}
