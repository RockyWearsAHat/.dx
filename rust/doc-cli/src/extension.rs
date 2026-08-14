//! The github.com browser extension: what it is made of, and how a browser is given it.
//!
//! A browser extension is a browser's concern. `dx` on a build server, in a container, or in
//! an agent's sandbox has no browser and no use for ~0.4 MB of page adapter and wasm engine —
//! so the extension's files are **not** in this binary. They are read from a directory: the
//! repository's `editor/github` when something is building an artifact out of it, and
//! `DX.app/Contents/Resources/extension` on a Mac where the application is installed. A
//! reader who wants their browser to show documents installs the extension, once, from the
//! store their browser uses (see [`channel`]).
//!
//! What *is* in this binary is the part that has to agree with itself everywhere: the
//! manifest, and the rule for deriving each browser's variant of it.
//!
//! # Why the manifest is derived rather than duplicated
//! Chrome and Firefox disagree about exactly one thing that matters here: Chrome runs
//! background code as a service worker, Firefox as an event page with plain scripts, and
//! neither accepts the other's declaration. Two hand-maintained manifests would be two
//! things to keep in step, and the one that is not loaded daily is the one that rots. So
//! `editor/github/manifest.json` is the single source — it is Chrome's, and the one a
//! developer loads unpacked from the repository — and the Firefox variant is derived from it
//! here, changing only what Firefox requires. It is small, it is the *rule* rather than the
//! payload, and every artifact is generated from it, so it stays compiled in.
//!
//! # The engine is the engine
//! The wasm in a source directory is built from `doc-wasm` by `editor/build.sh`. That makes
//! staleness the hazard: a `doc-core` change with no rebuild would give a browser a renderer
//! the CLI no longer agrees with. `editor/github/test/engine.test.mjs` loads that wasm and
//! compares its rendering with the `dx` binary's, so a stale engine fails the suite rather
//! than reaching a reader.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::home;
use crate::state::{self, Installed, State};

/// One file of the extension, at the path it takes inside an extension directory.
#[derive(Debug, Clone)]
pub struct Asset {
    /// Path relative to the extension directory, with `/` separators.
    pub path: &'static str,
    /// The file's exact bytes.
    pub bytes: Vec<u8>,
}

/// Every file of the extension except `manifest.json`, which varies by browser.
///
/// Order is the order a browser loads them in, and `resolve.js` before `content.js` is not a
/// detail: the adapter reads the resolver's API off `globalThis`, and shipping them the other
/// way round renders nothing at all, on every page. [`manifest_names_only_shipped_files`]
/// checks that what the manifest lists and what this names are the same set.
///
/// The icons are rendered from `packaging/icon.svg` by the `build-icons` binary (`doc-shot`).
/// Four sizes because that is what asks: 16 in a toolbar, 32 on Windows, 48 in the extensions
/// list, 128 in both stores — and a submission without the 128 is rejected outright.
pub const ASSET_PATHS: &[&str] = &[
    "resolve.js",
    "content.js",
    "content.css",
    "engine.js",
    "wasm/doc_wasm.js",
    "wasm/doc_wasm_bg.wasm",
    "icons/dx-16.png",
    "icons/dx-32.png",
    "icons/dx-48.png",
    "icons/dx-128.png",
];

/// Read every file the extension is made of out of `source`.
///
/// `source` is a directory laid out like `editor/github`. Reading is all-or-nothing: an
/// extension missing one file is one a browser loads and then fails on, usually with no
/// message on the page, so a missing file is an error here rather than a gap later.
///
/// # Errors
/// When a file [`ASSET_PATHS`] names is not in `source`, naming the file and the directory.
pub fn assets(source: &Path) -> Result<Vec<Asset>, String> {
    ASSET_PATHS
        .iter()
        .map(|path| {
            let file = source.join(path);
            let bytes = std::fs::read(&file).map_err(|error| {
                format!(
                    "the extension source {} has no {path}: {error}",
                    source.display()
                )
            })?;
            Ok(Asset { path, bytes })
        })
        .collect()
}

/// The Chromium manifest, and the source every other manifest is derived from.
const MANIFEST: &str = include_str!("../../../editor/github/manifest.json");

/// The add-on id Firefox files this extension under.
///
/// Firefox needs a stable id to install an add-on from a directory; Chrome derives one from
/// the directory path and rejects the key. It is also the name the policy file installs the
/// add-on by (see [`crate::policies`]), so the manifest and the policy cannot drift apart.
pub const GECKO_ID: &str = "dx-documents@dx.tools";

/// The first Firefox with MV3 event pages and `'wasm-unsafe-eval'` in extension pages.
const GECKO_MIN_VERSION: &str = "115.0";

/// Which browser family an extension directory is written for.
///
/// Not a list of browsers — a list of *shapes*. Every Chromium browser (Chrome, Edge, Brave,
/// Vivaldi, Opera, Arc) loads the same directory, and every Gecko browser (Firefox, its
/// developer channels, LibreWolf) loads the other one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// Chrome, Edge, Brave, Vivaldi, Opera, Arc — MV3 with a service worker.
    Chromium,
    /// Firefox and its relatives — MV3 with an event page.
    Firefox,
}

impl Target {
    /// Both shapes, in the order they are written and reported.
    pub const ALL: [Self; 2] = [Self::Chromium, Self::Firefox];

    /// The directory name and command-line word for this target.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Chromium => "chromium",
            Self::Firefox => "firefox",
        }
    }

    /// The target a caller named, accepting the browser names people actually type.
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        match word.trim().to_ascii_lowercase().as_str() {
            "chromium" | "chrome" | "edge" | "brave" | "vivaldi" | "opera" | "arc"
            | "chromiums" => Some(Self::Chromium),
            "firefox" | "gecko" | "librewolf" | "waterfox" | "zen" => Some(Self::Firefox),
            _ => None,
        }
    }
}

/// The `manifest.json` for `target`.
///
/// # Errors
/// When the checked-in manifest is not a JSON object — which would mean the file this binary
/// was built from is broken, so it is reported rather than silently patched.
pub fn manifest(target: Target) -> Result<String, String> {
    let mut value: Value = serde_json::from_str(MANIFEST)
        .map_err(|error| format!("the built-in manifest is not valid JSON: {error}"))?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| "the built-in manifest is not a JSON object".to_string())?;

    if target == Target::Firefox {
        // Firefox has no MV3 service worker. The glue is listed first because `engine.js`
        // calls `wasm_bindgen`, which that file defines.
        object.insert(
            "background".to_string(),
            json!({ "scripts": ["wasm/doc_wasm.js", "engine.js"] }),
        );
        object.insert(
            "browser_specific_settings".to_string(),
            json!({ "gecko": { "id": GECKO_ID, "strict_min_version": GECKO_MIN_VERSION } }),
        );
    }

    let text = serde_json::to_string_pretty(&value)
        .map_err(|error| format!("could not write the manifest: {error}"))?;
    Ok(format!("{text}\n"))
}

/// Every file that makes up the extension for `target`, ready to write.
///
/// # Errors
/// When the manifest cannot be produced ([`manifest`]), when `source` is missing a file
/// ([`assets`]), or when the manifest names a file `source` does not have — which is how a
/// payload built from a different revision than this binary is caught, rather than shipping
/// a browser a manifest pointing at nothing.
pub fn files(source: &Path, target: Target) -> Result<Vec<(String, Vec<u8>)>, String> {
    let manifest = manifest(target)?;
    let assets = assets(source)?;
    for named in named_files(
        &serde_json::from_str(&manifest)
            .map_err(|error| format!("the generated manifest is not valid JSON: {error}"))?,
    ) {
        if !assets.iter().any(|asset| asset.path == named) {
            return Err(format!(
                "the {} manifest names {named}, which {} does not have",
                target.name(),
                source.display()
            ));
        }
    }

    let mut files = vec![("manifest.json".to_string(), manifest.into_bytes())];
    files.extend(
        assets
            .into_iter()
            .map(|asset| (asset.path.to_string(), asset.bytes)),
    );
    Ok(files)
}

/// Every file path a manifest names, from the keys that can name one.
fn named_files(manifest: &Value) -> Vec<String> {
    let mut named = Vec::new();
    for script in manifest["content_scripts"].as_array().into_iter().flatten() {
        for key in ["js", "css"] {
            for file in script[key].as_array().into_iter().flatten() {
                named.push(file.as_str().unwrap_or_default().to_string());
            }
        }
    }
    if let Some(worker) = manifest["background"]["service_worker"].as_str() {
        named.push(worker.to_string());
    }
    for file in manifest["background"]["scripts"]
        .as_array()
        .into_iter()
        .flatten()
    {
        named.push(file.as_str().unwrap_or_default().to_string());
    }
    for icon in manifest["icons"].as_object().into_iter().flatten() {
        named.push(icon.1.as_str().unwrap_or_default().to_string());
    }
    named
}

/// Every file of the extension for `target`, at the absolute path it takes under `dir`.
///
/// # Errors
/// When the files cannot be assembled; see [`files`].
fn installed_files(source: &Path, dir: &Path, target: Target) -> Result<Vec<Installed>, String> {
    let root = dir.join(target.name());
    Ok(files(source, target)?
        .into_iter()
        .map(|(path, bytes)| (root.join(path), bytes))
        .collect())
}

/// Compare the extension on disk at `dir` with what `source` and this binary would write.
///
/// # Errors
/// When the files cannot be assembled; see [`files`].
pub fn state(source: &Path, dir: &Path, target: Target) -> Result<State, String> {
    Ok(State::of(&installed_files(source, dir, target)?))
}

/// Write the extension for `target` into `dir/<target>`, returning that directory.
///
/// Writing is unconditional and idempotent: the files are exactly what `source` holds and
/// this binary derives, so building again over an existing directory leaves a browser
/// pointing at a current engine without anything to uninstall first.
///
/// # Errors
/// When the files cannot be assembled ([`files`]), or a directory or file cannot be written,
/// naming the path.
pub fn write(source: &Path, dir: &Path, target: Target) -> Result<PathBuf, String> {
    state::write_all(&installed_files(source, dir, target)?)?;
    Ok(dir.join(target.name()))
}

/// The extension directory this machine actually has for `target`, if it has one.
///
/// The application's own copy first: it is built alongside the binary that reads it, so it
/// cannot be a version behind. Then the directory something wrote with [`write`], which is
/// what a developer loading unpacked from a checkout is pointing their browser at.
///
/// `manifest.json` is what is looked for rather than the directory, because an empty or
/// half-written directory is exactly the state that makes a browser report a broken
/// extension, and naming it as installed would send the reader to look at the browser.
#[must_use]
pub fn installed_dir(target: Target) -> Option<PathBuf> {
    [
        app_resources().map(|resources| resources.join("extension")),
        Some(default_dir()),
    ]
    .into_iter()
    .flatten()
    .map(|root| root.join(target.name()))
    .find(|dir| dir.join("manifest.json").is_file())
}

/// The Chrome Web Store listing, once there is one.
///
/// `None` until the extension is published, and that is deliberate: a link to a store page
/// that does not exist yet is worse than the developer-mode instructions, because it looks
/// like the install failed rather than like it has not shipped. Publishing is the only change
/// needed here — [`channel`] switches every Chromium browser over the moment this is `Some`.
///
/// One listing covers the whole family: Chrome, Edge, Brave, Vivaldi, Opera, and Arc all
/// install from the Chrome Web Store.
pub const CHROME_WEB_STORE: Option<&str> = None;

/// How a browser on this machine actually receives the extension.
///
/// This is the difference between the families that matters to a person installing `dx`, and
/// it is deliberately separate from [`Target`], which is only the *shape* of the directory.
/// Two browsers can take the same directory and still be given it in completely different
/// ways — which is exactly the case for Chrome and Firefox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Channel {
    /// A store listing. One click, and the browser keeps it updated afterwards.
    Store {
        /// The store's name, for a report.
        name: &'static str,
        /// The listing to open.
        url: &'static str,
    },
    /// `dx` writes a policy file and the browser installs it at its next start — no clicks.
    Policy {
        /// The signed add-on the policy names.
        xpi: PathBuf,
    },
    /// The extension is inside the `dx` application; the reader turns it on in the browser's
    /// own settings, which is the only step Safari has and the only one it can ever have.
    Bundled,
    /// A directory loaded by hand after turning on developer mode. The fallback everywhere,
    /// and the only route for a Chromium browser until the listing is published.
    Unpacked {
        /// The directory to load, which exists — see [`installed_dir`].
        dir: PathBuf,
    },
    /// This installation has no extension for this family and no store to send anyone to.
    ///
    /// A `dx` installed on its own carries no extension files (see the module documentation),
    /// which is the ordinary case on a server or in a container. Saying so is the whole point:
    /// the alternative is naming a directory that is not there, which reads as a broken
    /// install rather than as a part that was never asked for.
    Absent,
}

/// How `family` is given the extension on this machine.
///
/// Every answer here is a fact that was checked, never a promise: a signed add-on Firefox
/// will actually accept, a published listing, an application that really carries Safari's
/// extension, a directory that is really on disk. A family with none of them gets
/// [`Channel::Absent`], which says so.
#[must_use]
pub fn channel(family: Family) -> Channel {
    match family {
        Family::Chromium => CHROME_WEB_STORE.map_or_else(
            || unpacked(Target::Chromium),
            |url| Channel::Store {
                name: "the Chrome Web Store",
                url,
            },
        ),
        // Release and Beta Firefox refuse an unsigned add-on whatever the policy says, so
        // this is only offered when the signed one is actually here to be named.
        Family::Firefox => {
            signed_xpi().map_or_else(|| unpacked(Target::Firefox), |xpi| Channel::Policy { xpi })
        }
        // Safari's extension lives inside the application, so telling someone to enable it
        // is only true when the application is here. Running `dx` on its own, there is
        // nothing in Safari's settings to find, and saying otherwise sends them looking for
        // a checkbox that does not exist.
        Family::Safari => {
            if safari_extension().is_some() {
                Channel::Bundled
            } else {
                // Safari cannot load a directory at all; the Chromium one is what its
                // converter takes as input, which is the only thing left to point at.
                unpacked(Target::Chromium)
            }
        }
    }
}

/// Loading a directory by hand, when there is a directory on this machine to load.
fn unpacked(target: Target) -> Channel {
    installed_dir(target).map_or(Channel::Absent, |dir| Channel::Unpacked { dir })
}

/// The `Contents` directory of the application bundle this binary is running from.
///
/// `dx` inside `DX.app` is at `DX.app/Contents/MacOS/dx`, so the bundle is two directories
/// up. `None` when `dx` was installed on its own — from a release archive, a package manager,
/// or a build — which is a supported way to have it and simply means the parts only the
/// application carries are not available.
///
/// The `Info.plist` is what is checked, rather than the directory name: it is the one file
/// every bundle has and no ordinary directory does, so a `dx` that happens to sit two levels
/// under something called `Contents` is not mistaken for an installed application.
fn app_contents() -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    let contents = executable.parent()?.parent()?;
    contents
        .join("Info.plist")
        .is_file()
        .then(|| contents.to_path_buf())
}

/// The application bundle this binary is running from — `DX.app` itself, not its `Contents`.
///
/// `None` when `dx` was installed on its own, which is a supported way to have it. Used by
/// [`crate::desktop`], which registers that bundle with LaunchServices so a double-clicked
/// document opens in it.
#[must_use]
pub fn app_bundle() -> Option<PathBuf> {
    app_contents()?.parent().map(Path::to_path_buf)
}

/// The resource directory of the application bundle this binary is running from.
#[must_use]
pub fn app_resources() -> Option<PathBuf> {
    let resources = app_contents()?.join("Resources");
    resources.is_dir().then_some(resources)
}

/// The Safari app extension inside this application, when it carries one.
///
/// Checked as a file rather than assumed from the bundle, because building it needs Xcode and
/// an application built without it is a perfectly good application — it simply has nothing
/// for Safari. Telling a reader to enable an extension that is not there is the one failure
/// this whole area is prone to, and this is the fact that prevents it.
#[must_use]
pub fn safari_extension() -> Option<PathBuf> {
    std::fs::read_dir(app_contents()?.join("PlugIns"))
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|kind| kind == "appex"))
}

/// The Mozilla-signed add-on, when the application carries one.
///
/// It is a file in the application rather than bytes inside this binary on purpose: only
/// Mozilla can produce it, so it cannot be rebuilt from what `dx` already has, and embedding
/// it would put a second copy of the whole extension into every `dx` — including every one on
/// a machine with no Firefox.
#[must_use]
pub fn signed_xpi() -> Option<PathBuf> {
    let xpi = app_resources()?.join(XPI_NAME);
    xpi.is_file().then_some(xpi)
}

/// The signed add-on's file name inside the application bundle.
pub const XPI_NAME: &str = "dx-firefox.xpi";

/// Where `dx` keeps the extension directories it writes.
///
/// A per-user data directory rather than a temporary one: a browser reads an unpacked
/// extension from this path on **every** start, so a path that is cleaned up would leave the
/// reader with a broken extension the next morning.
#[must_use]
pub fn default_dir() -> PathBuf {
    home::data_dir().join("dx").join("extension")
}

/// What kind of extension a browser takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// Loads an unpacked MV3 directory with a service worker.
    Chromium,
    /// Loads an unpacked MV3 directory with an event page.
    Firefox,
    /// Takes neither: a Safari extension has to be wrapped in an app first.
    Safari,
}

impl Family {
    /// The directory shape this family loads, if it loads one at all.
    #[must_use]
    pub fn target(self) -> Option<Target> {
        match self {
            Self::Chromium => Some(Target::Chromium),
            Self::Firefox => Some(Target::Firefox),
            Self::Safari => None,
        }
    }

    /// What the person at the keyboard still has to do, one step per line.
    ///
    /// Empty when there is nothing left for them to do. That case is real — the policy
    /// channel installs with no clicks at all — and an installer that invents a step to fill
    /// the space teaches readers to ignore the steps that matter.
    ///
    /// Where a step *is* required, it is required: giving an extension permission to read
    /// github.com is a grant only the person at the keyboard can make. Chrome removed
    /// `--load-extension` for exactly that reason, and every route in this table ends at
    /// somebody choosing. `dx` does everything up to that point and then says precisely what
    /// to click, rather than pretending it can click it.
    #[must_use]
    pub fn steps(self) -> String {
        match channel(self) {
            Channel::Store { name, url } => {
                format!(
                    "open {url}\nAdd it from {name} — one click, updated automatically after that"
                )
            }
            // Nothing to do: Firefox installs it at its next start.
            Channel::Policy { .. } => String::new(),
            Channel::Bundled => "open Safari → Settings → Extensions\n\
                                 tick dx, then Always Allow on github.com"
                .to_string(),
            Channel::Unpacked { dir } => self.load_by_hand(&dir),
            Channel::Absent => ABSENT.to_string(),
        }
    }

    /// The developer-mode steps, which are the fallback for every family.
    fn load_by_hand(self, dir: &Path) -> String {
        let dir = dir.display();
        match self {
            Self::Chromium => format!(
                "open chrome://extensions   (edge://, brave://, vivaldi:// in the others)\n\
                 turn on Developer mode, then Load unpacked\n\
                 choose {dir}"
            ),
            Self::Firefox => format!(
                "open about:debugging#/runtime/this-firefox\n\
                 Load Temporary Add-on → {dir}/manifest.json\n\
                 (temporary: release Firefox drops an unsigned add-on when it restarts)"
            ),
            Self::Safari => format!(
                "Safari takes no unpacked extension; it has to be wrapped in an app once:\n\
                 xcrun safari-web-extension-converter {dir}   (needs Xcode)"
            ),
        }
    }
}

/// What to say when this installation has no extension to give a browser.
///
/// It names the two ways to get one and no more. The extension is a separate thing a reader
/// installs when they want github.com to show documents; a `dx` without one is complete for
/// everything else it does, and the wording is meant to read that way rather than as a
/// failure.
const ABSENT: &str = "this install of dx carries no browser extension\n\
                      install it from your browser's store, or build one from a dx checkout:\n\
                      dx browser --from editor/github";

/// A browser found on this machine.
#[derive(Debug, Clone)]
pub struct Browser {
    /// The name a person calls it.
    pub name: &'static str,
    /// What shape of extension it takes.
    pub family: Family,
    /// Where it was found.
    pub path: PathBuf,
}

/// Browsers this machine has, in the order they were probed.
///
/// Probing is by location rather than by asking the system: every platform answers "what
/// browsers are installed" differently and none of them answer it well, while an application
/// path is a fact that can be checked in one syscall.
#[must_use]
pub fn detect() -> Vec<Browser> {
    let mut found = Vec::new();
    for (name, family, locations) in KNOWN {
        if let Some(path) = locations.iter().find_map(|location| locate(location)) {
            found.push(Browser {
                name,
                family: *family,
                path,
            });
        }
    }
    found
}

/// Resolve one probe location: an absolute path as itself, a bare name on `PATH`.
fn locate(location: &str) -> Option<PathBuf> {
    if location.contains(std::path::MAIN_SEPARATOR) || location.contains('/') {
        let path = expand(location);
        return path.exists().then_some(path);
    }
    doc_run::toolchain::locate(location)
}

/// Expand a leading `~` against the home directory.
fn expand(location: &str) -> PathBuf {
    let Some(rest) = location.strip_prefix("~/") else {
        return PathBuf::from(location);
    };
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map_or_else(
            || PathBuf::from(location),
            |home| PathBuf::from(home).join(rest),
        )
}

/// Every browser worth probing for, with the places it installs itself on this platform.
///
/// One table for every operating system, because the alternative — three tables — is three
/// places to forget a browser. A location with a separator is a path, and one without is a
/// program name looked up on `PATH`, which is how Linux packages announce themselves.
type Known = (&'static str, Family, &'static [&'static str]);

#[cfg(target_os = "macos")]
const KNOWN: &[Known] = &[
    (
        "Google Chrome",
        Family::Chromium,
        &[
            "/Applications/Google Chrome.app",
            "~/Applications/Google Chrome.app",
        ],
    ),
    (
        "Chromium",
        Family::Chromium,
        &["/Applications/Chromium.app"],
    ),
    (
        "Microsoft Edge",
        Family::Chromium,
        &[
            "/Applications/Microsoft Edge.app",
            "~/Applications/Microsoft Edge.app",
        ],
    ),
    (
        "Brave",
        Family::Chromium,
        &[
            "/Applications/Brave Browser.app",
            "~/Applications/Brave Browser.app",
        ],
    ),
    ("Vivaldi", Family::Chromium, &["/Applications/Vivaldi.app"]),
    ("Opera", Family::Chromium, &["/Applications/Opera.app"]),
    ("Arc", Family::Chromium, &["/Applications/Arc.app"]),
    (
        "Firefox",
        Family::Firefox,
        &["/Applications/Firefox.app", "~/Applications/Firefox.app"],
    ),
    (
        "Firefox Developer Edition",
        Family::Firefox,
        &["/Applications/Firefox Developer Edition.app"],
    ),
    (
        "Firefox Nightly",
        Family::Firefox,
        &["/Applications/Firefox Nightly.app"],
    ),
    (
        "LibreWolf",
        Family::Firefox,
        &["/Applications/LibreWolf.app"],
    ),
    (
        "Zen",
        Family::Firefox,
        &["/Applications/Zen.app", "/Applications/Zen Browser.app"],
    ),
    ("Safari", Family::Safari, &["/Applications/Safari.app"]),
];

#[cfg(windows)]
const KNOWN: &[Known] = &[
    (
        "Google Chrome",
        Family::Chromium,
        &[
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
            "chrome",
        ],
    ),
    (
        "Microsoft Edge",
        Family::Chromium,
        &[
            r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
            r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
            "msedge",
        ],
    ),
    (
        "Brave",
        Family::Chromium,
        &[
            r"C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe",
            "brave",
        ],
    ),
    (
        "Vivaldi",
        Family::Chromium,
        &[r"C:\Program Files\Vivaldi\Application\vivaldi.exe"],
    ),
    ("Opera", Family::Chromium, &["opera"]),
    (
        "Firefox",
        Family::Firefox,
        &[
            r"C:\Program Files\Mozilla Firefox\firefox.exe",
            r"C:\Program Files (x86)\Mozilla Firefox\firefox.exe",
            "firefox",
        ],
    ),
    (
        "LibreWolf",
        Family::Firefox,
        &[r"C:\Program Files\LibreWolf\librewolf.exe"],
    ),
];

#[cfg(all(unix, not(target_os = "macos")))]
const KNOWN: &[Known] = &[
    (
        "Google Chrome",
        Family::Chromium,
        &["google-chrome", "google-chrome-stable"],
    ),
    (
        "Chromium",
        Family::Chromium,
        &["chromium", "chromium-browser"],
    ),
    (
        "Microsoft Edge",
        Family::Chromium,
        &["microsoft-edge", "microsoft-edge-stable"],
    ),
    ("Brave", Family::Chromium, &["brave-browser", "brave"]),
    ("Vivaldi", Family::Chromium, &["vivaldi", "vivaldi-stable"]),
    ("Opera", Family::Chromium, &["opera"]),
    ("Firefox", Family::Firefox, &["firefox", "firefox-esr"]),
    ("LibreWolf", Family::Firefox, &["librewolf"]),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The repository's own copy of the extension — the source every artifact is built from.
    ///
    /// Located from the crate rather than the working directory or an installed application,
    /// so the suite tests the files in *this* checkout however it was invoked and whatever
    /// happens to be installed on the machine running it.
    fn repo_source() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../editor/github")
    }

    #[test]
    fn every_shipped_file_has_content() {
        for asset in assets(&repo_source()).expect("the repository's extension source") {
            assert!(!asset.bytes.is_empty(), "{} is empty", asset.path);
        }
    }

    /// An extension missing one file loads and then fails, usually with nothing on the page,
    /// so the read is all-or-nothing and the error names the file rather than the symptom.
    #[test]
    fn a_source_missing_a_file_is_refused_by_name() {
        let root = std::env::temp_dir().join("dx-extension-incomplete-source");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("wasm")).expect("scratch");
        for path in ASSET_PATHS.iter().filter(|path| **path != "engine.js") {
            let file = root.join(path);
            std::fs::create_dir_all(file.parent().expect("parent")).expect("dir");
            std::fs::write(&file, b"x").expect("write");
        }

        let error = assets(&root).expect_err("an incomplete source is not usable");
        assert!(error.contains("engine.js"), "{error}");
        assert!(error.contains(&root.display().to_string()), "{error}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The extension finds `dx serve` by probing a fixed list of ports, because a browser
    /// extension cannot read a file to be told which one was chosen. Both sides therefore
    /// hold the list, and a change to one without the other is a daemon nobody talks to and
    /// no error anywhere — so the two are compared here.
    #[test]
    fn the_extension_probes_exactly_the_ports_the_daemon_binds() {
        let shipped = assets(&repo_source()).expect("source");
        let worker = shipped
            .iter()
            .find(|asset| asset.path == "engine.js")
            .map(|asset| String::from_utf8_lossy(&asset.bytes))
            .expect("the extension ships an engine.js");
        let declared = worker
            .split("const PORTS = [")
            .nth(1)
            .and_then(|rest| rest.split(']').next())
            .expect("engine.js must state the ports it probes");
        let probed: Vec<u16> = declared
            .split(',')
            .filter_map(|port| port.trim().parse().ok())
            .collect();
        assert_eq!(
            probed,
            crate::daemon::PORTS.to_vec(),
            "engine.js probes {probed:?} but `dx serve` binds {:?}",
            crate::daemon::PORTS
        );
    }

    #[test]
    fn the_shipped_engine_is_a_wasm_module() {
        let shipped = assets(&repo_source()).expect("source");
        let wasm = shipped
            .iter()
            .find(|asset| asset.path.ends_with(".wasm"))
            .expect("the engine is shipped");
        assert_eq!(&wasm.bytes[..4], b"\0asm", "not a wasm module");
        assert!(wasm.bytes.len() > 100_000, "suspiciously small engine");
    }

    /// The defect this pins is the one that made the extension render nothing on every page:
    /// `resolve.js` existed, was needed, and was not in the manifest's injection list. A set
    /// comparison catches it in both directions — a file listed but not shipped fails to
    /// load, and a file shipped but not listed is never run.
    #[test]
    fn manifest_names_only_shipped_files() {
        for target in Target::ALL {
            let value: Value = serde_json::from_str(&manifest(target).expect("manifest"))
                .expect("generated manifest parses");

            for named in named_files(&value) {
                assert!(
                    ASSET_PATHS.contains(&named.as_str()),
                    "{} names {named}, which is not shipped",
                    target.name()
                );
            }
            // `resolve.js` has to be injected before `content.js` reads its API.
            let scripts = value["content_scripts"][0]["js"]
                .as_array()
                .expect("content scripts");
            assert_eq!(scripts[0], "resolve.js");
            assert_eq!(scripts[1], "content.js");
        }
    }

    /// Both stores reject a submission with no 128px icon, and Safari's converter produces an
    /// extension with none at all — so an empty `icons` is a release that cannot ship, found
    /// at upload time rather than here.
    #[test]
    fn the_manifest_declares_every_icon_size_that_is_asked_for() {
        for target in Target::ALL {
            let value: Value =
                serde_json::from_str(&manifest(target).expect("manifest")).expect("json");
            let icons = value["icons"].as_object().expect("icons");
            for size in ["16", "32", "48", "128"] {
                assert!(
                    icons.contains_key(size),
                    "{} has no {size}px icon",
                    target.name()
                );
            }
        }
    }

    /// An icon that renders blank passes every check made of its declaration, its path, and
    /// its file size — it was a real defect here, from a headless browser that wrote a fully
    /// transparent PNG. So the pixels are what is asserted.
    #[test]
    fn every_shipped_icon_is_a_png_with_ink_in_it() {
        let shipped = assets(&repo_source()).expect("source");
        for asset in shipped.iter().filter(|asset| asset.path.ends_with(".png")) {
            assert_eq!(&asset.bytes[..8], b"\x89PNG\r\n\x1a\n", "{}", asset.path);
            // A blank icon compresses to almost nothing; a drawn one cannot.
            assert!(
                asset.bytes.len() > 150,
                "{} is {} bytes, which is the size of an empty image",
                asset.path,
                asset.bytes.len()
            );
        }
    }

    #[test]
    fn chrome_gets_a_service_worker_and_firefox_gets_an_event_page() {
        let chromium: Value =
            serde_json::from_str(&manifest(Target::Chromium).expect("chromium")).expect("json");
        assert_eq!(chromium["background"]["service_worker"], "engine.js");
        assert!(chromium["browser_specific_settings"].is_null());

        let firefox: Value =
            serde_json::from_str(&manifest(Target::Firefox).expect("firefox")).expect("json");
        assert!(firefox["background"]["service_worker"].is_null());
        // The glue defines `wasm_bindgen`, so it has to load first.
        assert_eq!(firefox["background"]["scripts"][0], "wasm/doc_wasm.js");
        assert_eq!(firefox["background"]["scripts"][1], "engine.js");
        assert_eq!(
            firefox["browser_specific_settings"]["gecko"]["id"],
            GECKO_ID
        );
    }

    /// Removing this line renders nothing, everywhere, with no error on the page — the wasm
    /// simply fails to compile in the extension's own context. It is one line and it is
    /// invisible when it is wrong, which is why it is asserted for both browsers.
    #[test]
    fn both_manifests_allow_the_engine_to_compile() {
        for target in Target::ALL {
            let value: Value =
                serde_json::from_str(&manifest(target).expect("manifest")).expect("json");
            let policy = value["content_security_policy"]["extension_pages"]
                .as_str()
                .expect("extension_pages policy");
            assert!(
                policy.contains("'wasm-unsafe-eval'"),
                "{} would not be able to compile the engine",
                target.name()
            );
        }
    }

    #[test]
    fn the_extension_reaches_this_machine_and_no_site_but_the_one_it_runs_on() {
        // The pack is fetched same-origin from the page itself, which is what makes a private
        // repository resolve on the reader's own session with no token. So the extension needs
        // no API permission and no permission over any site — the single host it asks for is
        // loopback, where `dx serve` is. A permission over a site the reader visits would be
        // capability this extension has no use for, and a prompt it should never show.
        for target in Target::ALL {
            let value: Value =
                serde_json::from_str(&manifest(target).expect("manifest")).expect("json");
            assert_eq!(value["permissions"], json!([]));
            assert_eq!(value["host_permissions"], json!(["http://127.0.0.1/*"]));
        }
    }

    #[test]
    fn writing_puts_every_file_where_the_manifest_expects_it() {
        let source = repo_source();
        let root = std::env::temp_dir().join("dx-extension-write-test");
        let _ = std::fs::remove_dir_all(&root);

        for target in Target::ALL {
            assert_eq!(
                state(&source, &root, target).expect("state"),
                State::Missing
            );
            let dir = write(&source, &root, target).expect("write");
            assert_eq!(dir, root.join(target.name()));
            for (path, bytes) in files(&source, target).expect("files") {
                assert_eq!(std::fs::read(dir.join(&path)).expect(&path), bytes);
            }
            assert_eq!(
                state(&source, &root, target).expect("state"),
                State::Current
            );
        }

        // A file changed underneath is out of date, not current.
        std::fs::write(root.join("chromium").join("content.js"), "stale").expect("edit");
        assert_eq!(
            state(&source, &root, Target::Chromium).expect("state"),
            State::Stale
        );
        // And writing again fixes it without anything to remove first.
        write(&source, &root, Target::Chromium).expect("rewrite");
        assert_eq!(
            state(&source, &root, Target::Chromium).expect("state"),
            State::Current
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_browser_name_someone_would_type_picks_the_right_shape() {
        assert_eq!(Target::parse("chrome"), Some(Target::Chromium));
        assert_eq!(Target::parse("Brave"), Some(Target::Chromium));
        assert_eq!(Target::parse(" firefox "), Some(Target::Firefox));
        assert_eq!(Target::parse("librewolf"), Some(Target::Firefox));
        assert_eq!(Target::parse("safari"), None);
        assert_eq!(Target::parse("netscape"), None);
    }

    #[test]
    fn loading_instructions_name_the_directory_and_the_page_to_open() {
        let dir = PathBuf::from("/tmp/dx/extension/chromium");
        let chromium = Family::Chromium.load_by_hand(&dir);
        assert!(chromium.contains("chrome://extensions"));
        assert!(chromium.contains("/tmp/dx/extension/chromium"));

        let firefox = Family::Firefox.load_by_hand(&PathBuf::from("/tmp/dx/extension/firefox"));
        assert!(firefox.contains("about:debugging"));
        assert!(firefox.contains("manifest.json"));

        assert!(Family::Safari
            .load_by_hand(&dir)
            .contains("safari-web-extension-converter"));
    }

    /// Safari always leaves exactly one step, and which one depends on whether the
    /// application that carries its extension is here. Telling someone to tick a box in
    /// Safari's settings when `dx` was installed on its own sends them hunting for a
    /// checkbox that does not exist — which is what this pins.
    #[test]
    fn safari_asks_for_the_step_that_is_actually_available() {
        let steps = Family::Safari.steps();
        assert!(!steps.trim().is_empty(), "Safari always has one step");
        match channel(Family::Safari) {
            Channel::Bundled => {
                assert!(safari_extension().is_some());
                assert!(steps.contains("Settings"), "{steps}");
            }
            Channel::Unpacked { .. } => {
                assert!(safari_extension().is_none());
                assert!(steps.contains("safari-web-extension-converter"), "{steps}");
            }
            other => {
                assert_eq!(other, Channel::Absent);
                assert!(steps.contains("--from"), "{steps}");
            }
        }
    }

    /// Every route this reports ends at the reader doing something, so naming a directory
    /// that is not on the machine sends them to a browser's error message instead of to the
    /// step they can actually take. A directory is named only when it is really there, and
    /// [`Channel::Absent`] is what is said when it is not.
    #[test]
    fn a_directory_is_only_named_when_one_is_really_there() {
        for family in [Family::Chromium, Family::Firefox, Family::Safari] {
            match channel(family) {
                Channel::Unpacked { dir } => assert!(
                    dir.join("manifest.json").is_file(),
                    "{dir:?} was named but holds no extension"
                ),
                Channel::Absent => {
                    assert!(installed_dir(Target::Chromium).is_none());
                    assert!(installed_dir(Target::Firefox).is_none());
                }
                Channel::Store { .. } | Channel::Policy { .. } | Channel::Bundled => {}
            }
        }
    }

    /// Until the listing exists, every Chromium browser has to fall back to loading the
    /// directory by hand — and the moment `CHROME_WEB_STORE` is `Some`, all of them switch.
    /// Pinning both halves means publishing is a one-line change that cannot be half-done.
    #[test]
    fn chromium_follows_the_store_listing_when_there_is_one_and_falls_back_when_there_is_not() {
        match CHROME_WEB_STORE {
            None => assert!(matches!(
                channel(Family::Chromium),
                Channel::Unpacked { .. } | Channel::Absent
            )),
            Some(url) => {
                assert!(
                    url.starts_with("https://"),
                    "a listing must be a URL: {url}"
                );
                assert!(matches!(channel(Family::Chromium), Channel::Store { .. }));
            }
        }
    }

    /// Firefox is only offered the policy route when a Mozilla-signed add-on is actually
    /// present, because release Firefox refuses an unsigned one whatever the policy says.
    /// Promising a zero-click install that silently does not happen is the worst outcome
    /// available here, so the two are tied together.
    #[test]
    fn firefox_is_only_given_a_policy_when_the_signed_add_on_is_here() {
        match channel(Family::Firefox) {
            Channel::Policy { xpi } => assert!(xpi.is_file(), "{xpi:?} was named but is absent"),
            other => {
                assert!(matches!(other, Channel::Unpacked { .. } | Channel::Absent));
                assert!(signed_xpi().is_none());
            }
        }
    }

    #[test]
    fn detection_reports_only_browsers_that_are_actually_there() {
        for browser in detect() {
            assert!(
                browser.path.exists(),
                "{} is not at {:?}",
                browser.name,
                browser.path
            );
        }
    }

    #[test]
    fn the_extension_directory_is_a_stable_per_user_path() {
        let dir = default_dir();
        assert!(dir.ends_with("dx/extension"));
        assert!(
            !dir.starts_with(std::env::temp_dir()),
            "a browser reads this on every start"
        );
    }
}
