//! The Mac application, and how this device learns to open a `.dx` by double-clicking it.
//!
//! `DX.app` is the viewer: a window showing the rendered page, drawn by the `dx` inside that
//! same bundle. Finder only sends a document there once **LaunchServices** knows the
//! application exists and claims the type, and that is a per-device registration — which makes
//! it `dx setup`'s business, like the login service and every browser. One install, per
//! device, never one per program.
//!
//! Two things are deliberate here.
//!
//! The application is **copied into `/Applications` first**, for the same reason `dx setup`
//! copies the binary onto `PATH`: registering a bundle where it happens to be sitting binds
//! every double-click on the machine to a download folder or a build directory, and the next
//! build deletes it. What Finder opens should be somewhere applications live.
//!
//! And nothing is ever *named* that is not really on disk. A `dx` installed on its own — from
//! a build, a package manager, an agent's sandbox — has no application to register, and says
//! so, rather than sending a reader to hunt for a bundle that was never there.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::extension;
use crate::home;

/// The application bundle's name, wherever it is.
pub const APP: &str = "DX.app";

/// `lsregister`, the only interface LaunchServices offers for registering a bundle.
///
/// It is inside a private support directory of a system framework and has been at this path
/// since OS X 10.5. There is no public command; `open` registers only as a side effect of
/// launching, which is not something an installer should have to do to a reader's screen.
const LSREGISTER: &str =
    "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";

/// What `dx setup` did about the document viewer.
///
/// Every variant is something that actually happened on this machine, so the report can say it
/// plainly instead of describing an intention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The application is at this path and Finder now opens `.dx` documents with it.
    Registered(PathBuf),
    /// The application was copied here first, then registered.
    Installed(PathBuf),
    /// There is no `DX.app` on this machine to register.
    Absent,
    /// This is not macOS, where the whole idea lives.
    Elsewhere,
    /// Something was there to do and it failed, with the sentence saying what.
    Failed(String),
}

impl Outcome {
    /// One line for a report, describing what is true now.
    #[must_use]
    pub fn line(&self) -> String {
        match self {
            Self::Registered(app) => format!("  double-click  {}", app.display()),
            Self::Installed(app) => format!("  installed     {}", app.display()),
            Self::Absent => {
                "  double-click  no DX.app on this machine — it is what carries the viewer"
                    .to_string()
            }
            Self::Elsewhere => "  double-click  macOS only".to_string(),
            Self::Failed(why) => format!("  double-click  {why}"),
        }
    }
}

/// Put `DX.app` where applications live and tell LaunchServices about it.
///
/// Idempotent: an application already in place is registered again, which is how a rebuilt
/// bundle takes effect, and costs nothing when nothing changed.
#[must_use]
pub fn install() -> Outcome {
    if !cfg!(target_os = "macos") {
        return Outcome::Elsewhere;
    }
    let Some(source) = bundle() else {
        return Outcome::Absent;
    };

    let moved = match place(&source) {
        Ok(placed) => placed,
        Err(why) => return Outcome::Failed(why),
    };
    if let Err(why) = register(&moved.path) {
        return Outcome::Failed(why);
    }
    if moved.copied {
        Outcome::Installed(moved.path)
    } else {
        Outcome::Registered(moved.path)
    }
}

/// What `dx doctor` says about opening documents from Finder.
#[must_use]
pub fn status_lines() -> Vec<String> {
    if !cfg!(target_os = "macos") {
        return vec![Outcome::Elsewhere.line()];
    }
    match bundle() {
        Some(app) => vec![Outcome::Registered(app).line()],
        None => vec![Outcome::Absent.line()],
    }
}

/// The `DX.app` this machine has: the one this binary is running from, else an installed one.
///
/// The running bundle comes first for the same reason it does when finding the browser
/// extension — it was built beside the binary reading it, so it cannot be a version behind.
#[must_use]
pub fn bundle() -> Option<PathBuf> {
    running_from().or_else(installed)
}

/// The application bundle this binary is running inside, if it is running inside one.
fn running_from() -> Option<PathBuf> {
    let app = extension::app_bundle()?;
    is_bundle(&app).then_some(app)
}

/// A `DX.app` already sitting in one of the two places applications live.
fn installed() -> Option<PathBuf> {
    candidates().into_iter().find(|app| is_bundle(app))
}

/// Where an application may be installed, in the order they are preferred.
fn candidates() -> Vec<PathBuf> {
    let mut places = vec![PathBuf::from("/Applications").join(APP)];
    if let Some(home) = home::home() {
        places.push(home.join("Applications").join(APP));
    }
    places
}

/// Whether `path` is an application bundle and not merely a directory with the right name.
///
/// The `Info.plist` is what is checked: it is the one file every bundle has, so a folder
/// somebody named `DX.app` is not mistaken for an application and handed to LaunchServices.
fn is_bundle(path: &Path) -> bool {
    path.extension().is_some_and(|kind| kind == "app")
        && path.join("Contents").join("Info.plist").is_file()
}

/// An application in its final place, and whether getting it there meant copying.
struct Placed {
    path: PathBuf,
    copied: bool,
}

/// Ensure the application is somewhere applications live, copying it there if it is not.
///
/// A bundle already under an `Applications` directory is left exactly where it is: moving a
/// reader's application out from under them is not an installer's decision to make.
fn place(source: &Path) -> Result<Placed, String> {
    if in_applications(source) {
        return Ok(Placed {
            path: source.to_path_buf(),
            copied: false,
        });
    }

    let directory = writable_applications_dir()?;
    let target = directory.join(APP);
    if target == source {
        return Ok(Placed {
            path: target,
            copied: false,
        });
    }

    // The old bundle goes before the new one arrives. Copying over a bundle in place leaves
    // files from the previous version behind — which for a signed application means a seal
    // that no longer matches its contents, and macOS refusing to launch it.
    if is_bundle(&target) {
        std::fs::remove_dir_all(&target).map_err(|error| {
            format!(
                "could not replace {}: {error}. Quit it, drag it to the Trash, and run `dx setup` again.",
                target.display()
            )
        })?;
    }
    copy_bundle(source, &target)?;
    Ok(Placed {
        path: target,
        copied: true,
    })
}

/// Whether `app` already sits directly inside some `Applications` directory.
fn in_applications(app: &Path) -> bool {
    app.parent()
        .and_then(Path::file_name)
        .is_some_and(|name| name == "Applications")
}

/// `/Applications` when this user may write to it, else their own `~/Applications`.
///
/// `/Applications` is writable by administrators, which is most people on their own Mac, and
/// it is where an application belongs. A standard account gets the per-user directory instead
/// — LaunchServices treats both the same — rather than a prompt for a password.
fn writable_applications_dir() -> Result<PathBuf, String> {
    let shared = PathBuf::from("/Applications");
    if is_writable(&shared) {
        return Ok(shared);
    }
    let home = home::home().ok_or_else(|| {
        "no home directory, so there is nowhere to install DX.app. Set HOME and run `dx setup` \
         again."
            .to_string()
    })?;
    let personal = home.join("Applications");
    std::fs::create_dir_all(&personal)
        .map_err(|error| format!("could not create {}: {error}", personal.display()))?;
    Ok(personal)
}

/// Whether a directory can be written to, tested by writing rather than by reading permissions.
///
/// A mode bit says what the file system thinks; it does not know about a read-only volume, an
/// immutable flag, or a sandbox. Creating and removing a file is the question actually being
/// asked.
fn is_writable(directory: &Path) -> bool {
    let probe = directory.join(".dx-write-test");
    let written = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    written
}

/// Copy an application bundle, preserving everything a bundle needs to stay valid.
///
/// `ditto` rather than a recursive copy written here: a signed bundle carries its signature in
/// extended attributes as well as in files, and a copy that drops them produces an application
/// macOS refuses to launch. It is Apple's own tool for exactly this and is on every Mac.
fn copy_bundle(source: &Path, target: &Path) -> Result<(), String> {
    let output = Command::new("/usr/bin/ditto")
        .arg(source)
        .arg(target)
        .output()
        .map_err(|error| format!("could not run /usr/bin/ditto: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let complaint = String::from_utf8_lossy(&output.stderr);
    Err(format!(
        "could not copy {} to {}: {}",
        source.display(),
        target.display(),
        complaint.trim()
    ))
}

/// Tell LaunchServices about the application, so Finder routes `.dx` to it.
///
/// `-f` re-reads a bundle it already knows, which is what makes this idempotent and what makes
/// a rebuilt application take effect without a logout.
fn register(app: &Path) -> Result<(), String> {
    let output = Command::new(LSREGISTER)
        .arg("-f")
        .arg(app)
        .output()
        .map_err(|error| {
            format!("could not run {LSREGISTER}: {error}. Open {} once by hand and macOS will register it.", app.display())
        })?;
    if output.status.success() {
        return Ok(());
    }
    let complaint = String::from_utf8_lossy(&output.stderr);
    Err(format!(
        "could not register {}: {}",
        app.display(),
        complaint.trim()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory named like an application but without the one file every bundle has is not
    /// an application — and handing it to LaunchServices is how a double-click starts opening
    /// nothing at all.
    #[test]
    fn a_bundle_is_recognized_by_its_info_plist_and_not_by_its_name() {
        let root = std::env::temp_dir().join("dx-desktop-bundle-test");
        let _ = std::fs::remove_dir_all(&root);
        let app = root.join(APP);
        std::fs::create_dir_all(app.join("Contents")).expect("make the directory");
        assert!(!is_bundle(&app), "a bare directory is not an application");

        std::fs::write(app.join("Contents").join("Info.plist"), "<plist/>").expect("write");
        assert!(is_bundle(&app));

        assert!(
            !is_bundle(&root.join("DX")),
            "an extension alone is not enough"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_application_already_where_applications_live_is_left_alone() {
        assert!(in_applications(Path::new("/Applications/DX.app")));
        assert!(in_applications(Path::new("/Users/x/Applications/DX.app")));
        assert!(!in_applications(Path::new("/Users/x/Downloads/DX.app")));
        assert!(!in_applications(Path::new(
            "/Users/x/Desktop/DOC/packaging/build/DX.app"
        )));
    }

    /// Every outcome has to say something a reader can act on. An empty line — or one that
    /// names a path that is not there — is how someone ends up looking for an application
    /// nobody installed.
    #[test]
    fn every_outcome_reports_something_true() {
        let app = PathBuf::from("/Applications/DX.app");
        for outcome in [
            Outcome::Registered(app.clone()),
            Outcome::Installed(app),
            Outcome::Absent,
            Outcome::Elsewhere,
            Outcome::Failed("could not copy it".to_string()),
        ] {
            let line = outcome.line();
            assert!(line.trim_start().starts_with(|c: char| !c.is_whitespace()));
            assert!(line.len() > 20, "unhelpfully short: {line}");
        }
        assert!(Outcome::Absent.line().contains("no DX.app"));
    }

    /// The status lines are what `dx doctor` prints, and they must be true of *this* machine
    /// rather than of a machine where everything is installed.
    #[test]
    fn doctor_says_either_where_the_application_is_or_that_there_is_none() {
        let lines = status_lines();
        assert_eq!(lines.len(), 1);
        let line = &lines[0];
        assert!(line.contains("double-click"));
        if let Some(app) = bundle() {
            assert!(line.contains(&app.display().to_string()));
            assert!(is_bundle(&app), "a named application must really be there");
        }
    }
}
