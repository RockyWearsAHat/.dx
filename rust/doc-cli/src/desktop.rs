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
    /// The application is registered, but the `dx` inside it is not the binary running this
    /// command — the viewer would keep rendering with the older engine. A bare binary has no
    /// bundle to install (that is by design), so the remedy is named instead of implied.
    StaleEngine(PathBuf),
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
            Self::StaleEngine(app) => format!(
                "  double-click  {} carries a different dx than this one — rebuild it with \
                 packaging/build-app.sh and run that bundle's `dx setup`",
                app.display()
            ),
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
    } else if carries_other_engine(&moved.path) {
        Outcome::StaleEngine(moved.path)
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
        Some(app) if carries_other_engine(&app) => vec![Outcome::StaleEngine(app).line()],
        Some(app) => vec![Outcome::Registered(app).line()],
        None => vec![Outcome::Absent.line()],
    }
}

/// Whether the application's bundled `dx` is a different binary than the one answering this
/// command.
///
/// This is what makes a stale viewer *visible*: a `dx setup` run from a bare binary leaves an
/// installed `DX.app` in place — a bare binary has no bundle to copy, and that is right — but
/// saying nothing about it once let the installed application render with an older engine for
/// days while everyone believed setup had refreshed everything. A read-only comparison; when
/// either side cannot be read there is nothing to claim, so nothing is claimed.
fn carries_other_engine(app: &Path) -> bool {
    let Ok(current) = std::env::current_exe() else {
        return false;
    };
    let bundled = app.join("Contents").join("MacOS").join("dx");
    same_link(&bundled, &current) == Some(false)
}

/// Whether two Mach-O files came out of the same link. `None` when either cannot be read or
/// parsed, which is "cannot tell", not "different".
///
/// The question is *which engine is in there*, and comparing bytes cannot answer it: the
/// bundle is code-signed on the way into `DX.app`, so the bundled `dx` and the `dx` it was
/// built from are never byte-identical — which made every `dx doctor` on a developer's own
/// machine report a stale viewer that was in fact the same build. The linker's `LC_UUID` is
/// the identity of the link itself; `codesign` rewrites the signature and `__LINKEDIT` and
/// leaves it alone, so two files with the same UUID hold the same engine.
fn same_link(a: &Path, b: &Path) -> Option<bool> {
    Some(link_uuids(a)? == link_uuids(b)?)
}

/// The `LC_UUID` of every architecture in the Mach-O file at `path`, in slice order.
///
/// `None` for anything this cannot read or does not recognize as a Mach-O carrying a UUID —
/// a truncated file, a shell script, a header whose counts do not fit the bytes present.
/// Unrecognized is always "cannot tell": claiming a difference from a file we failed to
/// parse would send a reader off to rebuild an application that was perfectly current.
fn link_uuids(path: &Path) -> Option<Vec<[u8; 16]>> {
    let bytes = std::fs::read(path).ok()?;
    match u32::from_le_bytes(*read_array::<4>(&bytes, 0)?) {
        // A universal binary: a big-endian table of (architecture, offset, size) records,
        // each naming a thin Mach-O inside this same file.
        FAT_MAGIC | FAT_MAGIC_64 => fat_uuids(&bytes),
        _ => Some(vec![thin_uuid(&bytes)?]),
    }
}

/// `LC_UUID`, the load command carrying the linker's identity for the build.
const LC_UUID: u32 = 0x1b;
/// A 64-bit thin Mach-O, as its first four bytes read little-endian.
const MH_MAGIC_64: u32 = 0xfeed_facf;
/// A 32-bit thin Mach-O, whose header is four bytes shorter.
const MH_MAGIC_32: u32 = 0xfeed_face;
/// A universal binary — the bytes `ca fe ba be`, read little-endian like every other magic
/// here. Its own header and tables are big-endian, whatever the slices inside it are.
const FAT_MAGIC: u32 = 0xbeba_feca;
/// A universal binary with 64-bit offsets (`ca fe ba bf`), whose records are twelve bytes
/// longer.
const FAT_MAGIC_64: u32 = 0xbfba_feca;

/// The UUID of every slice of a universal binary, or `None` if any slice cannot be read.
fn fat_uuids(bytes: &[u8]) -> Option<Vec<[u8; 16]>> {
    let wide = u32::from_le_bytes(*read_array::<4>(bytes, 0)?) == FAT_MAGIC_64;
    let count = u32::from_be_bytes(*read_array::<4>(bytes, 4)?) as usize;
    let record = if wide { 32 } else { 20 };
    let mut uuids = Vec::with_capacity(count);
    for index in 0..count {
        // Each record is cputype, cpusubtype, offset, size, align — the offset is the third
        // field, and is 64 bits wide only in the `FAT_MAGIC_64` form.
        let at = 8 + index.checked_mul(record)?;
        let offset = if wide {
            u64::from_be_bytes(*read_array::<8>(bytes, at + 8)?) as usize
        } else {
            u32::from_be_bytes(*read_array::<4>(bytes, at + 8)?) as usize
        };
        uuids.push(thin_uuid(bytes.get(offset..)?)?);
    }
    (!uuids.is_empty()).then_some(uuids)
}

/// The `LC_UUID` of a thin Mach-O starting at the front of `bytes`.
fn thin_uuid(bytes: &[u8]) -> Option<[u8; 16]> {
    let commands_at = match u32::from_le_bytes(*read_array::<4>(bytes, 0)?) {
        MH_MAGIC_64 => 32,
        MH_MAGIC_32 => 28,
        _ => return None,
    };
    let count = u32::from_le_bytes(*read_array::<4>(bytes, 16)?) as usize;

    let mut at = commands_at;
    for _ in 0..count {
        let kind = u32::from_le_bytes(*read_array::<4>(bytes, at)?);
        let size = u32::from_le_bytes(*read_array::<4>(bytes, at + 4)?) as usize;
        // A command that does not advance is a file that would loop forever, and a command
        // smaller than its own header is not one.
        if size < 8 {
            return None;
        }
        if kind == LC_UUID {
            return read_array::<16>(bytes, at + 8).copied();
        }
        at = at.checked_add(size)?;
    }
    None
}

/// The `N` bytes at `offset`, or `None` when the file is too short to hold them.
fn read_array<const N: usize>(bytes: &[u8], offset: usize) -> Option<&[u8; N]> {
    bytes.get(offset..offset.checked_add(N)?)?.try_into().ok()
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

    /// An empty directory of this test's own, so one test's files cannot be another's.
    fn scratch(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dx-desktop-tests-{label}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch");
        root
    }

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
            Outcome::Installed(app.clone()),
            Outcome::StaleEngine(app),
            Outcome::Absent,
            Outcome::Elsewhere,
            Outcome::Failed("could not copy it".to_string()),
        ] {
            let line = outcome.line();
            assert!(line.trim_start().starts_with(|c: char| !c.is_whitespace()));
            assert!(line.len() > 20, "unhelpfully short: {line}");
        }
        assert!(Outcome::Absent.line().contains("no DX.app"));
        // A stale viewer names its remedy — the rebuild, and the setup run from the bundle.
        let stale = Outcome::StaleEngine(PathBuf::from("/Applications/DX.app")).line();
        assert!(stale.contains("packaging/build-app.sh"), "{stale}");
        assert!(stale.contains("dx setup"), "{stale}");
    }

    /// A 64-bit thin Mach-O carrying one `LC_UUID`, preceded by `padding` unrelated load
    /// commands so the walk has to reach it rather than land on it.
    fn mach_o(uuid: u8, padding: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&MH_MAGIC_64.to_le_bytes());
        bytes.extend_from_slice(&[0; 12]); // cputype, cpusubtype, filetype
        bytes.extend_from_slice(&(padding + 1).to_le_bytes()); // ncmds
        bytes.extend_from_slice(&[0; 12]); // sizeofcmds, flags, reserved
        for _ in 0..padding {
            bytes.extend_from_slice(&0x19_u32.to_le_bytes()); // LC_SEGMENT_64, near enough
            bytes.extend_from_slice(&16_u32.to_le_bytes());
            bytes.extend_from_slice(&[0; 8]);
        }
        bytes.extend_from_slice(&LC_UUID.to_le_bytes());
        bytes.extend_from_slice(&24_u32.to_le_bytes());
        bytes.extend_from_slice(&[uuid; 16]);
        bytes
    }

    /// Same link, same `LC_UUID` — and that has to survive the one thing that happens to the
    /// bundled copy and to nothing else: `codesign` rewrites the signature and `__LINKEDIT`,
    /// which is why the byte comparison this replaced called every developer's own build
    /// stale, on every `dx doctor`, forever.
    #[test]
    fn the_same_link_is_recognized_through_a_signature_that_changed_the_bytes() {
        let root = scratch("uuid");
        let built = root.join("dx");
        let signed = root.join("dx-signed");
        std::fs::write(&built, mach_o(0xab, 2)).expect("write");

        let mut resigned = mach_o(0xab, 2);
        resigned.extend_from_slice(b"a signature that was not here before");
        std::fs::write(&signed, resigned).expect("write");

        assert_eq!(same_link(&built, &signed), Some(true));
        assert_eq!(link_uuids(&built), Some(vec![[0xab; 16]]));
    }

    /// A different link is a different engine, which is the whole point of the check.
    #[test]
    fn a_different_link_is_reported_as_a_different_engine() {
        let root = scratch("uuid-differs");
        let one = root.join("one");
        let other = root.join("other");
        std::fs::write(&one, mach_o(0x01, 0)).expect("write");
        std::fs::write(&other, mach_o(0x02, 3)).expect("write");
        assert_eq!(same_link(&one, &other), Some(false));
    }

    /// "Cannot parse" is "cannot tell", never "stale": a truncated or unrecognized file must
    /// not send a reader off to rebuild an application that was already current.
    #[test]
    fn an_unreadable_or_unparseable_file_declines_to_guess() {
        let root = scratch("uuid-garbage");
        let good = root.join("dx");
        std::fs::write(&good, mach_o(0x07, 1)).expect("write");

        let cases: [(&str, Vec<u8>); 5] = [
            ("empty", Vec::new()),
            ("not mach-o", b"#!/bin/sh\necho hi\n".to_vec()),
            ("truncated header", mach_o(0x07, 0)[..20].to_vec()),
            ("truncated command", {
                let bytes = mach_o(0x07, 0);
                bytes[..bytes.len() - 4].to_vec()
            }),
            ("no LC_UUID", {
                let mut bytes = mach_o(0x07, 1);
                bytes.truncate(32 + 16); // the padding command alone
                bytes[16..20].copy_from_slice(&1_u32.to_le_bytes());
                bytes
            }),
        ];
        for (label, bytes) in cases {
            let path = root.join(label.replace(' ', "-"));
            std::fs::write(&path, &bytes).expect("write");
            assert_eq!(link_uuids(&path), None, "{label}");
            assert_eq!(same_link(&good, &path), None, "{label}");
        }

        assert_eq!(same_link(&good, &root.join("missing")), None);
    }

    /// A universal binary holds one engine per architecture, and all of them have to match.
    #[test]
    fn a_universal_binary_is_compared_slice_by_slice() {
        let root = scratch("uuid-fat");
        let two = |first: u8, second: u8| {
            let (a, b) = (mach_o(first, 0), mach_o(second, 1));
            let (offset_a, offset_b) = (8 + 20 * 2, 8 + 20 * 2 + a.len());
            let mut bytes = Vec::new();
            bytes.extend_from_slice(&FAT_MAGIC.to_le_bytes());
            bytes.extend_from_slice(&2_u32.to_be_bytes());
            for (offset, slice) in [(offset_a, &a), (offset_b, &b)] {
                bytes.extend_from_slice(&[0; 8]); // cputype, cpusubtype
                bytes.extend_from_slice(&(offset as u32).to_be_bytes());
                bytes.extend_from_slice(&(slice.len() as u32).to_be_bytes());
                bytes.extend_from_slice(&[0; 4]); // align
            }
            bytes.extend_from_slice(&a);
            bytes.extend_from_slice(&b);
            bytes
        };

        let universal = root.join("fat");
        let rebuilt = root.join("fat-again");
        std::fs::write(&universal, two(0x11, 0x22)).expect("write");
        std::fs::write(&rebuilt, two(0x11, 0x33)).expect("write");

        assert_eq!(
            link_uuids(&universal),
            Some(vec![[0x11; 16], [0x22; 16]]),
            "both slices, in order"
        );
        assert_eq!(same_link(&universal, &universal), Some(true));
        assert_eq!(
            same_link(&universal, &rebuilt),
            Some(false),
            "one slice relinked is a different engine"
        );

        let mut lying = two(0x11, 0x22);
        lying[4..8].copy_from_slice(&9_u32.to_be_bytes()); // more slices than are present
        let bad = root.join("fat-lying");
        std::fs::write(&bad, lying).expect("write");
        assert_eq!(link_uuids(&bad), None, "a count the file cannot back up");
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
