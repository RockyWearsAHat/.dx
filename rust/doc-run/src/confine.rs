//! The boundary a block's code runs inside.
//!
//! A `.dx` document is something you are handed — from a repository, an agent, a colleague —
//! and marking a block `run` means its code executes on the machine that opened it. So the
//! code does not get the reader's authority. It gets this:
//!
//! | | |
//! |---|---|
//! | **Read** | anything, minus a short list of secret stores — a block is meant to open the data sitting beside the document |
//! | **Write** | its own block directory, plus any folder of the document's own that the block declares with `writes=` — the grant joins the fingerprint, so it is reviewed and approved with the code |
//! | **Network** | never, while the block's own code is running |
//!
//! Those three lines are the whole security model, and the first is why the other two have to
//! be absolute: a block that can read your disk and reach the network is a block that can
//! empty it. Read plus no-egress is a block that can *use* your files and do nothing with
//! them but show you the result.
//!
//! Only dependency installation gets the network, because `uv`, `npm`, `cargo`, and `gem`
//! cannot work without it — and installation is still confined in every other way, which
//! matters more than it sounds: an `npm install` runs the package's own install scripts.
//!
//! # How the boundary is imposed
//! The kernel imposes it, not this crate:
//!
//! - **macOS** — Seatbelt, through `/usr/bin/sandbox-exec`, with a deny-by-default profile.
//!   Every Mac has it, so there is no install step and no daemon.
//! - **Linux** — `bwrap` (bubblewrap): a read-only bind of the filesystem, a network
//!   namespace with nothing in it, and the block directory bound back in writable.
//!
//! The profile is inherited across `exec` and `fork`, so a block that shells out gets no more
//! than the block itself had.
//!
//! # When there is no boundary
//! [`confine`] returns an error and **the block does not run**. A machine that cannot confine
//! code is not a machine that runs a stranger's code anyway. `DX_UNCONFINED=1` overrides
//! that, and a run made under it says so in its own output — there is no silent way to end up
//! outside the boundary.

use std::path::{Path, PathBuf};

use crate::process::CommandSpec;

/// Environment variable that turns the boundary off, for a machine that has none.
const OVERRIDE: &str = "DX_UNCONFINED";

/// Text stamped into the output of any block that ran without a boundary.
pub const UNCONFINED_NOTICE: &str =
    "--- ran without a sandbox: DX_UNCONFINED is set, so this code had your own permissions ---";

/// Paths never readable by a block, relative to the reader's home directory.
///
/// Reads are otherwise open, which is what lets a block do the ordinary thing and open the
/// spreadsheet next to the document. These are the files where opening one is never ordinary:
/// the keys and tokens that would let whoever wrote the document be *you* somewhere else.
///
/// This list is a deny-list and is therefore the one part of this module that is best-effort
/// — a secret in a file nobody thought of is a secret a block can read, and print. The write
/// and network boundaries are the opposite shape (deny everything, allow a named few) and do
/// not share that weakness. What the deny-list buys is that a block cannot *quietly* pick up
/// a credential: with no network and no writes, the only place it can put one is the output,
/// on the page, where the reader is looking.
const SECRET_PATHS: &[&str] = &[
    ".ssh",
    ".aws",
    ".gnupg",
    ".netrc",
    ".docker/config.json",
    ".kube",
    ".config/gh",
    ".config/gcloud",
    ".npmrc",
    ".pypirc",
    "Library/Keychains",
    "Library/Application Support/Google/Chrome",
    "Library/Application Support/Firefox",
];

/// What one phase of a block's execution may do.
#[derive(Debug, Clone)]
pub struct Grant {
    /// Directories the process may write to. Everything else on disk is readable only.
    pub writable: Vec<PathBuf>,
    /// Whether the process may reach the network.
    ///
    /// True only while installing declared dependencies. The block's own code never gets it.
    pub network: bool,
}

impl Grant {
    /// A grant that may write to `writable` and reach nothing.
    #[must_use]
    pub fn offline(writable: Vec<PathBuf>) -> Self {
        Self {
            writable,
            network: false,
        }
    }

    /// The same grant, plus the network — for installing declared dependencies.
    #[must_use]
    pub fn with_network(mut self) -> Self {
        self.network = true;
        self
    }
}

/// Wrap `spec` so the kernel confines it to `grant`.
///
/// # Errors
/// Returns a sentence naming what to install when this machine has no boundary to impose, so
/// the caller can report a blocked block rather than running one unconfined.
pub fn confine(spec: &CommandSpec, grant: &Grant) -> Result<CommandSpec, String> {
    if overridden() {
        return Ok(spec.clone());
    }
    if cfg!(target_os = "macos") {
        return Ok(seatbelt(spec, grant));
    }
    if cfg!(target_os = "linux") {
        return bubblewrap(spec, grant);
    }
    Err(no_boundary_message())
}

/// Whether the reader has deliberately turned the boundary off.
#[must_use]
pub fn overridden() -> bool {
    std::env::var(OVERRIDE).is_ok_and(|value| value != "0" && !value.is_empty())
}

/// Name of the boundary this machine imposes, for `dx doctor` and error messages.
#[must_use]
pub fn describe() -> &'static str {
    if overridden() {
        return "none (DX_UNCONFINED is set)";
    }
    if cfg!(target_os = "macos") {
        "seatbelt"
    } else if cfg!(target_os = "linux") {
        "bubblewrap"
    } else {
        "none"
    }
}

/// What to say on a machine where no code can be safely run.
fn no_boundary_message() -> String {
    format!(
        "this platform has no sandbox dx can use, so no code was run.\n\n\
         Running a document runs code someone else wrote. On macOS dx confines it with \
         Seatbelt and on Linux with bubblewrap (`apt install bubblewrap`); here there is \
         neither.\n\n\
         Set {OVERRIDE}=1 to run it anyway, with your own permissions."
    )
}

/// Wrap `spec` in `sandbox-exec` carrying a deny-by-default Seatbelt profile.
///
/// The profile goes in as an argument rather than a file on purpose: a profile written to
/// disk would sit in the very directory the block can write to, and a block that runs long
/// enough to rewrite it before its next `exec` would be choosing its own boundary.
fn seatbelt(spec: &CommandSpec, grant: &Grant) -> CommandSpec {
    let mut args = vec![
        "-p".to_string(),
        seatbelt_profile(grant),
        spec.program.clone(),
    ];
    args.extend(spec.args.iter().cloned());
    CommandSpec {
        program: "/usr/bin/sandbox-exec".to_string(),
        args,
        env: spec.env.clone(),
    }
}

/// Build the Seatbelt profile for `grant`.
///
/// Written as one line because it is passed as a single argument. Order matters in SBPL —
/// the last rule matching an operation wins — so the broad `allow file-read*` is stated
/// before the narrow `deny` that carves the secret stores back out of it.
fn seatbelt_profile(grant: &Grant) -> String {
    let mut rules = vec![
        "(version 1)".to_string(),
        "(deny default)".to_string(),
        // A block runs an interpreter, which runs the block. Both are execs.
        "(allow process-exec)".to_string(),
        "(allow process-fork)".to_string(),
        // dyld, locale data, the interpreter's own standard library, and the document's
        // neighbours all have to be readable for any of this to work at all.
        "(allow file-read*)".to_string(),
        "(allow sysctl-read)".to_string(),
        // dyld and the system frameworks resolve services through the bootstrap port.
        "(allow mach-lookup)".to_string(),
        "(allow signal (target self))".to_string(),
        // Not files so much as holes: an interpreter writing to /dev/null is not a write.
        "(allow file-write-data (literal \"/dev/null\") (literal \"/dev/zero\") \
         (literal \"/dev/random\") (literal \"/dev/urandom\") (literal \"/dev/stdout\") \
         (literal \"/dev/stderr\") (literal \"/dev/dtracehelper\"))"
            .to_string(),
    ];

    if let Some(denied) = secret_subpaths() {
        rules.push(format!("(deny file-read* {denied})"));
    }

    let writable = grant
        .writable
        .iter()
        .map(|path| format!("(subpath {})", quote(&resolved(path))))
        .collect::<Vec<_>>()
        .join(" ");
    if !writable.is_empty() {
        rules.push(format!("(allow file-write* {writable})"));
    }

    rules.push(if grant.network {
        "(allow network*)".to_string()
    } else {
        // Every family, not just IP: a unix socket reaches a local daemon that has the
        // network this process was just denied.
        "(deny network*)".to_string()
    });

    rules.join(" ")
}

/// The `(subpath …)` clauses naming every secret store that exists on this machine.
///
/// Returns `None` when there is no home directory to resolve them against, which is the
/// case in a container and on a build machine — there are no user secrets there to name.
fn secret_subpaths() -> Option<String> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let clauses = SECRET_PATHS
        .iter()
        .map(|name| format!("(subpath {})", quote(&home.join(name).to_string_lossy())))
        .collect::<Vec<_>>()
        .join(" ");
    (!clauses.is_empty()).then_some(clauses)
}

/// Wrap `spec` in `bwrap`: a read-only filesystem with the writable paths bound back in.
fn bubblewrap(spec: &CommandSpec, grant: &Grant) -> Result<CommandSpec, String> {
    if crate::toolchain::first_available(&["bwrap"]).is_none() {
        return Err(format!(
            "no sandbox: dx confines a document's code with bubblewrap on Linux, and `bwrap` \
             is not installed, so no code was run.\n\n\
             Install it (`apt install bubblewrap`), or set {OVERRIDE}=1 to run this \
             document with your own permissions."
        ));
    }

    let mut args: Vec<String> = vec![
        // Everything readable, nothing writable, until a --bind says otherwise.
        "--ro-bind".into(),
        "/".into(),
        "/".into(),
        "--dev".into(),
        "/dev".into(),
        "--proc".into(),
        "/proc".into(),
        // A private /tmp, so one block cannot read what another left there.
        "--tmpfs".into(),
        "/tmp".into(),
        "--unshare-ipc".into(),
        "--unshare-uts".into(),
        "--unshare-pid".into(),
        // The whole tree goes when dx does, so a block cannot outlive the run it belongs to.
        "--die-with-parent".into(),
    ];

    if !grant.network {
        args.push("--unshare-net".into());
    }

    // An empty filesystem over each secret store: present, and holding nothing.
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        for name in SECRET_PATHS {
            let path = home.join(name);
            if path.exists() {
                args.push("--tmpfs".into());
                args.push(path.to_string_lossy().into_owned());
            }
        }
    }

    for path in &grant.writable {
        let resolved = resolved(path);
        args.push("--bind".into());
        args.push(resolved.clone());
        args.push(resolved);
    }

    args.push("--".into());
    args.push(spec.program.clone());
    args.extend(spec.args.iter().cloned());

    Ok(CommandSpec {
        program: "bwrap".to_string(),
        args,
        env: spec.env.clone(),
    })
}

/// A path as the kernel sees it, following the symlinks it will follow.
///
/// `/tmp` is a symlink to `/private/tmp` on macOS, and a rule naming the unresolved path
/// matches nothing — which would silently make a writable directory read-only.
fn resolved(path: &Path) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

/// Quote a path as an SBPL string literal.
fn quote(path: &str) -> String {
    let escaped = path.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grant() -> Grant {
        Grant::offline(vec![PathBuf::from("/tmp")])
    }

    #[test]
    fn the_profile_denies_everything_before_it_allows_anything() {
        let profile = seatbelt_profile(&grant());
        let deny_default = profile.find("(deny default)").expect("deny default");
        let first_allow = profile.find("(allow").expect("some allow");
        assert!(deny_default < first_allow, "{profile}");
    }

    #[test]
    fn an_offline_grant_denies_every_network_family() {
        let profile = seatbelt_profile(&grant());
        assert!(profile.contains("(deny network*)"), "{profile}");
        assert!(!profile.contains("(allow network*)"), "{profile}");
    }

    #[test]
    fn only_a_dependency_install_is_given_the_network() {
        let profile = seatbelt_profile(&grant().with_network());
        assert!(profile.contains("(allow network*)"), "{profile}");
    }

    #[test]
    fn a_writable_path_is_resolved_through_its_symlinks() {
        // /tmp is a symlink to /private/tmp on macOS; a rule naming the link matches nothing.
        let profile = seatbelt_profile(&grant());
        let named = if cfg!(target_os = "macos") {
            "/private/tmp"
        } else {
            "/tmp"
        };
        assert!(profile.contains(named), "{profile}");
    }

    #[test]
    fn reads_are_open_and_then_the_secret_stores_are_taken_back() {
        let _env = crate::env_lock();
        let original = std::env::var_os("HOME");
        std::env::set_var("HOME", "/Users/nobody");
        let profile = seatbelt_profile(&grant());
        match original {
            Some(home) => std::env::set_var("HOME", home),
            None => std::env::remove_var("HOME"),
        }
        let allow_read = profile.find("(allow file-read*)").expect("allow read");
        let deny_read = profile.find("(deny file-read*").expect("deny read");
        assert!(allow_read < deny_read, "the deny must win: {profile}");
        assert!(profile.contains("/Users/nobody/.ssh"), "{profile}");
        assert!(
            profile.contains("/Users/nobody/Library/Keychains"),
            "{profile}"
        );
    }

    #[test]
    fn a_quoted_path_cannot_end_the_string_it_is_in() {
        assert_eq!(quote(r#"/a"b"#), r#""/a\"b""#);
        assert_eq!(quote(r"/a\b"), r#""/a\\b""#);
    }

    #[test]
    fn confining_wraps_the_command_rather_than_replacing_it() {
        // Reads DX_UNCONFINED (via `confine`), so it must not overlap the override test.
        let _env = crate::env_lock();
        let spec = CommandSpec::new("python3", &["block.py"]);
        let confined = confine(&spec, &grant()).expect("a boundary on this platform");
        if cfg!(target_os = "macos") {
            assert_eq!(confined.program, "/usr/bin/sandbox-exec");
        }
        assert!(
            confined.display().contains("python3"),
            "{}",
            confined.display()
        );
        assert!(
            confined.display().contains("block.py"),
            "{}",
            confined.display()
        );
    }

    #[test]
    fn the_override_is_the_only_way_past_and_it_announces_itself() {
        let _env = crate::env_lock();
        std::env::set_var(OVERRIDE, "1");
        assert!(overridden());
        assert_eq!(describe(), "none (DX_UNCONFINED is set)");
        let spec = CommandSpec::new("python3", &[]);
        assert_eq!(
            confine(&spec, &grant()).expect("override").program,
            "python3"
        );
        std::env::remove_var(OVERRIDE);
        assert!(!overridden());
    }
}
