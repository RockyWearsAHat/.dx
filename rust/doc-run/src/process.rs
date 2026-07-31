//! Running one child process: capture its output, and never hang.
//!
//! Three guarantees matter here.
//!
//! 1. **A runaway block cannot wedge the document.** Every process gets a deadline, and when
//!    it passes the whole tree it started is killed — not just the process dx launched, which
//!    on a `sh -c "thing &"` is the one process that has already exited.
//! 2. **What the reader sees is what the process printed.** stdout and stderr are both
//!    captured, with stderr under a marker, so an error is never silently dropped.
//! 3. **The child inherits an environment, not the reader's.** See [`child_environment`]: a
//!    shell full of API tokens is the most valuable thing on a developer's machine, and a
//!    block has no business being handed it.

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
        .stderr(Stdio::piped())
        .env_clear();
    for (key, value) in child_environment(spec) {
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
            // Descendants first, while this process is still alive to be their ancestor.
            // Once it is reaped they are re-parented and there is no trail left to follow —
            // and a `sh -c "sleep 999 &"` is a block whose direct child exits immediately.
            kill_tree(child.id());
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

/// Variables forwarded from dx's own environment, and nothing else.
///
/// Everything a block needs to *work* is here — where to find programs, how to format
/// numbers and dates. Everything a block could want to *steal* is not: `GITHUB_TOKEN`,
/// `AWS_SECRET_ACCESS_KEY`, `OPENAI_API_KEY`, and the rest of what accumulates in a
/// developer's shell never reach it.
///
/// This is an allow-list and stays one. A deny-list of "the secret-looking names" would be
/// wrong the first time someone exported a credential under a name nobody predicted.
const FORWARDED: &[&str] = &[
    "PATH",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "TZ",
    "TERM",
    // Windows cannot start a process without these.
    "SYSTEMROOT",
    "WINDIR",
    "COMSPEC",
    "NUMBER_OF_PROCESSORS",
    "PROCESSOR_ARCHITECTURE",
];

/// The environment a block's process actually gets: [`FORWARDED`], then `spec`'s own.
///
/// `spec`'s variables are applied last, so a caller that redirects `HOME` or `TMPDIR` into
/// the block's directory always wins over whatever was inherited.
fn child_environment(spec: &CommandSpec) -> Vec<(String, String)> {
    let mut environment: Vec<(String, String)> = FORWARDED
        .iter()
        .filter_map(|name| {
            std::env::var(name)
                .ok()
                .map(|value| ((*name).to_string(), value))
        })
        .collect();
    for (key, value) in &spec.env {
        environment.retain(|(existing, _)| existing != key);
        environment.push((key.clone(), value.clone()));
    }
    environment
}

/// Kill `pid`'s descendants — frozen first, then reaped — so nothing it started outlives it.
///
/// The freeze is what makes the kill airtight rather than a race: signals land one process
/// at a time, and a parent whose child dies is immediately runnable again — a
/// `(sleep 20; write) &` subshell used to slip its write into the gap between the kill that
/// took its `sleep` and the one aimed at it. A `SIGSTOP`ped process cannot run anything, so
/// the whole tree is stopped (twice — a process can fork between the table read and the
/// freeze), and only then killed. `doc-run/tests/attacks.rs` holds the marker-file proof.
///
/// Best-effort by design: it reads the process table through `ps`, which is the only way to
/// walk children without `unsafe`. A process that forks faster than the table can be read
/// can outrun it. What it does reliably catch is the ordinary case — a block that started a
/// server, or backgrounded a job, and then hit its deadline.
///
/// A process orphaned by a block that *exited* normally is not reachable this way at all,
/// because its parent link is gone. It is still inside the sandbox, so it can write nothing
/// and reach nothing; it is CPU, and the machine's own scheduler is what bounds it.
fn kill_tree(pid: u32) {
    if cfg!(windows) {
        let _ = Command::new("taskkill")
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        return;
    }

    let mut targets = descendants_of(pid);
    signal_all("STOP", &targets);
    // Anything forked between the read and the freeze shows up in a second look; a stopped
    // process cannot fork, so two passes converge.
    for late in descendants_of(pid) {
        if !targets.contains(&late) {
            targets.push(late);
        }
    }
    signal_all("STOP", &targets);
    signal_all("KILL", &targets);
}

/// Send one signal to every pid in `targets`, in a single `kill` invocation.
fn signal_all(signal: &str, targets: &[u32]) {
    if targets.is_empty() {
        return;
    }
    let mut command = Command::new("kill");
    command.arg(format!("-{signal}"));
    for pid in targets {
        command.arg(pid.to_string());
    }
    let _ = command.stdout(Stdio::null()).stderr(Stdio::null()).status();
}

/// Every process descended from `pid`, in breadth-first order.
fn descendants_of(pid: u32) -> Vec<u32> {
    let Ok(listing) = Command::new("ps").args(["-Ao", "pid=,ppid="]).output() else {
        return Vec::new();
    };
    let table: Vec<(u32, u32)> = String::from_utf8_lossy(&listing.stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            Some((fields.next()?.parse().ok()?, fields.next()?.parse().ok()?))
        })
        .collect();

    let mut found = Vec::new();
    let mut frontier = vec![pid];
    // Bounded by the table: every process is added at most once, so this cannot loop even
    // if the table reports a cycle.
    while let Some(parent) = frontier.pop() {
        for (child, _) in table.iter().filter(|(_, ppid)| *ppid == parent) {
            if *child != pid && !found.contains(child) {
                found.push(*child);
                frontier.push(*child);
            }
        }
    }
    found
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
