//! Where this user's files live, on whichever operating system is running.
//!
//! Three directories are needed by more than one part of the installer — the home directory
//! itself, the per-user data directory a browser reads an unpacked extension from, and the
//! log directory the launch agent writes to. Each platform spells all three differently, and
//! each was previously spelled out again wherever it was wanted. One copy here means a
//! machine with an unusual `HOME` is wrong in one place rather than four.

use std::path::PathBuf;

/// The current user's home directory.
///
/// `None` on a machine with neither `HOME` nor `USERPROFILE`, which is a service account or
/// a broken environment — callers decide whether that is fatal, because for some of them
/// (writing a launch agent) it is and for others (reporting status) it is not.
#[must_use]
pub fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
}

/// The home directory, or `.` when there is none.
///
/// For paths that are only ever *reported* or compared. Anything that installs should use
/// [`home`] and fail with a sentence instead of quietly writing into the working directory.
#[must_use]
pub fn home_or_here() -> PathBuf {
    home().unwrap_or_else(|| PathBuf::from("."))
}

/// This operating system's per-user data directory.
#[must_use]
pub fn data_dir() -> PathBuf {
    let home = home_or_here();
    if cfg!(target_os = "macos") {
        return home.join("Library").join("Application Support");
    }
    if cfg!(windows) {
        return local_app_data(&home);
    }
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| home.join(".local").join("share"))
}

/// Where `dx` writes what it wants to survive a restart but nobody edits by hand.
#[must_use]
pub fn logs_dir() -> PathBuf {
    let home = home_or_here();
    if cfg!(target_os = "macos") {
        return home.join("Library").join("Logs").join("dx");
    }
    if cfg!(windows) {
        return local_app_data(&home).join("dx").join("logs");
    }
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| home.join(".local").join("state"))
        .join("dx")
}

/// Windows' per-user application data directory.
#[must_use]
pub fn local_app_data(home: &std::path::Path) -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| home.join("AppData").join("Local"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn the_data_directory_is_under_the_home_directory() {
        let home = home_or_here();
        assert!(
            data_dir().starts_with(&home) || home == Path::new("."),
            "data dir {:?} is not under {:?}",
            data_dir(),
            home
        );
    }

    #[test]
    fn logs_are_kept_somewhere_durable_rather_than_in_a_temporary_directory() {
        // A launch agent appends here across restarts; a path that is cleaned up would lose
        // exactly the record someone goes looking for after a failure.
        assert!(!logs_dir().starts_with(std::env::temp_dir()));
        assert!(logs_dir().ends_with("dx"));
    }

    #[test]
    fn a_home_directory_is_found_on_any_machine_a_test_runs_on() {
        assert!(home().is_some(), "no HOME and no USERPROFILE");
    }
}
