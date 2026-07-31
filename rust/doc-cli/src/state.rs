//! Whether something `dx` writes onto this machine still matches what this binary carries.
//!
//! Several things are written *out of* the binary and then read by something else: the
//! browser extension a browser loads on every start, the launch agent that keeps `dx serve`
//! running, the policy file that hands Firefox the extension. Each one can be absent, left
//! over from an older `dx`, or exactly current — and every one of them is repaired the same
//! way, by writing it again.
//!
//! So the question is asked once, here. `dx doctor` can then report every installed thing in
//! one voice, and no caller invents a fourth word for "out of date".

use std::path::{Path, PathBuf};

/// A file this binary installs, and the exact bytes it should hold.
pub type Installed = (PathBuf, Vec<u8>);

/// How what is on disk compares with what this binary would write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Nothing is installed.
    Missing,
    /// Something is installed, but not what this binary carries.
    Stale,
    /// Byte-for-byte what this binary carries.
    Current,
}

impl State {
    /// Compare `files` — each a path and the bytes it should hold — with what is on disk.
    ///
    /// [`Missing`](Self::Missing) means *nothing* is there, which is the only case a reader
    /// should read as "never installed". A partial install is [`Stale`](Self::Stale): some of
    /// it exists, so something did install it once, and the repair is the same either way.
    /// An empty list is [`Current`](Self::Current) — there is nothing to disagree about.
    #[must_use]
    pub fn of(files: &[Installed]) -> Self {
        let mut found = 0;
        let mut matched = 0;
        for (path, expected) in files {
            if let Ok(bytes) = std::fs::read(path) {
                found += 1;
                if &bytes == expected {
                    matched += 1;
                }
            }
        }
        if found == 0 && !files.is_empty() {
            return Self::Missing;
        }
        if matched == files.len() {
            Self::Current
        } else {
            Self::Stale
        }
    }

    /// One line for a report, naming the command that puts it right.
    ///
    /// The remedy is the caller's because the same three states are reached by different
    /// commands, and a reader who is told "out of date" without being told what to run has
    /// been given a problem rather than an answer.
    #[must_use]
    pub fn label(self, remedy: &str) -> String {
        match self {
            Self::Missing => format!("not installed — run `{remedy}`"),
            Self::Stale => format!("out of date — run `{remedy}`"),
            Self::Current => "current".to_string(),
        }
    }
}

/// Write every file in `files`, creating parent directories as needed.
///
/// Shared because every installer here does exactly this and each one had its own copy of
/// the create-parent-then-write dance, with its own wording when a path failed.
///
/// # Errors
/// When a directory or file cannot be written, naming the path that failed.
pub fn write_all(files: &[Installed]) -> Result<(), String> {
    for (path, bytes) in files {
        write_file(path, bytes)?;
    }
    Ok(())
}

/// Write one file, creating its parent directory.
///
/// # Errors
/// When the directory or the file cannot be written, naming the path that failed.
pub fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    std::fs::write(path, bytes)
        .map_err(|error| format!("could not write {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dx-state-tests-{label}"));
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    fn files(root: &Path) -> Vec<Installed> {
        vec![
            (root.join("one.txt"), b"one".to_vec()),
            (root.join("nested/two.txt"), b"two".to_vec()),
        ]
    }

    #[test]
    fn nothing_on_disk_is_missing_and_writing_makes_it_current() {
        let root = scratch("roundtrip");
        let files = files(&root);
        assert_eq!(State::of(&files), State::Missing);

        write_all(&files).expect("write");
        assert_eq!(State::of(&files), State::Current);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_file_changed_underneath_is_stale_and_writing_again_repairs_it() {
        let root = scratch("stale");
        let files = files(&root);
        write_all(&files).expect("write");

        std::fs::write(root.join("one.txt"), "edited").expect("edit");
        assert_eq!(State::of(&files), State::Stale);

        write_all(&files).expect("rewrite");
        assert_eq!(State::of(&files), State::Current);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A half-written install is repaired the same way a stale one is, so it must not be
    /// reported as never-installed — a reader told "not installed" about a directory that
    /// already has files in it goes looking for the wrong problem.
    #[test]
    fn a_partial_install_is_stale_rather_than_missing() {
        let root = scratch("partial");
        let files = files(&root);
        write_file(&files[0].0, &files[0].1).expect("write one");
        assert_eq!(State::of(&files), State::Stale);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn every_state_names_the_command_that_puts_it_right() {
        assert!(State::Missing.label("dx setup").contains("dx setup"));
        assert!(State::Stale.label("dx setup").contains("dx setup"));
        assert_eq!(State::Current.label("dx setup"), "current");
    }

    #[test]
    fn nothing_to_install_is_current_rather_than_missing() {
        assert_eq!(State::of(&[]), State::Current);
    }
}
