//! The per-block sandbox directory: where a block's files and installed libraries live.
//!
//! Each block gets a directory named after its fingerprint, so two blocks never share
//! state and an unchanged block reuses everything it built last time. Dependency
//! installation is guarded by a marker file, which is what turns "install matplotlib" from
//! a cost paid on every run into a cost paid once.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::plan::Plan;
use crate::process;

/// Marker written once a sandbox's setup commands have all succeeded.
const READY_MARKER: &str = ".dx-ready";

/// Extra time allowed for dependency installation, which is slower than running code.
const SETUP_TIMEOUT_FACTOR: u32 = 6;

/// Where sandboxes live when the caller does not choose.
///
/// Honors `DX_CACHE_DIR`, then the platform cache location, then a temp-directory
/// fallback so the engine still works on a machine with no home directory.
#[must_use]
pub fn default_cache_root() -> PathBuf {
    if let Some(explicit) = std::env::var_os("DX_CACHE_DIR") {
        return PathBuf::from(explicit);
    }
    if cfg!(windows) {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            return PathBuf::from(local).join("dx").join("run-cache");
        }
    }
    if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME") {
        return PathBuf::from(xdg).join("dx-run");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".cache").join("dx-run");
    }
    std::env::temp_dir().join("dx-run")
}

/// Materialize `plan`'s files in `directory` and run its setup commands once.
///
/// Returns a human-readable message on failure — the caller shows it to the reader in
/// place of program output, so it must explain what went wrong without a stack trace.
pub fn prepare(directory: &Path, plan: &Plan, timeout: Duration) -> Result<(), String> {
    fs::create_dir_all(directory)
        .map_err(|error| format!("could not create sandbox {}: {error}", directory.display()))?;

    for (name, contents) in &plan.files {
        write_file(directory, name, contents)?;
    }

    if plan.setup.is_empty() || directory.join(READY_MARKER).exists() {
        return Ok(());
    }

    let setup_timeout = timeout * SETUP_TIMEOUT_FACTOR;
    for step in &plan.setup {
        let capture = process::run(step, directory, setup_timeout);
        if !capture.succeeded() {
            return Err(format!(
                "dependency setup failed: `{}`\n\n{}",
                step.display(),
                capture.output
            ));
        }
    }

    fs::write(directory.join(READY_MARKER), b"ok")
        .map_err(|error| format!("could not record sandbox setup: {error}"))
}

/// Write one plan file, creating any parent directory it needs (`src/main.rs`).
fn write_file(directory: &Path, name: &str, contents: &str) -> Result<(), String> {
    let target = directory.join(name);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    fs::write(&target, contents)
        .map_err(|error| format!("could not write {}: {error}", target.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::CommandSpec;

    fn scratch(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("dx-sandbox-tests-{label}"));
        let _ = fs::remove_dir_all(&path);
        path
    }

    fn plan_with(setup: Vec<CommandSpec>, files: Vec<(String, String)>) -> Plan {
        Plan {
            files,
            setup,
            run: CommandSpec::new("true", &[]),
            run_in_sandbox: true,
        }
    }

    #[test]
    fn writes_nested_files_into_the_sandbox() {
        let directory = scratch("files");
        let plan = plan_with(
            Vec::new(),
            vec![("src/main.rs".to_string(), "fn main() {}".to_string())],
        );
        prepare(&directory, &plan, Duration::from_secs(5)).expect("prepare");
        assert_eq!(
            fs::read_to_string(directory.join("src/main.rs")).expect("read"),
            "fn main() {}"
        );
    }

    #[test]
    fn setup_runs_once_and_is_then_skipped() {
        let directory = scratch("setup-once");
        let counter = directory.join("count");
        let step = CommandSpec::new("sh", &["-c", "printf x >> count"]);
        let plan = plan_with(vec![step], Vec::new());

        prepare(&directory, &plan, Duration::from_secs(5)).expect("first prepare");
        prepare(&directory, &plan, Duration::from_secs(5)).expect("second prepare");

        assert_eq!(fs::read_to_string(&counter).expect("counter"), "x");
        assert!(directory.join(READY_MARKER).exists());
    }

    #[test]
    fn a_failing_setup_reports_the_command_and_is_not_marked_ready() {
        let directory = scratch("setup-fail");
        let step = CommandSpec::new("sh", &["-c", "echo nope 1>&2; exit 1"]);
        let error = prepare(
            &directory,
            &plan_with(vec![step], Vec::new()),
            Duration::from_secs(5),
        )
        .expect_err("setup should fail");
        assert!(error.contains("dependency setup failed"));
        assert!(error.contains("nope"));
        assert!(!directory.join(READY_MARKER).exists());
    }

    #[test]
    fn cache_root_honors_an_explicit_override() {
        std::env::set_var("DX_CACHE_DIR", "/tmp/dx-explicit-cache");
        assert_eq!(
            default_cache_root(),
            PathBuf::from("/tmp/dx-explicit-cache")
        );
        std::env::remove_var("DX_CACHE_DIR");
        assert!(default_cache_root().is_absolute());
    }
}
