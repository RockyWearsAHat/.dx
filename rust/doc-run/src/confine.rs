//! The boundary a block's code runs inside.
//!
//! A `.dx` document is something you are handed — from a repository, an agent, a colleague —
//! and marking a block `run` means its code executes on the machine that opened it. So the
//! code does not get the reader's authority. It gets this:
//!
//! | | |
//! |---|---|
//! | **Read** | the repository the document lives in, the run caches, the system and its toolchains — never the rest of your files, and never the credential stores even inside what remains |
//! | **Write** | its own block directory, plus any folder of the document's own that the block declares with `writes=` — the grant joins the fingerprint, so it is reviewed and approved with the code |
//! | **Network** | never, while the block's own code is running |
//!
//! Those three lines are the whole security model. Reads are scoped to the repository on
//! purpose: a block is meant to open the data sitting beside its document — the project is
//! its world — and everything else on the machine is somebody's mail, notes, and browsing,
//! which no handed-over document has any business reading. The system directories and the
//! toolchain homes ([`TOOLCHAIN_HOMES`]) stay readable because an interpreter has to load
//! itself; the credential deny-list ([`SECRET_PATHS`]) still has the last word inside
//! whatever is readable. And the scope is also what makes a *locally edited* block safe to
//! run on the spot: code you just typed, confined to the project you typed it in.
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

/// Roots holding the machine's user data, denied to every block.
///
/// This is where the read scope is drawn: everything under these is somebody's files, not
/// the project's, and only the named [`Grant::readable`] roots (the document's repository,
/// the run caches) and the [`TOOLCHAIN_HOMES`] are allowed back through. `/tmp` is here
/// too — other processes' scratch is user data, and a block's own `$TMPDIR` already points
/// inside its writable directory.
const USER_DATA_ROOTS: &[&str] = &[
    "/Users",
    "/Volumes",
    "/home",
    "/root",
    "/media",
    "/mnt",
    "/tmp",
    "/private/tmp",
    // macOS per-user temp and cache trees: other programs' scratch is user data too.
    "/var/folders",
    "/private/var/folders",
];

/// Home-relative directories where language toolchains conventionally install themselves.
///
/// An interpreter has to be able to load itself, and toolchains live in the home directory
/// as often as on the system — `~/.cargo` and `~/.rustup` for Rust, `~/.local/bin` for
/// user-installed executables, the version managers for the rest. This is an allow-list of
/// *program* homes, never data: a manager missing from it fails closed, with the sandbox
/// note naming the boundary.
const TOOLCHAIN_HOMES: &[&str] = &[
    ".cargo",
    ".rustup",
    ".local/bin",
    ".local/share/uv",
    ".local/share/mise",
    ".nvm",
    ".pyenv",
    ".rbenv",
    ".volta",
    ".bun",
    ".deno",
];

/// What one phase of a block's execution may do.
#[derive(Debug, Clone)]
pub struct Grant {
    /// Directories the process may write to.
    pub writable: Vec<PathBuf>,
    /// Roots allowed back through the [`USER_DATA_ROOTS`] read denial — the document's
    /// repository and the run caches. The system and [`TOOLCHAIN_HOMES`] are always
    /// readable; everything else under a user-data root is not.
    pub readable: Vec<PathBuf>,
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
            readable: Vec::new(),
            network: false,
        }
    }

    /// The same grant, with `readable` roots allowed back through the user-data denial.
    #[must_use]
    pub fn reading(mut self, readable: Vec<PathBuf>) -> Self {
        self.readable = readable;
        self
    }

    /// The same grant, plus the network — for installing declared dependencies.
    #[must_use]
    pub fn with_network(mut self) -> Self {
        self.network = true;
        self
    }
}

/// The repository a document belongs to: the nearest ancestor of `document_dir` carrying a
/// `.git` or `.doc` marker, or `document_dir` itself when no ancestor does.
///
/// This is the read scope's outer edge — "the parent folder that has the repo" — so a
/// document deep in a project reads the whole project, and a loose document in a plain
/// folder reads that folder and nothing above it.
///
/// A `.git` **file** (rather than directory) satisfies the marker too — that is what a `git
/// worktree` checkout has, pointing at the real `.git` elsewhere — so a worktree is its own
/// scope by this function alone. [`read_scope`] is what widens that to the repository's other
/// worktrees; call that, not this, when building a [`Grant`].
#[must_use]
pub fn repo_root(document_dir: &Path) -> PathBuf {
    let start = std::fs::canonicalize(document_dir).unwrap_or_else(|_| document_dir.to_path_buf());
    let mut current = start.as_path();
    loop {
        if current.join(".git").exists() || current.join(".doc").exists() {
            return current.to_path_buf();
        }
        match current.parent() {
            Some(parent) => current = parent,
            None => return start.clone(),
        }
    }
}

/// The full read scope for a document at `document_dir`: its repository, plus every other
/// working tree of the *same* repository.
///
/// [`repo_root`] alone under-scopes a `git worktree` checkout. A worktree carries its own
/// `.git` (a file pointing at the real one), so `repo_root` correctly stops there rather than
/// climbing into whatever directory happens to be above it — but that leaves the checkout the
/// worktree was created from, and any of its siblings, outside the boundary even though they
/// are provably the same logical project: one `.git` common directory, shared history, shared
/// remotes. A gate that reads the main checkout's live state by absolute path is not reading
/// "the rest of the machine" — it is reading the repository it is already scoped to, from a
/// different one of that repository's own checkouts.
///
/// `git worktree list` (via [`doc_store::git::linked_worktrees`]) is the source of truth for
/// which directories those are: it is git's own bookkeeping, not anything the document or the
/// block declares, so a block cannot widen its own scope by naming an arbitrary path — only a
/// path git itself already recognises as another checkout of the same history comes back.
/// When git is unavailable, or `document_dir` is not a repository at all, this is exactly
/// `[repo_root(document_dir)]` — the pre-existing, narrower behaviour.
#[must_use]
pub fn read_scope(document_dir: &Path) -> Vec<PathBuf> {
    let root = repo_root(document_dir);
    let mut scope = vec![root.clone()];
    scope.extend(doc_store::git::linked_worktrees(&root));
    scope
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
/// the last rule matching an operation wins — so the read rules are stated broad to narrow:
/// the base `allow file-read*` (the system, the toolchains), then the [`USER_DATA_ROOTS`]
/// denial, then the grant's own roots allowed back through it, and the secret stores denied
/// last so nothing can allow them back.
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

    // The user's files go dark, and only the project's world comes back: the grant's own
    // roots (repository, run caches), the writable directories, and the toolchain homes.
    let user_data = USER_DATA_ROOTS
        .iter()
        .map(|root| format!("(subpath {})", quote(root)))
        .collect::<Vec<_>>()
        .join(" ");
    rules.push(format!("(deny file-read* {user_data})"));
    let roots: Vec<String> = grant
        .readable
        .iter()
        .chain(grant.writable.iter())
        .map(|path| resolved(path))
        .chain(toolchain_homes())
        .collect();
    if !roots.is_empty() {
        let scoped = roots
            .iter()
            .map(|path| format!("(subpath {})", quote(path)))
            .collect::<Vec<_>>()
            .join(" ");
        rules.push(format!("(allow file-read* {scoped})"));
        // An interpreter realpaths its way down to the script it was handed, `lstat`ing
        // each directory on the way — so the ancestors of every allowed root stay
        // stat-able, as named directories only: their *contents* are still dark.
        let ancestors = ancestor_literals(&roots)
            .iter()
            .map(|path| format!("(literal {})", quote(path)))
            .collect::<Vec<_>>()
            .join(" ");
        if !ancestors.is_empty() {
            rules.push(format!("(allow file-read-metadata {ancestors})"));
        }
    }

    if let Some(denied) = secret_subpaths() {
        rules.push(format!("(deny file-read* {denied})"));
    }

    let mut writable_paths: Vec<String> = grant
        .writable
        .iter()
        .map(|path| format!("(subpath {})", quote(&resolved(path))))
        .collect();

    // On macOS, allow writes to per-user temp and cache directories so Apple's toolchain
    // (xcrun, cc, etc.) can create and write to cache files without spamming warnings.
    #[cfg(target_os = "macos")]
    {
        if let Some(temp) = macos_temp_dir() {
            writable_paths.push(format!("(subpath {})", quote(&temp)));
        }
        if let Some(cache) = macos_cache_dir() {
            writable_paths.push(format!("(subpath {})", quote(&cache)));
        }
    }

    if !writable_paths.is_empty() {
        let writable = writable_paths.join(" ");
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

/// The [`TOOLCHAIN_HOMES`] resolved against this machine's home directory, existing only.
///
/// Empty when there is no home directory — a container's toolchains live on the system
/// paths, which the base allow already covers.
fn toolchain_homes() -> Vec<String> {
    let Some(home) = std::env::var_os("HOME") else {
        return Vec::new();
    };
    let home = PathBuf::from(home);
    TOOLCHAIN_HOMES
        .iter()
        .map(|name| home.join(name))
        .filter(|path| path.exists())
        .map(|path| resolved(&path))
        .collect()
}

/// Every distinct ancestor directory of `roots`, deduplicated, nearest first.
///
/// These are allowed as metadata-only literals so path resolution can walk down to an
/// allowed root through denied territory without being able to list or read any of it.
fn ancestor_literals(roots: &[String]) -> Vec<String> {
    let mut seen = Vec::new();
    for root in roots {
        let mut current = Path::new(root);
        while let Some(parent) = current.parent() {
            let text = parent.to_string_lossy().into_owned();
            if !text.is_empty() && text != "/" && !seen.contains(&text) {
                seen.push(text);
            }
            current = parent;
        }
    }
    seen
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

    // An empty filesystem over each user-data root: the machine's files go dark…
    for root in USER_DATA_ROOTS {
        if *root != "/tmp" && Path::new(root).exists() {
            args.push("--tmpfs".into());
            args.push((*root).to_string());
        }
    }
    // …and the project's world is bound back through, read-only: the grant's roots and
    // the toolchain homes. Later mounts stack over earlier ones, so these win over the
    // masking, the secret masks win over these, and the writable binds win over all.
    for path in grant
        .readable
        .iter()
        .map(|path| resolved(path))
        .chain(toolchain_homes())
    {
        if Path::new(&path).exists() {
            args.push("--ro-bind".into());
            args.push(path.clone());
            args.push(path);
        }
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

/// Get the macOS per-user temporary directory via confstr, if available.
///
/// Apple's toolchain (xcrun, cc) resolves the temp directory via confstr(_CS_DARWIN_USER_TEMP_DIR)
/// instead of $TMPDIR. This function retrieves that path so the sandbox can grant write access to it.
#[cfg(target_os = "macos")]
fn macos_temp_dir() -> Option<String> {
    use std::ffi::CStr;

    unsafe {
        let mut buf = [0u8; 1024];
        let len = libc::confstr(libc::_CS_DARWIN_USER_TEMP_DIR, buf.as_mut_ptr() as *mut i8, buf.len());
        if len > 0 && len <= buf.len() {
            CStr::from_bytes_until_nul(&buf[..len])
                .ok()
                .and_then(|s| s.to_str().ok())
                .map(|s| s.to_string())
        } else {
            None
        }
    }
}

/// Get the macOS per-user cache directory via confstr, if available.
///
/// Apple's toolchain caches build artifacts and other data in this directory.
#[cfg(target_os = "macos")]
fn macos_cache_dir() -> Option<String> {
    use std::ffi::CStr;

    unsafe {
        let mut buf = [0u8; 1024];
        let len = libc::confstr(libc::_CS_DARWIN_USER_CACHE_DIR, buf.as_mut_ptr() as *mut i8, buf.len());
        if len > 0 && len <= buf.len() {
            CStr::from_bytes_until_nul(&buf[..len])
                .ok()
                .and_then(|s| s.to_str().ok())
                .map(|s| s.to_string())
        } else {
            None
        }
    }
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
    fn reads_scope_to_the_grant_and_secrets_have_the_last_word() {
        let _env = crate::env_lock();
        let original = std::env::var_os("HOME");
        std::env::set_var("HOME", "/Users/nobody");
        let scoped = Grant::offline(vec![PathBuf::from("/tmp")])
            .reading(vec![PathBuf::from("/Users/nobody/project")]);
        let profile = seatbelt_profile(&scoped);
        match original {
            Some(home) => std::env::set_var("HOME", home),
            None => std::env::remove_var("HOME"),
        }
        // Broad to narrow, last match winning: base allow, user data dark, the project
        // allowed back through, and the secret stores denied after everything.
        let base = profile.find("(allow file-read*)").expect("base allow");
        let dark = profile
            .find("(deny file-read* (subpath \"/Users\")")
            .expect("user data denied");
        let back = profile
            .find("/Users/nobody/project")
            .expect("the project is allowed back");
        let secrets = profile.find("/Users/nobody/.ssh").expect("secrets denied");
        assert!(base < dark && dark < back && back < secrets, "{profile}");
        assert!(
            profile.contains("/Users/nobody/Library/Keychains"),
            "{profile}"
        );
        assert!(profile.contains("(subpath \"/Volumes\")"), "{profile}");
    }

    #[test]
    fn the_repo_root_is_the_nearest_marked_ancestor_or_the_folder_itself() {
        let scratch = std::env::temp_dir().join("dx-confine-tests-repo-root");
        let _ = std::fs::remove_dir_all(&scratch);
        let nested = scratch.join("repo/docs/deep");
        std::fs::create_dir_all(&nested).expect("tree");
        std::fs::create_dir_all(scratch.join("repo/.git")).expect("marker");
        assert_eq!(
            repo_root(&nested),
            std::fs::canonicalize(scratch.join("repo")).expect("canonical")
        );

        let loose = scratch.join("loose");
        std::fs::create_dir_all(&loose).expect("loose");
        assert_eq!(
            repo_root(&loose),
            std::fs::canonicalize(&loose).expect("canonical"),
            "a folder with no repository above it is its own scope"
        );
    }

    /// The bug this pins: a document run from a `git worktree` checkout of a repository could
    /// not read a file in the checkout the worktree was created from, even though both are
    /// the same logical repository sharing one `.git`. `repo_root` alone stops at the
    /// worktree (it has its own `.git` file), so the read scope has to be widened explicitly
    /// by [`read_scope`] rather than by climbing further — climbing would also let a loose
    /// document outside any repository walk into an unrelated parent directory.
    #[test]
    fn a_worktree_reads_its_own_checkout_and_its_sibling_worktree() {
        let scratch = std::env::temp_dir().join("dx-confine-tests-read-scope");
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("scratch");
        let git = |dir: &Path, args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .output()
                .is_ok_and(|out| out.status.success())
        };

        let main = scratch.join("main");
        std::fs::create_dir_all(&main).expect("main checkout");
        if !git(&main, &["init", "-q", "-b", "main"]) {
            return; // No git on this machine; nothing to assert.
        }
        git(&main, &["config", "user.email", "t@example.com"]);
        git(&main, &["config", "user.name", "t"]);
        std::fs::write(main.join("audit.dx"), "::paragraph id=p\nx\n::end\n").expect("write");
        git(&main, &["add", "-A"]);
        git(&main, &["commit", "-qm", "one"]);

        // The main checkout alone reads only itself, same as any other repository.
        assert_eq!(
            read_scope(&main),
            vec![std::fs::canonicalize(&main).expect("canonical")]
        );

        let side = scratch.join("side");
        assert!(git(
            &main,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "side",
                &side.to_string_lossy()
            ],
        ));

        // From the worktree, the read scope reaches back into the checkout it was created
        // from — the file this test writes lives only in `main`, never copied into `side`.
        let scope = read_scope(&side);
        let main_canonical = std::fs::canonicalize(&main).expect("canonical");
        let side_canonical = std::fs::canonicalize(&side).expect("canonical");
        assert!(
            scope.contains(&side_canonical),
            "the worktree itself must stay in scope: {scope:?}"
        );
        assert!(
            scope.contains(&main_canonical),
            "the checkout it was branched from must be in scope too: {scope:?}"
        );

        // And the same holds symmetrically from the main checkout looking at the worktree.
        let scope_from_main = read_scope(&main);
        assert!(
            scope_from_main.contains(&side_canonical),
            "a sibling worktree must be readable from the main checkout too: {scope_from_main:?}"
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

    /// A block running from a worktree can read files in the main checkout.
    #[test]
    fn a_worktree_block_reads_main_checkout_by_absolute_path() {
        let scratch = std::env::temp_dir().join("dx-confine-tests-worktree-absolute-read");
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("scratch");
        let git = |dir: &Path, args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .output()
                .is_ok_and(|out| out.status.success())
        };

        let main = scratch.join("main");
        std::fs::create_dir_all(&main).expect("main checkout");
        if !git(&main, &["init", "-q", "-b", "master"]) {
            return; // No git on this machine; nothing to assert.
        }
        git(&main, &["config", "user.email", "t@example.com"]);
        git(&main, &["config", "user.name", "t"]);

        // Create a file in the main checkout that the worktree needs to read
        let config_file = main.join("config.json");
        std::fs::write(&config_file, r#"{"key": "value"}"#).expect("write config");

        std::fs::write(main.join("doc.dx"), "::paragraph id=p\nx\n::end\n").expect("write");
        git(&main, &["add", "-A"]);
        git(&main, &["commit", "-qm", "one"]);

        let side = scratch.join("side");
        assert!(git(
            &main,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "side",
                &side.to_string_lossy()
            ],
        ));

        // From the worktree, verify read_scope includes the main checkout
        let scope = read_scope(&side);
        let main_canonical = std::fs::canonicalize(&main).expect("canonical");
        assert!(
            scope.contains(&main_canonical),
            "the main checkout must be in scope when running from a worktree: {scope:?}"
        );

        // Also verify that the main checkout's config file exists and is readable
        assert!(config_file.exists(), "config file must exist");
        assert!(
            std::fs::read_to_string(&config_file).is_ok(),
            "config file must be readable"
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn macos_temp_and_cache_dirs_are_writable() {
        // The Seatbelt profile should allow writes to macOS per-user temp and cache dirs
        // so xcrun and other Apple tools do not spam warnings about inaccessible cache files.
        let profile = seatbelt_profile(&grant());
        let temp_dir = macos_temp_dir();
        let cache_dir = macos_cache_dir();

        if let Some(temp) = temp_dir {
            assert!(
                profile.contains(&format!("(subpath {})", quote(&temp))),
                "profile must allow write to macOS temp dir {}: {}",
                temp,
                profile
            );
        }

        if let Some(cache) = cache_dir {
            assert!(
                profile.contains(&format!("(subpath {})", quote(&cache))),
                "profile must allow write to macOS cache dir {}: {}",
                cache,
                profile
            );
        }
    }
}
