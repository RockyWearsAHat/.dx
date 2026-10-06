//! What a block's run fingerprint costs to know again, kept so a second look over
//! unchanged inputs does almost nothing.
//!
//! Two layers, both exact:
//!
//! - **The read cache.** The run fingerprint embeds each declared file's *text* (a binary
//!   file's digest, as the resolver hands it back), not a per-file digest, so this holds
//!   exactly what the resolver returned. A file is keyed by its absolute path and its stamp
//!   (inode, size and mtime in nanoseconds; path, size and mtime where there are no inodes).
//!   A folder is keyed by the stamps of every file the resolver's walk reads: the walk is
//!   mirrored here (hidden entries and `target`/`node_modules` left out), statting only. A
//!   path absent on disk can still be answered by the workspace store, so "no answer" for
//!   it is keyed on the store's own stamps (`.doc/index.db`, its WAL, `repo.dxcp`).
//! - **The fingerprint memo.** A block's fingerprint, keyed by everything it is computed
//!   from except file text — runner, code, dependencies, write grant, host, timeout — plus
//!   the stamps of every path its `reads=` resolution could consult (the path under the
//!   document's folder, the same path under the workspace root, and the store when either
//!   is absent or names a `.dx` document). Equal stamps mean the resolver would hand back
//!   equal text, so the memoized value is byte-identical to a fresh one; a warm call is
//!   stat calls only and hashes no file bytes. The memo holds digests, never text, so it
//!   persists: [`load_memo`] / [`save_memo`] keep it in a small file a fresh process starts
//!   from.
//!
//! Only a resolver that answers from disk ([`Resolver::on_disk`]) is cached at all; any
//! other is asked every time.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::cell::Cell;
use std::collections::HashSet;

use doc_core::digest::sha256_hex;

/// What identifies one file's content without reading it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    inode: u64,
    size: u64,
    mtime_ns: i128,
}

#[cfg(unix)]
fn stamp(meta: &fs::Metadata) -> Stamp {
    use std::os::unix::fs::MetadataExt;
    Stamp {
        inode: meta.ino(),
        size: meta.size(),
        mtime_ns: i128::from(meta.mtime()) * 1_000_000_000 + i128::from(meta.mtime_nsec()),
    }
}

#[cfg(not(unix))]
fn stamp(meta: &fs::Metadata) -> Stamp {
    let mtime_ns = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos() as i128);
    Stamp {
        inode: 0,
        size: meta.len(),
        mtime_ns,
    }
}

/// The stamps a cached value was read under: one unnamed entry for a file, one per walked
/// file (relative to the folder) for a folder.
type Key = Vec<(String, Stamp)>;

#[derive(Clone)]
enum Value {
    File(String),
    /// The resolver produced no file text for this path, a folder on disk.
    NotAFile,
    Tree(Vec<(String, String)>),
}

/// Which question a path was asked: its file text, or its folder walk. The same folder is
/// asked both (a `reads=` path is tried as a file first), so the answers are kept apart.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Ask {
    File,
    Tree,
}

fn cache() -> &'static Mutex<HashMap<(PathBuf, Ask), (Key, Value)>> {
    static CACHE: OnceLock<Mutex<HashMap<(PathBuf, Ask), (Key, Value)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lookup(ask: Ask, path: &Path, key: &Key) -> Option<Value> {
    let map = cache()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    map.get(&(path.to_path_buf(), ask))
        .filter(|(held, _)| held == key)
        .map(|(_, value)| value.clone())
}

fn store(ask: Ask, path: PathBuf, key: Key, value: Value) {
    cache()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert((path, ask), (key, value));
}

/// `read()` for the file `relative` names under `base`, answered from the cache when the
/// file's stamp is unchanged since the last read. Without a base, or when the path is not
/// a file on disk, `read` is simply called.
pub(crate) fn file(
    base: Option<&Path>,
    relative: &str,
    read: impl FnOnce() -> Option<String>,
) -> Option<String> {
    let Some(path) = base.map(|b| b.join(relative)) else {
        return read();
    };
    let Ok(meta) = fs::metadata(&path) else {
        // Absent on disk: only the workspace store could answer, so "no answer" is kept
        // under the store's stamps and asked again once the store changes.
        let key = store_stamps(&path);
        if let Some(Value::NotAFile) = lookup(Ask::File, &path, &key) {
            return None;
        }
        let text = read();
        if text.is_none() {
            store(Ask::File, path, key, Value::NotAFile);
        }
        return text;
    };
    let key = vec![(String::new(), stamp(&meta))];
    if meta.is_dir() {
        // A folder is not a file: the resolver's answer is "no file here", and asking it
        // costs a store open. That answer is kept under the folder's own stamp.
        if let Some(Value::NotAFile) = lookup(Ask::File, &path, &key) {
            return None;
        }
        let text = read();
        if text.is_none() {
            store(Ask::File, path, key, Value::NotAFile);
        }
        return text;
    }
    if !meta.is_file() {
        return read();
    }
    if let Some(Value::File(text)) = lookup(Ask::File, &path, &key) {
        return Some(text);
    }
    let text = read()?;
    store(Ask::File, path, key, Value::File(text.clone()));
    Some(text)
}

/// `walk()` for the folder `relative` names under `base`, answered from the cache when
/// every file under it carries the stamp it had at the last walk.
pub(crate) fn tree(
    base: Option<&Path>,
    relative: &str,
    walk: impl FnOnce() -> Option<Vec<(String, String)>>,
) -> Option<Vec<(String, String)>> {
    let Some(path) = base.map(|b| b.join(relative)) else {
        return walk();
    };
    if !path.is_dir() {
        return walk();
    }
    let mut key = Vec::new();
    stamps_under(&path, "", &mut key);
    key.sort_by(|a, b| a.0.cmp(&b.0));
    if let Some(Value::Tree(files)) = lookup(Ask::Tree, &path, &key) {
        return Some(files);
    }
    let files = walk()?;
    store(Ask::Tree, path, key, Value::Tree(files.clone()));
    Some(files)
}

/// The stamp of every file the CLI resolver's folder walk reads under `dir`: the same
/// entries it visits (hidden names and `target`/`node_modules` folders left out).
fn stamps_under(dir: &Path, prefix: &str, out: &mut Key) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        let held = format!("{prefix}/{name}");
        let Ok(meta) = fs::metadata(&path) else {
            // Vanished or dangling: present as an entry the walk saw, so its return misses.
            out.push((
                held,
                Stamp {
                    inode: 0,
                    size: 0,
                    mtime_ns: -1,
                },
            ));
            continue;
        };
        if meta.is_dir() {
            if name != "target" && name != "node_modules" {
                stamps_under(&path, &held, out);
            }
        } else {
            out.push((held, stamp(&meta)));
        }
    }
}

/// The stamp an absent file is recorded with.
const ABSENT: Stamp = Stamp {
    inode: 0,
    size: 0,
    mtime_ns: -1,
};

fn stamp_of(path: &Path) -> Stamp {
    fs::metadata(path).map_or(ABSENT, |meta| stamp(&meta))
}

/// The workspace root the CLI resolver opens the store of for `path`: the nearest folder
/// at or above it holding a `.doc` folder or a `.git` entry (`workspace::workspace_root`).
fn marker_root(path: &Path) -> PathBuf {
    let absolute = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mut cursor: &Path = if absolute.is_dir() {
        &absolute
    } else {
        absolute.parent().unwrap_or(&absolute)
    };
    loop {
        if cursor.join(".doc").is_dir() || cursor.join(".git").exists() {
            return cursor.to_path_buf();
        }
        match cursor.parent() {
            Some(parent) if parent != cursor => cursor = parent,
            _ => return cursor.to_path_buf(),
        }
    }
}

/// The stamps of the store files that can answer for `path` when it is not on disk.
fn store_stamps(path: &Path) -> Key {
    let doc = marker_root(path).join(".doc");
    ["index.db", "index.db-wal", "repo.dxcp"]
        .iter()
        .map(|name| {
            let file = doc.join(name);
            (file.to_string_lossy().into_owned(), stamp_of(&file))
        })
        .collect()
}

/// Append a textual stamp of `path` (file, folder walk, or absent) to `out`; true when the
/// path is absent, so only the store could answer for it.
fn describe(path: &Path, out: &mut String) -> bool {
    use std::fmt::Write as _;
    let Ok(meta) = fs::metadata(path) else {
        out.push_str("\u{1f}a");
        return true;
    };
    if meta.is_dir() {
        let mut key = Vec::new();
        stamps_under(path, "", &mut key);
        key.sort_by(|a, b| a.0.cmp(&b.0));
        out.push_str("\u{1f}d");
        for (name, s) in key {
            let _ = write!(out, "\u{1e}{name}\u{1d}{} {} {}", s.inode, s.size, s.mtime_ns);
        }
    } else {
        let s = stamp(&meta);
        let _ = write!(out, "\u{1f}f {} {} {}", s.inode, s.size, s.mtime_ns);
    }
    false
}

/// Everything a block's fingerprint is computed from, except file text.
pub(crate) struct Identity<'a> {
    pub runner: &'a str,
    pub code: &'a str,
    pub deps: &'a [String],
    pub read_paths: &'a [String],
    pub writes: &'a [String],
    pub host: bool,
    pub timeout: u32,
}

/// The memo key of a block: its identity plus the stamps of every path its `reads=`
/// resolution could consult, or `None` when the resolver does not answer from disk.
///
/// `root_prefix` is the `../` climb from the document's folder to the workspace root, when
/// there is one: a declared path the folder lacks is looked for there, so both are stamped.
pub(crate) fn memo_key(
    identity: &Identity<'_>,
    document_dir: &Path,
    root_prefix: Option<&str>,
) -> String {
    use std::fmt::Write as _;
    let mut text = String::from("memo.v1");
    let _ = write!(
        text,
        "\u{1e}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}",
        identity.runner,
        identity.deps.join(","),
        identity.writes.join(","),
        identity.host,
        identity.timeout,
        identity.code
    );
    for path in identity.read_paths {
        let _ = write!(text, "\u{1e}{path}");
        let here = document_dir.join(path);
        let mut needs_store = describe(&here, &mut text) || path.ends_with(".dx");
        let climbs = path == ".." || path.starts_with("../");
        if let Some(up) = root_prefix.filter(|_| !climbs) {
            needs_store |= describe(&document_dir.join(format!("{up}{path}")), &mut text);
        }
        if needs_store {
            for (file, s) in store_stamps(&here) {
                let _ = write!(text, "\u{1e}{file}\u{1d}{} {} {}", s.inode, s.size, s.mtime_ns);
            }
        }
    }
    sha256_hex(text.as_bytes())
}

#[derive(Default)]
struct Memo {
    fingerprints: HashMap<String, String>,
    /// Keys used or added by this process — what a save keeps first.
    touched: HashSet<String>,
    /// Memo files already read into this process.
    loaded: HashSet<PathBuf>,
    dirty: bool,
}

fn memo() -> &'static Mutex<Memo> {
    static MEMO: OnceLock<Mutex<Memo>> = OnceLock::new();
    MEMO.get_or_init(|| Mutex::new(Memo::default()))
}

fn memo_lock() -> std::sync::MutexGuard<'static, Memo> {
    memo().lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(crate) fn memo_get(key: &str) -> Option<String> {
    let mut memo = memo_lock();
    let found = memo.fingerprints.get(key).cloned();
    if found.is_some() {
        memo.touched.insert(key.to_string());
    }
    found
}

pub(crate) fn memo_put(key: String, fingerprint: String) {
    let mut memo = memo_lock();
    memo.touched.insert(key.clone());
    if memo.fingerprints.get(&key) != Some(&fingerprint) {
        memo.fingerprints.insert(key, fingerprint);
        memo.dirty = true;
    }
}

/// Most entries a memo file keeps; those this process used are kept first.
const MEMO_CAP: usize = 20_000;

/// Read the memo file at `path` into this process, once per process (a missing or
/// unreadable file is an empty memo). Lines are `<key hex> <fingerprint hex>`.
pub fn load_memo(path: &Path) {
    if memo_lock().loaded.contains(path) {
        return;
    }
    let text = fs::read_to_string(path).unwrap_or_default();
    let mut memo = memo_lock();
    memo.loaded.insert(path.to_path_buf());
    for line in text.lines() {
        let mut parts = line.split(' ');
        let (Some(key), Some(value), None) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        let hex = |s: &str| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit());
        if hex(key) && hex(value) {
            memo.fingerprints
                .entry(key.to_string())
                .or_insert_with(|| value.to_string());
        }
    }
}

/// Write the memo to `path` (tmp file, then rename) when this process changed it since the
/// last save. Returns whether a file was written.
///
/// # Errors
/// The I/O error when the folder cannot be made or the file cannot be written.
pub fn save_memo(path: &Path) -> std::io::Result<bool> {
    use std::fmt::Write as _;
    let body = {
        let mut memo = memo_lock();
        if !memo.dirty {
            return Ok(false);
        }
        memo.dirty = false;
        let mut keys: Vec<&String> = memo.touched.iter().collect();
        keys.extend(memo.fingerprints.keys().filter(|k| !memo.touched.contains(*k)));
        let mut body = String::new();
        for key in keys.into_iter().take(MEMO_CAP) {
            if let Some(value) = memo.fingerprints.get(key) {
                let _ = writeln!(body, "{key} {value}");
            }
        }
        body
    };
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    fs::write(&tmp, body)?;
    fs::rename(&tmp, path)?;
    Ok(true)
}

/// Forget everything this process cached or loaded, as a fresh process would start.
/// For tests that measure the cold path; correctness never needs it.
#[doc(hidden)]
pub fn forget_all() {
    *memo_lock() = Memo::default();
    cache()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
}

thread_local! {
    static HASHED: Cell<u64> = const { Cell::new(0) };
}

/// Count `bytes` of fingerprint material hashed on this thread.
pub(crate) fn count_hashed(bytes: usize) {
    HASHED.with(|h| h.set(h.get() + bytes as u64));
}

/// Bytes of fingerprint material this thread has hashed so far — a test hook: a warm
/// look at unchanged inputs must not move it.
#[doc(hidden)]
#[must_use]
pub fn hashed_bytes() -> u64 {
    HASHED.with(Cell::get)
}
