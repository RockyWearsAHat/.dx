//! The per-block working directory: where a block's files and installed libraries live.
//!
//! Each block gets a directory named after its fingerprint, so two blocks never share state
//! and an unchanged block reuses everything it built last time. Dependency installation is
//! guarded by a marker file, which is what turns "install matplotlib" from a cost paid on
//! every run into a cost paid once.
//!
//! This directory is also the **only** place a block may write. That is imposed by
//! [`crate::confine`], not by anything here — this module decides *where*, the kernel decides
//! *whether*. Setup runs inside the same boundary as the code, with one difference: it may
//! reach the network, because installing a library requires it.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::confine::{confine, Grant};
use crate::plan::Plan;
use crate::process;

/// Marker written once a directory's setup commands have all succeeded.
const READY_MARKER: &str = ".dx-ready";

/// Extra time allowed for dependency installation, which is slower than running code.
const SETUP_TIMEOUT_FACTOR: u32 = 6;

/// Where block directories and toolchain caches live when the caller does not choose.
///
/// Honors `DX_CACHE_DIR`, then the platform cache location, then a temp-directory fallback
/// so the engine still works on a machine with no home directory.
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
/// `grant` is the setup phase's own — the block directory and the toolchain caches writable,
/// and the network available, which is the one thing setup has that the run does not.
///
/// # Errors
/// Returns a human-readable message the caller shows to the reader in place of program
/// output, so it must explain what went wrong without a stack trace.
pub fn prepare(
    directory: &Path,
    plan: &Plan,
    grant: &Grant,
    timeout: Duration,
) -> Result<(), String> {
    fs::create_dir_all(directory)
        .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
    for path in &grant.writable {
        fs::create_dir_all(path)
            .map_err(|error| format!("could not create {}: {error}", path.display()))?;
    }

    for (name, contents) in &plan.files {
        write_file(directory, name, contents)?;
    }

    if plan.setup.is_empty() || directory.join(READY_MARKER).exists() {
        return Ok(());
    }

    let setup_timeout = timeout * SETUP_TIMEOUT_FACTOR;
    for step in &plan.setup {
        // Installing a library runs the library's own install scripts. Setup is inside the
        // boundary for exactly that reason — the network is all it gets extra.
        let confined = confine(step, grant)?;
        let capture = process::run(&confined, directory, setup_timeout);
        if !capture.succeeded() {
            return Err(format!(
                "dependency setup failed: `{}`\n\n{}",
                step.display(),
                capture.output
            ));
        }
    }

    fs::write(directory.join(READY_MARKER), b"ok")
        .map_err(|error| format!("could not record setup: {error}"))
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
        let path = std::env::temp_dir().join(format!("dx-workdir-tests-{label}"));
        let _ = fs::remove_dir_all(&path);
        path
    }

    fn plan_with(setup: Vec<CommandSpec>, files: Vec<(String, String)>) -> Plan {
        Plan {
            files,
            setup,
            run: CommandSpec::new("true", &[]),
        }
    }

    fn grant(directory: &Path) -> Grant {
        Grant::offline(vec![directory.to_path_buf()]).with_network()
    }

    #[test]
    fn writes_nested_files_into_the_block_directory() {
        let directory = scratch("files");
        let plan = plan_with(
            Vec::new(),
            vec![("src/main.rs".to_string(), "fn main() {}".to_string())],
        );
        prepare(
            &directory,
            &plan,
            &grant(&directory),
            Duration::from_secs(5),
        )
        .expect("prepare");
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
        let grant = grant(&directory);

        prepare(&directory, &plan, &grant, Duration::from_secs(20)).expect("first prepare");
        prepare(&directory, &plan, &grant, Duration::from_secs(20)).expect("second prepare");

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
            &grant(&directory),
            Duration::from_secs(20),
        )
        .expect_err("setup should fail");
        assert!(error.contains("dependency setup failed"));
        assert!(error.contains("nope"));
        assert!(!directory.join(READY_MARKER).exists());
    }

    /// Setup is confined too. An `npm install` runs the package's own install scripts, and
    /// those are as much a stranger's code as the block is.
    #[test]
    fn setup_cannot_write_outside_the_directories_it_was_granted() {
        if crate::confine::overridden() {
            return;
        }
        let directory = scratch("setup-confined");
        let outside = std::env::temp_dir().join("dx-workdir-tests-escape.txt");
        let _ = fs::remove_file(&outside);
        let step = CommandSpec::new(
            "sh",
            &["-c", &format!("echo pwned > {}", outside.display())],
        );
        let result = prepare(
            &directory,
            &plan_with(vec![step], Vec::new()),
            &Grant::offline(vec![directory.clone()]).with_network(),
            Duration::from_secs(20),
        );
        assert!(result.is_err(), "the write outside should have failed");
        assert!(!outside.exists(), "setup wrote outside its directory");
    }

    #[test]
    fn cache_root_honors_an_explicit_override() {
        let _env = crate::env_lock();
        std::env::set_var("DX_CACHE_DIR", "/tmp/dx-explicit-cache");
        assert_eq!(
            default_cache_root(),
            PathBuf::from("/tmp/dx-explicit-cache")
        );
        std::env::remove_var("DX_CACHE_DIR");
        assert!(default_cache_root().is_absolute());
    }
}
