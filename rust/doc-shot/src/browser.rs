//! Finding a browser that can take the picture.
//!
//! Screenshots use a Chromium-family browser that is already installed — Chrome, Edge,
//! Brave, or Chromium. Nothing is downloaded and no browser is bundled, so capture works
//! on a normal Mac or Windows machine without a setup step, and simply reports what to
//! install when none of them is present.

use std::path::PathBuf;

/// Environment variable that overrides browser discovery with an explicit path.
pub const BROWSER_ENV: &str = "DX_BROWSER";

/// Executables to look for on `PATH`, in preference order.
const PATH_CANDIDATES: &[&str] = &[
    "google-chrome",
    "google-chrome-stable",
    "chromium",
    "chromium-browser",
    "microsoft-edge",
    "brave-browser",
    "chrome",
];

/// Application paths to try on macOS, in preference order.
const MACOS_CANDIDATES: &[&str] = &[
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
];

/// Install paths to try on Windows, relative to the standard program directories.
const WINDOWS_CANDIDATES: &[&str] = &[
    r"Google\Chrome\Application\chrome.exe",
    r"Microsoft\Edge\Application\msedge.exe",
    r"BraveSoftware\Brave-Browser\Application\brave.exe",
    r"Chromium\Application\chrome.exe",
];

/// Locate a usable browser, or `None` when the machine has none.
///
/// `DX_BROWSER` wins when set, so a reader can point at any Chromium build they prefer.
#[must_use]
pub fn find() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os(BROWSER_ENV) {
        let path = PathBuf::from(explicit);
        if path.is_file() {
            return Some(path);
        }
    }
    platform_candidates()
        .into_iter()
        .find(|candidate| candidate.is_file())
        .or_else(find_on_path)
}

/// Well-known install locations for the current operating system.
fn platform_candidates() -> Vec<PathBuf> {
    if cfg!(target_os = "macos") {
        return MACOS_CANDIDATES.iter().map(PathBuf::from).collect();
    }
    if cfg!(windows) {
        return windows_candidates();
    }
    Vec::new()
}

/// Expand the Windows candidates against every standard program directory.
fn windows_candidates() -> Vec<PathBuf> {
    let roots = ["PROGRAMFILES", "PROGRAMFILES(X86)", "LOCALAPPDATA"];
    let mut paths = Vec::new();
    for root in roots {
        let Some(base) = std::env::var_os(root) else {
            continue;
        };
        for candidate in WINDOWS_CANDIDATES {
            paths.push(PathBuf::from(&base).join(candidate));
        }
    }
    paths
}

/// Search `PATH` for any of the known browser executable names.
fn find_on_path() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        for name in PATH_CANDIDATES {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// The message shown when no browser could be found.
#[must_use]
pub fn missing_message() -> String {
    format!(
        "no Chromium-family browser found for rendering images — install Google Chrome, \
         Microsoft Edge, Brave, or Chromium, or set {BROWSER_ENV} to a browser executable. \
         Text rendering works without one."
    )
}

/// `DX_BROWSER` is process-global, so every test that sets it — and every test that
/// *reads* it by launching a real capture — must take turns. A capture whose measuring
/// pass lands inside another test's bogus override silently measures nothing and
/// paginates blind.
#[cfg(test)]
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::ENV_LOCK;
    use super::*;

    #[test]
    fn an_explicit_override_is_used_when_it_exists() {
        let _turn = ENV_LOCK.lock().expect("env lock");
        let shell = if cfg!(windows) {
            "C:\\Windows\\System32\\cmd.exe"
        } else {
            "/bin/sh"
        };
        std::env::set_var(BROWSER_ENV, shell);
        assert_eq!(find(), Some(PathBuf::from(shell)));
        std::env::remove_var(BROWSER_ENV);
    }

    #[test]
    fn a_bogus_override_falls_back_to_discovery() {
        let _turn = ENV_LOCK.lock().expect("env lock");
        std::env::set_var(BROWSER_ENV, "/definitely/not/a/browser");
        // Either a real browser is found, or none is — but never the bogus path.
        assert_ne!(find(), Some(PathBuf::from("/definitely/not/a/browser")));
        std::env::remove_var(BROWSER_ENV);
    }

    #[test]
    fn missing_message_says_what_to_install_and_what_still_works() {
        let message = missing_message();
        assert!(message.contains("Chrome"));
        assert!(message.contains(BROWSER_ENV));
        assert!(message.contains("Text rendering works"));
    }
}
