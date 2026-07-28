//! Finding the tools a document needs, and saying so plainly when they are missing.
//!
//! Nothing here is bundled or downloaded. A `.dx` document runs Python with *your* Python
//! and Node with *your* Node, so a block behaves the same as the same code pasted into a
//! terminal. When a tool is absent the answer is a clear sentence naming what to install,
//! not a stack trace.

use std::path::{Path, PathBuf};

/// Locate `program` on the current `PATH`, returning its full path.
///
/// On Windows every extension in `PATHEXT` is tried, so `node` finds `node.exe`.
#[must_use]
pub fn locate(program: &str) -> Option<PathBuf> {
    if program.contains(std::path::MAIN_SEPARATOR) {
        let direct = PathBuf::from(program);
        return direct.is_file().then_some(direct);
    }

    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        if let Some(found) = probe(&directory, program) {
            return Some(found);
        }
    }
    None
}

/// Whether `program` is available on this machine.
#[must_use]
pub fn have(program: &str) -> bool {
    locate(program).is_some()
}

/// Return the first available program from `candidates`.
#[must_use]
pub fn first_available(candidates: &[&str]) -> Option<String> {
    candidates
        .iter()
        .find(|program| have(program))
        .map(|program| (*program).to_string())
}

/// Try `program` in `directory`, including Windows executable extensions.
fn probe(directory: &Path, program: &str) -> Option<PathBuf> {
    let plain = directory.join(program);
    if plain.is_file() {
        return Some(plain);
    }
    for extension in windows_extensions() {
        let candidate = directory.join(format!("{program}{extension}"));
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Executable extensions to try, from `PATHEXT`. Empty on non-Windows systems.
fn windows_extensions() -> Vec<String> {
    if !cfg!(windows) {
        return Vec::new();
    }
    std::env::var("PATHEXT")
        .unwrap_or_else(|_| ".EXE;.CMD;.BAT;.COM".to_string())
        .split(';')
        .map(|extension| extension.trim().to_string())
        .filter(|extension| !extension.is_empty())
        .collect()
}

/// A sentence telling the reader exactly what to install for `language`.
#[must_use]
pub fn missing_toolchain_message(language: &str, wanted: &[&str]) -> String {
    let names = wanted.join(" or ");
    format!(
        "no {language} toolchain found — install {names} and make sure it is on your PATH, \
         then run this block again"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_program_that_exists_and_misses_one_that_does_not() {
        assert!(have("sh") || have("cmd"));
        assert!(!have("dx-definitely-not-a-real-program"));
    }

    #[test]
    fn locate_returns_a_usable_absolute_path() {
        let shell = locate("sh").or_else(|| locate("cmd"));
        let path = shell.expect("a system shell exists");
        assert!(path.is_absolute());
        assert!(path.is_file());
    }

    #[test]
    fn first_available_prefers_earlier_candidates() {
        let found = first_available(&["dx-not-real", "sh", "cmd"]);
        assert!(found.is_some());
        assert!(first_available(&["dx-not-real", "dx-also-not-real"]).is_none());
    }

    #[test]
    fn missing_toolchain_message_names_the_tools() {
        let message = missing_toolchain_message("python", &["uv", "python3"]);
        assert!(message.contains("uv or python3"));
        assert!(message.contains("PATH"));
    }
}
