//! Keeping `dx serve` running, so this device resolves documents without anyone starting it.
//!
//! # Why a login service and not "start it when something needs it"
//! Every surface that shows a document is a thin client of the rendering service: the browser
//! extension, an editor, a file preview, a phone on the same machine. None of them can start
//! a process — a browser extension in particular is sandboxed precisely so that it cannot —
//! and all of them are used at moments when nobody is at a terminal. A service that has to be
//! started by hand is therefore a service that is not running at the moment it is wanted,
//! which is the moment a reader opens a pull request and sees a pointer.
//!
//! So the daemon is registered with whatever this operating system uses to start things at
//! login: `launchd` on macOS, a systemd user unit on Linux, a Startup entry on Windows. All
//! three are *per-user* registrations that need no administrator, which is deliberate — an
//! installer that asks for a password to run a rendering service is asking for more than it
//! needs.
//!
//! # What it costs when it is not running
//! Nothing that matters. The extension prefers the daemon and falls back to its bundled
//! engine, so a reader whose service is stopped still sees documents; they are just decoded
//! once per page instead of once per machine. That is what makes it safe to install this
//! without ceremony, and safe for [`uninstall`] to be a complete undo.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use crate::daemon;
use crate::home;
use crate::state::{self, State};

/// What the operating system files this service under.
///
/// A reverse-DNS label because macOS and Linux both expect one, and one spelling everywhere
/// means a person hunting for it finds the same string in `launchctl list`, `systemctl
/// --user status`, and the Startup folder.
pub const LABEL: &str = "tools.dx.serve";

/// How long to wait for the daemon to answer a probe.
///
/// Short on purpose: this runs inside `dx doctor` and inside the installer, and every port
/// in [`daemon::PORTS`] is tried in turn, so a generous timeout on a machine where nothing
/// is listening becomes a visible stall for no information.
const PROBE: Duration = Duration::from_millis(300);

/// The most of a health response worth reading before deciding.
const PROBE_LIMIT: usize = 4096;

/// What a machine with no home directory is told, once.
const NO_HOME: &str =
    "dx could not find your home directory (neither HOME nor USERPROFILE is set),\n\
     so it cannot register a login service. Run `dx serve` yourself, or set HOME.";

/// Where this platform's service definition lives, and how the platform is told about it.
///
/// Separated from the definition's *contents* so that removing the service needs to know
/// only this — an uninstall must work even when the binary the service names is already
/// gone.
struct Plan {
    /// The file the operating system reads the service definition from.
    file: PathBuf,
    /// Commands that start it, run after the file is written. A failure here is reported.
    activate: Vec<Vec<String>>,
    /// Commands that make the platform forget it, run before the file is removed. Failures
    /// are ignored: the usual reason one fails is that the service was not loaded, which is
    /// the state the caller wanted anyway.
    deactivate: Vec<Vec<String>>,
}

/// Register the service and start it now, returning the file that defines it.
///
/// Installing is idempotent: the definition is rewritten, the platform is told to forget the
/// old one, and then to take the new one. That ordering is what makes an upgrade work —
/// `launchd` and `systemd` both keep running the *old* command until the old registration is
/// dropped, so writing the file alone would leave a previous `dx` serving until the next
/// reboot.
///
/// # Errors
/// When there is no home directory, when the definition cannot be written, or when the
/// platform refuses to start it — each naming what failed.
pub fn install(binary: &Path) -> Result<PathBuf, String> {
    let plan = plan()?;
    state::write_file(&plan.file, contents(binary)?.as_bytes())?;

    for command in &plan.deactivate {
        let _ = run(command);
    }
    for command in &plan.activate {
        run(command)?;
    }
    Ok(plan.file)
}

/// Stop the service and remove its definition, reporting whether anything was there.
///
/// # Errors
/// When there is no home directory, or the definition exists and cannot be removed.
pub fn uninstall() -> Result<bool, String> {
    let plan = plan()?;
    for command in &plan.deactivate {
        let _ = run(command);
    }
    if !plan.file.exists() {
        return Ok(false);
    }
    std::fs::remove_file(&plan.file)
        .map_err(|error| format!("could not remove {}: {error}", plan.file.display()))?;
    Ok(true)
}

/// Whether the installed definition is the one this binary would write for `binary`.
///
/// # Errors
/// When there is no home directory to look in.
pub fn state(binary: &Path) -> Result<State, String> {
    let plan = plan()?;
    Ok(State::of(&[(plan.file, contents(binary)?.into_bytes())]))
}

/// The port a running `dx serve` is answering on, if one is.
///
/// This asks the daemon rather than the service manager on purpose: what a reader cares
/// about is whether a document will resolve, and that is true exactly when something answers
/// on a port clients probe — whether this service started it, a terminal did, or a developer
/// build is running instead.
#[must_use]
pub fn running_port() -> Option<u16> {
    daemon::PORTS.iter().copied().find(|port| answers(*port))
}

/// The port the service is answering on, waiting up to `patience` for it to come up.
///
/// A service manager starts a process asynchronously: `launchctl bootstrap` returns once the
/// job is *loaded*, not once it has bound a port, and the gap is a few hundred milliseconds.
/// A single immediate probe therefore reports "not running" for a service that is about to
/// answer — at exactly the moment somebody is deciding whether the install worked. Measured
/// here: the probe said not running, and the daemon was serving on 7333 by the time the next
/// command ran.
#[must_use]
pub fn wait_until_serving(patience: Duration) -> Option<u16> {
    let deadline = Instant::now() + patience;
    loop {
        if let Some(port) = running_port() {
            return Some(port);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(RETRY);
    }
}

/// How long to wait for a freshly started service before reporting it as not running.
pub const STARTUP: Duration = Duration::from_secs(3);

/// How long to pause between probes while waiting for the service to come up.
const RETRY: Duration = Duration::from_millis(100);

/// Whether a dx daemon answers on `port`.
fn answers(port: u16) -> bool {
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let Ok(mut stream) = TcpStream::connect_timeout(&address, PROBE) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(PROBE));
    let _ = stream.set_write_timeout(Some(PROBE));
    // `Connection: close` so the read below ends at end-of-stream rather than at the timeout.
    let request = b"GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
    if stream.write_all(request).is_err() {
        return false;
    }

    // Bounded: whatever is on that port was not necessarily written by us, and a peer that
    // sends without end is a hang here rather than anywhere useful.
    let mut answer = Vec::new();
    let mut chunk = [0_u8; 512];
    while answer.len() < PROBE_LIMIT {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(read) => answer.extend_from_slice(&chunk[..read]),
        }
    }
    String::from_utf8_lossy(&answer).contains("\"name\":\"dx\"")
}

/// How `dx doctor` reports the service: what is registered, and what is actually answering.
///
/// Both, because they answer different questions and either can be true without the other —
/// a service can be registered and crashing, or unregistered and running from a terminal.
#[must_use]
pub fn status_lines(binary: &Path) -> Vec<String> {
    let registered = match state(binary) {
        Ok(state) => state.label("dx setup"),
        Err(error) => error,
    };
    let running = running_port().map_or_else(
        || "not running — run `dx setup` to start it at login".to_string(),
        |port| format!("http://127.0.0.1:{port}"),
    );
    vec![
        format!("  {:<9} {registered}", "at login"),
        format!("  {:<9} {running}", "serving"),
    ]
}

/// Run one command to completion, reporting a failure with what it printed.
fn run(command: &[String]) -> Result<(), String> {
    let Some((program, arguments)) = command.split_first() else {
        return Ok(());
    };
    let output = Command::new(program)
        .args(arguments)
        .output()
        .map_err(|error| format!("could not run `{program}`: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let complaint = String::from_utf8_lossy(&output.stderr);
    let complaint = complaint.trim();
    Err(format!(
        "`{}` failed{}",
        command.join(" "),
        if complaint.is_empty() {
            String::new()
        } else {
            format!(": {complaint}")
        }
    ))
}

/// A command line, as owned words.
fn words(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| (*part).to_string()).collect()
}

// ---------------------------------------------------------------------------------------
// macOS — a launchd user agent
// ---------------------------------------------------------------------------------------

/// The launch agent's file, and the `launchctl` calls that load and unload it.
#[cfg(target_os = "macos")]
fn plan() -> Result<Plan, String> {
    let home = home::home().ok_or_else(|| NO_HOME.to_string())?;
    let file = home
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{LABEL}.plist"));
    // `bootstrap`/`bootout` against the GUI domain rather than the deprecated `load`/`unload`:
    // the modern pair reports why it refused, which is the difference between a fixable
    // message and a service that silently never starts.
    let domain = format!("gui/{}", user_id(&home)?);
    let target = format!("{domain}/{LABEL}");
    let path = file.to_string_lossy().into_owned();
    Ok(Plan {
        activate: vec![words(&["launchctl", "bootstrap", &domain, &path])],
        deactivate: vec![words(&["launchctl", "bootout", &target])],
        file,
    })
}

/// The numeric user id `launchd` files this agent's domain under.
#[cfg(target_os = "macos")]
fn user_id(home: &Path) -> Result<u32, String> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(home)
        .map(|data| data.uid())
        .map_err(|error| format!("could not read {}: {error}", home.display()))
}

/// The launch agent property list.
///
/// `KeepAlive` is conditional on an unsuccessful exit, so a crash is restarted but a
/// deliberate stop is not fought. `ProcessType` is `Background`, which tells macOS this may
/// be throttled in favour of whatever the person is actually looking at.
#[cfg(target_os = "macos")]
fn contents(binary: &Path) -> Result<String, String> {
    let logs = home::logs_dir();
    Ok(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n\
         <dict>\n\
         \x20 <key>Label</key>\n  <string>{label}</string>\n\
         \x20 <key>ProgramArguments</key>\n  <array>\n    <string>{binary}</string>\n\
         \x20   <string>serve</string>\n  </array>\n\
         \x20 <key>RunAtLoad</key>\n  <true/>\n\
         \x20 <key>KeepAlive</key>\n  <dict>\n    <key>SuccessfulExit</key>\n    <false/>\n  </dict>\n\
         \x20 <key>ProcessType</key>\n  <string>Background</string>\n\
         \x20 <key>StandardOutPath</key>\n  <string>{log}</string>\n\
         \x20 <key>StandardErrorPath</key>\n  <string>{log}</string>\n\
         </dict>\n\
         </plist>\n",
        label = LABEL,
        binary = xml(&binary.to_string_lossy()),
        log = xml(&logs.join("serve.log").to_string_lossy()),
    ))
}

/// Escape text for an XML text node.
///
/// A path can contain `&` and `<`, and a plist that does not parse is a launch agent macOS
/// refuses without saying why.
#[cfg(target_os = "macos")]
fn xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

// ---------------------------------------------------------------------------------------
// Linux — a systemd user unit
// ---------------------------------------------------------------------------------------

/// The unit file, and the `systemctl --user` calls that enable and disable it.
#[cfg(all(unix, not(target_os = "macos")))]
fn plan() -> Result<Plan, String> {
    let home = home::home().ok_or_else(|| NO_HOME.to_string())?;
    let unit = format!("{LABEL}.service");
    let file = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| home.join(".config"))
        .join("systemd")
        .join("user")
        .join(&unit);
    Ok(Plan {
        activate: vec![
            words(&["systemctl", "--user", "daemon-reload"]),
            words(&["systemctl", "--user", "enable", "--now", &unit]),
        ],
        deactivate: vec![words(&["systemctl", "--user", "disable", "--now", &unit])],
        file,
    })
}

/// The systemd user unit.
#[cfg(all(unix, not(target_os = "macos")))]
fn contents(binary: &Path) -> Result<String, String> {
    Ok(format!(
        "[Unit]\n\
         Description=dx — the local document rendering service\n\
         Documentation=https://github.com/RockyWearsAHat/dx\n\n\
         [Service]\n\
         ExecStart={} serve\n\
         Restart=on-failure\n\
         RestartSec=5\n\n\
         [Install]\n\
         WantedBy=default.target\n",
        binary.display()
    ))
}

// ---------------------------------------------------------------------------------------
// Windows — a Startup entry
// ---------------------------------------------------------------------------------------

/// The Startup script, and the call that runs it without waiting for it.
///
/// A Startup entry rather than a scheduled task or a service: both of those want an
/// administrator, and this needs to be a per-user install like the other two platforms.
#[cfg(windows)]
fn plan() -> Result<Plan, String> {
    let home = home::home().ok_or_else(|| NO_HOME.to_string())?;
    let file = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| home.join("AppData").join("Roaming"))
        .join("Microsoft")
        .join("Windows")
        .join("Start Menu")
        .join("Programs")
        .join("Startup")
        .join("dx-serve.cmd");
    // `start` returns as soon as it has launched the script, which is what keeps installing
    // from blocking forever on a server that by design never exits.
    let path = file.to_string_lossy().into_owned();
    Ok(Plan {
        activate: vec![words(&["cmd", "/C", "start", "", "/b", &path])],
        deactivate: Vec::new(),
        file,
    })
}

/// The Startup script.
#[cfg(windows)]
fn contents(binary: &Path) -> Result<String, String> {
    Ok(format!(
        "@echo off\r\nrem {LABEL} — the local document rendering service\r\nstart \"\" /b \"{}\" serve\r\n",
        binary.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tests here deliberately do **not** install: registering a launch agent from a unit
    /// test would start a daemon on the machine running the suite and leave it there. What is
    /// checked is everything that can be wrong without doing that — the definition's content,
    /// its location, and the probe.
    #[test]
    fn the_definition_starts_the_binary_it_was_given_with_the_serve_command() {
        let text = contents(Path::new("/opt/dx/bin/dx")).expect("contents");
        assert!(text.contains("/opt/dx/bin/dx"), "{text}");
        assert!(text.contains("serve"), "{text}");
    }

    #[test]
    fn the_definition_lives_where_this_platform_looks_for_one() {
        let file = plan().expect("plan").file;
        assert!(file.starts_with(home::home_or_here()), "{file:?}");
        assert!(
            !file.starts_with(std::env::temp_dir()),
            "a login service read on every boot cannot live in a temporary directory"
        );
    }

    #[test]
    fn a_definition_naming_a_different_binary_is_stale_rather_than_current() {
        // What this pins: upgrading `dx` to a new location has to be visible, or `dx doctor`
        // reports a healthy service that is starting a binary which no longer exists.
        let file = plan().expect("plan").file;
        let mine = contents(Path::new("/one/dx")).expect("contents");
        let theirs = contents(Path::new("/another/dx")).expect("contents");
        assert_ne!(
            mine, theirs,
            "the definition must name the binary, or an upgrade is invisible"
        );
        assert_eq!(
            State::of(&[(file.clone(), mine.into_bytes())]),
            State::of(&[(file, theirs.into_bytes())]),
            "neither is installed here, so both report the same state"
        );
    }

    #[test]
    fn probing_a_port_nothing_is_listening_on_is_false_rather_than_a_hang() {
        // Port 1 is reserved and never bound; the point is that this returns at all.
        assert!(!answers(1));
    }

    /// Waiting has to end. The service manager may never start anything — a unit file it
    /// refused, a binary that is gone — and an installer that blocks forever on that is worse
    /// than one that reports the truth.
    #[test]
    fn waiting_for_a_service_that_never_comes_up_gives_up_rather_than_blocking() {
        let started = Instant::now();
        // Only meaningful when nothing is already serving; on a machine running `dx serve`
        // this returns at once, which is the other half of the contract.
        let found = wait_until_serving(Duration::from_millis(300));
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "waiting did not respect its deadline"
        );
        assert_eq!(found.is_some(), running_port().is_some());
    }

    #[test]
    fn the_probe_only_believes_a_daemon_that_names_itself() {
        // `running_port` must not report a port some other program happens to hold, because
        // the installer uses it to decide whether the service came up.
        let listener = std::net::TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .expect("bind an ephemeral port");
        let port = listener.local_addr().expect("address").port();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\n\r\nnot dx");
            }
        });
        assert!(!answers(port));
    }

    #[test]
    fn a_failing_command_is_reported_with_the_command_that_failed() {
        let error = run(&words(&["false"])).expect_err("`false` fails by definition");
        assert!(error.contains("false"), "{error}");
    }

    #[test]
    fn an_empty_command_is_nothing_to_do_rather_than_a_panic() {
        assert!(run(&[]).is_ok());
    }
}
