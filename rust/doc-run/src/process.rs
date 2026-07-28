//! Running one child process: capture its output, and never hang.
//!
//! Two guarantees matter here. First, a runaway block cannot wedge the document — every
//! process gets a deadline and is killed when it passes. Second, what the reader sees is
//! what the process printed: stdout and stderr are both captured, with stderr appended
//! under a marker so an error message is never silently dropped.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// How long to wait between checks on a running child.
const POLL_INTERVAL: Duration = Duration::from_millis(25);

/// Exit code reported when a process is killed for exceeding its deadline.
pub const TIMEOUT_EXIT: i32 = 124;

/// Exit code reported when the program could not be started at all.
pub const LAUNCH_FAILED_EXIT: i32 = 127;

/// What to run: a program, its arguments, and the environment it needs.
#[derive(Debug, Clone)]
pub struct CommandSpec {
    /// Program name or absolute path.
    pub program: String,
    /// Arguments passed to the program.
    pub args: Vec<String>,
    /// Extra environment variables layered over the inherited environment.
    pub env: Vec<(String, String)>,
}

impl CommandSpec {
    /// Build a spec for `program` with `args` and no extra environment.
    pub fn new(program: impl Into<String>, args: &[&str]) -> Self {
        Self {
            program: program.into(),
            args: args.iter().map(|arg| (*arg).to_string()).collect(),
            env: Vec::new(),
        }
    }

    /// Add one environment variable, returning the modified spec.
    #[must_use]
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    /// A shell-ish rendering of the command, for error messages and logs.
    #[must_use]
    pub fn display(&self) -> String {
        let mut parts = vec![self.program.clone()];
        parts.extend(self.args.iter().cloned());
        parts.join(" ")
    }
}

/// What a finished process produced.
#[derive(Debug, Clone)]
pub struct Capture {
    /// Combined stdout and stderr, in that order.
    pub output: String,
    /// Process exit code; see [`TIMEOUT_EXIT`] and [`LAUNCH_FAILED_EXIT`].
    pub exit: i32,
    /// Whether the process was killed for exceeding its deadline.
    pub timed_out: bool,
}

impl Capture {
    /// Whether the process finished successfully.
    #[must_use]
    pub fn succeeded(&self) -> bool {
        self.exit == 0 && !self.timed_out
    }

    /// A failure capture describing a process that never started.
    fn launch_failed(spec: &CommandSpec, error: &std::io::Error) -> Self {
        Self {
            output: format!(
                "could not start `{}`: {error}\n\nInstall it, or remove `run` from this block.",
                spec.program
            ),
            exit: LAUNCH_FAILED_EXIT,
            timed_out: false,
        }
    }
}

/// Run `spec` in `working_dir`, killing it after `timeout` and returning what it produced.
///
/// Never returns an error: a process that cannot start, fails, or overruns its deadline is
/// reported as a [`Capture`] the document can display, because all three are results the
/// reader needs to see rather than exceptions the caller can act on.
pub fn run(spec: &CommandSpec, working_dir: &Path, timeout: Duration) -> Capture {
    let mut command = Command::new(&spec.program);
    command
        .args(&spec.args)
        .current_dir(working_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in &spec.env {
        command.env(key, value);
    }

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => return Capture::launch_failed(spec, &error),
    };

    let stdout = child.stdout.take().map(drain_stream);
    let stderr = child.stderr.take().map(drain_stream);
    let deadline = Instant::now() + timeout;
    let mut timed_out = false;

    let exit = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code().unwrap_or(1),
            Ok(None) => {}
            Err(_) => break 1,
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            timed_out = true;
            break TIMEOUT_EXIT;
        }
        thread::sleep(POLL_INTERVAL);
    };

    let out_text = stdout.map(collect_stream).unwrap_or_default();
    let err_text = stderr.map(collect_stream).unwrap_or_default();

    Capture {
        output: merge_streams(&out_text, &err_text, timed_out, timeout),
        exit,
        timed_out,
    }
}

/// Read a child pipe on its own thread so a full pipe buffer can never deadlock the wait
/// loop, returning the channel the bytes will arrive on.
fn drain_stream<R: Read + Send + 'static>(mut stream: R) -> mpsc::Receiver<String> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = stream.read_to_end(&mut buffer);
        let _ = sender.send(String::from_utf8_lossy(&buffer).into_owned());
    });
    receiver
}

/// Collect whatever a drained stream produced, tolerating a thread that never reported.
fn collect_stream(receiver: mpsc::Receiver<String>) -> String {
    receiver.recv().unwrap_or_default()
}

/// Join stdout, stderr, and any timeout notice into the text the document will show.
fn merge_streams(stdout: &str, stderr: &str, timed_out: bool, timeout: Duration) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !stdout.trim().is_empty() {
        parts.push(stdout.trim_end().to_string());
    }
    if !stderr.trim().is_empty() {
        parts.push(format!("--- stderr ---\n{}", stderr.trim_end()));
    }
    if timed_out {
        parts.push(format!(
            "--- timed out after {}s ---",
            timeout.as_secs().max(1)
        ));
    }
    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env::temp_dir;

    #[test]
    fn captures_stdout_from_a_successful_command() {
        let spec = CommandSpec::new("echo", &["hello"]);
        let capture = run(&spec, &temp_dir(), Duration::from_secs(10));
        assert!(capture.succeeded());
        assert_eq!(capture.output, "hello");
    }

    #[test]
    fn reports_stderr_and_the_exit_code_on_failure() {
        let spec = CommandSpec::new("sh", &["-c", "echo oops 1>&2; exit 3"]);
        let capture = run(&spec, &temp_dir(), Duration::from_secs(10));
        assert_eq!(capture.exit, 3);
        assert!(capture.output.contains("--- stderr ---"));
        assert!(capture.output.contains("oops"));
    }

    #[test]
    fn kills_a_process_that_overruns_its_deadline() {
        let spec = CommandSpec::new("sh", &["-c", "sleep 30"]);
        let capture = run(&spec, &temp_dir(), Duration::from_millis(300));
        assert!(capture.timed_out);
        assert_eq!(capture.exit, TIMEOUT_EXIT);
        assert!(capture.output.contains("timed out"));
    }

    #[test]
    fn a_missing_program_is_a_result_not_a_crash() {
        let spec = CommandSpec::new("dx-no-such-program-anywhere", &[]);
        let capture = run(&spec, &temp_dir(), Duration::from_secs(5));
        assert_eq!(capture.exit, LAUNCH_FAILED_EXIT);
        assert!(capture.output.contains("could not start"));
    }

    #[test]
    fn environment_overrides_reach_the_child() {
        let spec = CommandSpec::new("sh", &["-c", "echo $DX_TEST_VALUE"])
            .with_env("DX_TEST_VALUE", "visible");
        let capture = run(&spec, &temp_dir(), Duration::from_secs(10));
        assert_eq!(capture.output, "visible");
    }
}
