//! Stub recognition, path normalization, identity, and timestamp helpers.
//!
//! `.dx` files on disk are tiny stub pointers (content lives in the bundle); these
//! free functions mirror the reference helpers in `src/doc-service.ts` and
//! `src/git-doc-state.ts` for recognizing stubs, normalizing caller-supplied paths,
//! deriving stable ids/titles, and formatting timestamps.

use doc_core::digest::sha1_hex;
use doc_core::model::Document as CoreDocument;

use super::{FALLBACK_TIMESTAMP, STUB_TINY_PREFIX};

/// Derive a numeric id from a workspace-relative path, matching `toStableDocumentId`.
///
/// Takes the first 8 hex chars of `sha1(relativePath)`, parses them as a `u32`, masks to
/// 31 bits, and floors the result to `1` so ids are positive and stable across runs.
pub(super) fn stable_document_id(relative_path: &str) -> i64 {
    let digest = sha1_hex(relative_path.as_bytes());
    let prefix = &digest[..8];
    let value = u32::from_str_radix(prefix, 16).unwrap_or(1) & 0x7fff_ffff;
    if value > 0 {
        i64::from(value)
    } else {
        1
    }
}

/// Choose a display title: the document's title, else its first heading, else the file
/// stem of the relative path (mirroring the reference fallback to the `.dx` basename).
pub(super) fn derive_title(document: &CoreDocument, relative_path: &str) -> String {
    if !document.title.is_empty() {
        return document.title.clone();
    }
    let heading = document.first_heading_text();
    if !heading.is_empty() {
        return heading.to_string();
    }
    file_stem(relative_path)
}

/// The final path segment with a trailing `.dx` extension removed.
fn file_stem(relative_path: &str) -> String {
    let name = relative_path.rsplit('/').next().unwrap_or(relative_path);
    name.strip_suffix(".dx").unwrap_or(name).to_string()
}

/// Whether `text` is a stub pointer rather than full document source.
///
/// Recognizes the tiny `~` / `@d3` forms and the legacy `@docstub` header, matching
/// `parseStubTarget` in `src/doc-service.ts`.
pub(super) fn is_stub(text: &str) -> bool {
    let first = text.lines().next().unwrap_or("").trim();
    first == STUB_TINY_PREFIX
        || first.starts_with("~ ")
        || first == "@d3"
        || first.starts_with("@d3 ")
        || first.starts_with("@docstub")
}

/// Normalize a caller-supplied path to a clean `/`-separated workspace-relative path.
///
/// Strips a leading `/`, rejects any `..` traversal (returning empty so callers raise an
/// invalid-argument error), and ensures a `.dx` extension.
pub(super) fn normalize_relative_path(path: &str) -> String {
    let trimmed = path.trim().trim_start_matches('/').replace('\\', "/");
    let mut parts = Vec::new();
    for segment in trimmed.split('/') {
        match segment {
            "" | "." => {}
            ".." => return String::new(),
            other => parts.push(other.to_string()),
        }
    }
    let joined = parts.join("/");
    if joined.is_empty() {
        return String::new();
    }
    if joined.ends_with(".dx") {
        joined
    } else {
        format!("{joined}.dx")
    }
}

/// Slugify `value` into a lowercase, hyphen-separated identifier (port of `toSlug`).
pub(super) fn slug(value: &str) -> String {
    let mut out = String::new();
    let mut prev_hyphen = false;
    for ch in value.trim().to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            prev_hyphen = false;
        } else if !prev_hyphen {
            out.push('-');
            prev_hyphen = true;
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "untitled".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Format a [`std::time::SystemTime`] as an ISO-8601 UTC timestamp with millisecond
/// precision (`YYYY-MM-DDTHH:MM:SS.mmmZ`), matching the reference's `Date.toISOString()`.
pub(super) fn iso8601_from_system_time(time: std::time::SystemTime) -> String {
    let duration = match time.duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => duration,
        Err(_) => return FALLBACK_TIMESTAMP.to_string(),
    };
    let total_secs = duration.as_secs();
    let millis = duration.subsec_millis();
    let days = total_secs / 86_400;
    let secs_of_day = total_secs % 86_400;
    let (hour, minute, second) = (
        secs_of_day / 3_600,
        (secs_of_day % 3_600) / 60,
        secs_of_day % 60,
    );
    let (year, month, day) = civil_from_days(days as i64);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

/// Convert days-since-Unix-epoch into a `(year, month, day)` civil date.
///
/// Uses Howard Hinnant's well-known `civil_from_days` algorithm (valid for the full
/// proleptic Gregorian range), so no date-handling dependency is needed.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
}
