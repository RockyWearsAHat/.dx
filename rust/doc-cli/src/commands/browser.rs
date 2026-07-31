//! `dx browser` — make a browser show `.dx` documents on github.com.
//!
//! A committed `.dx` file is one line, and github.com shows that line: its file viewer picks
//! renderers from a fixed first-party list and no repository, app, or `.gitattributes` entry
//! can add to it (`docs/github.md` has the detail). The only place left to resolve a pointer
//! is the reader's own browser, which is why a browser extension exists at all.
//!
//! The extension's files are not in the binary — see [`crate::extension`] for why. This
//! command has two jobs, and which one it does depends on whether it was given a source:
//!
//! - `dx browser --from <dir>` **builds** a loadable directory per browser family out of a
//!   source laid out like `editor/github`. That is how the repository produces every
//!   artifact — `DX.app`'s copy, the Chrome Web Store archive, the add-on Mozilla signs — so
//!   what ships is what this command writes, byte for byte, and there is no second builder to
//!   keep in step.
//! - `dx browser` with nothing to build from **reports**: which browsers are on this machine,
//!   how each one takes the extension, and the one step each reserves for the person at the
//!   keyboard.

use std::path::{Path, PathBuf};

use crate::args::Args;
use crate::extension::{self, Browser, Channel, Family, Target};
use crate::policies;
use crate::state::State;

/// `dx browser` — build the extension when given a source, and say how to load it.
///
/// # Errors
/// When a named source is unusable or the extension cannot be written, naming the path.
pub fn run(args: &Args) -> Result<String, String> {
    let targets = targets(args)?;
    let mut out = String::from("dx browser\n\nextension\n");

    match args.value("from") {
        Some(source) => out.push_str(&build(Path::new(source), &directory(args), &targets)?),
        None => out.push_str(&installed(&targets)),
    }

    out.push_str(&summary(&targets));
    if args.present("open") {
        if let Some(dir) = targets
            .iter()
            .find_map(|target| extension::installed_dir(*target))
        {
            out.push_str(&reveal(&dir));
        }
    }
    Ok(out)
}

/// Write a loadable directory per family out of `source`, reporting where each one landed.
fn build(source: &Path, dir: &Path, targets: &[Target]) -> Result<String, String> {
    let mut out = String::new();
    for target in targets {
        let before = extension::state(source, dir, *target)?;
        let written = extension::write(source, dir, *target)?;
        out.push_str(&format!(
            "  {:<9} {}{}\n",
            target.name(),
            written.display(),
            match before {
                State::Current => " (unchanged)",
                State::Stale => " (updated)",
                State::Missing => " (new)",
            }
        ));
    }
    Ok(out)
}

/// Report the extension directories this machine already has, one line per family.
fn installed(targets: &[Target]) -> String {
    let mut out = String::new();
    for target in targets {
        match extension::installed_dir(*target) {
            Some(dir) => out.push_str(&format!("  {:<9} {}\n", target.name(), dir.display())),
            None => out.push_str(&format!("  {:<9} not in this install\n", target.name())),
        }
    }
    out
}

/// Where to write, defaulting to the per-user directory a browser can read on every start.
fn directory(args: &Args) -> PathBuf {
    args.value("dir")
        .map_or_else(extension::default_dir, PathBuf::from)
}

/// The families to write, all of them unless the caller named one.
fn targets(args: &Args) -> Result<Vec<Target>, String> {
    let Some(word) = args.value("browser").or_else(|| args.value("target")) else {
        return Ok(Target::ALL.to_vec());
    };
    Target::parse(word).map(|target| vec![target]).ok_or_else(|| {
        format!(
            "`{word}` is not a browser dx can write an extension for.\n\
             Chromium browsers (chrome, edge, brave, vivaldi, opera, arc) take `--browser chrome`,\n\
             Firefox and its relatives take `--browser firefox`.\n\
             Safari needs a one-time Xcode conversion — run `dx browser` to see the command."
        )
    })
}

/// The browsers this machine has, and what each one needs next.
fn summary(targets: &[Target]) -> String {
    let found = extension::detect();
    if found.is_empty() {
        return format!(
            "\nNo browser found in the usual places. A directory above loads in any\n\
             Chromium browser (Developer mode → Load unpacked) or Firefox\n\
             (about:debugging → Load Temporary Add-on → manifest.json).\n\
             Reported for: {}\n",
            targets
                .iter()
                .map(|target| target.name())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    let mut out = String::from("\nbrowsers on this machine\n");
    let mut anything_to_do = false;
    for browser in &found {
        let entry = describe(browser, targets);
        anything_to_do |= entry.steps;
        out.push_str(&entry.text);
    }
    if anything_to_do {
        out.push_str(
            "\nWhere a step is listed it is yours to take: permission to read github.com is a\n\
             grant only the person at the keyboard can make. Everything before it is done.\n",
        );
    }
    out
}

/// One browser's entry in the report, and whether it left the reader anything to do.
struct Entry {
    /// The lines to print.
    text: String,
    /// Whether any of them ask the reader for something.
    steps: bool,
}

/// One browser's entry: what it is, and the steps that give it the extension.
fn describe(browser: &Browser, targets: &[Target]) -> Entry {
    let mut text = format!("  {} ({})\n", browser.name, label(browser.family));

    // Safari cannot load a directory, so it is pointed at the Chromium one to convert.
    let target = browser.family.target().unwrap_or(Target::Chromium);
    if !targets.contains(&target) {
        // Excluded by `--browser`, so nothing here is a step toward installing: the line
        // points at the full report rather than asking the reader for a grant, and counting
        // it as one would print the "yours to take" footer under a report that lists none.
        text.push_str("      not reported — run `dx browser`\n");
        return Entry { text, steps: false };
    }

    let steps = browser.family.steps();
    if steps.trim().is_empty() {
        text.push_str("      installed by dx — nothing for you to do\n");
        return Entry { text, steps: false };
    }
    for step in steps.lines() {
        text.push_str(&format!("      {step}\n"));
    }
    Entry { text, steps: true }
}

/// The word for a family in a report.
fn label(family: Family) -> &'static str {
    match family {
        Family::Chromium => "chromium",
        Family::Firefox => "firefox",
        Family::Safari => "safari",
    }
}

/// Open the extension directory in the system file manager, reporting what was run.
///
/// Only the folder: no browser will navigate to its own extensions page on request from
/// outside, so opening one would land on a blank tab and look broken.
fn reveal(dir: &Path) -> String {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    match std::process::Command::new(program).arg(dir).status() {
        Ok(status) if status.success() => format!("\nopened {}\n", dir.display()),
        Ok(_) | Err(_) => format!(
            "\ncould not open a file manager — the directory is {}\n",
            dir.display()
        ),
    }
}

/// How `dx doctor` reports the browser extension: one line per family.
#[must_use]
pub fn status_lines() -> Vec<String> {
    let mut lines = Vec::new();
    for target in Target::ALL {
        let where_it_is = extension::installed_dir(target).map_or_else(
            || "not in this install — `dx browser` says where to get it".to_string(),
            |dir| dir.display().to_string(),
        );
        lines.push(format!("  {:<9} {where_it_is}", target.name()));
    }
    let found = extension::detect();
    if found.is_empty() {
        lines.push(format!(
            "  {:<9} none found in the usual places",
            "browsers"
        ));
    }
    for browser in found {
        lines.push(format!(
            "  {:<9} {} — {}",
            label(browser.family),
            browser.name,
            given(&browser)
        ));
    }
    lines
}

/// Whether this browser has the extension yet, in one phrase.
///
/// Reported per browser rather than per family because the answer genuinely differs between
/// two browsers of the same family: one Firefox can carry the policy and another, installed
/// somewhere `dx` could not write, cannot.
fn given(browser: &Browser) -> String {
    match extension::channel(browser.family) {
        Channel::Policy { xpi } => {
            if policies::installed(&policies::policy_file(&browser.path), &xpi) {
                "installed by dx".to_string()
            } else {
                "not configured — run `dx setup`".to_string()
            }
        }
        Channel::Store { name, .. } => format!("install it from {name}"),
        Channel::Bundled => "enable it in Safari → Settings → Extensions".to_string(),
        Channel::Unpacked { .. } => {
            "load the directory above — `dx browser` has the steps".to_string()
        }
        Channel::Absent => "not in this install — `dx browser` says where to get it".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(tokens: &[&str]) -> Args {
        Args::parse(&tokens.iter().map(|t| (*t).to_string()).collect::<Vec<_>>())
    }

    fn scratch(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dx-browser-command-{label}"));
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    /// The repository's own extension source — what every shipped artifact is built from.
    fn repo_source() -> String {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../editor/github")
            .display()
            .to_string()
    }

    #[test]
    fn building_both_families_reports_where_each_landed() {
        let root = scratch("both");
        let path = root.to_str().expect("path");
        let report = run(&args(&["--from", &repo_source(), "--dir", path])).expect("browser");
        assert!(report.contains("chromium"));
        assert!(report.contains("firefox"));
        assert!(report.contains("(new)"));
        assert!(root.join("chromium/manifest.json").exists());
        assert!(root.join("firefox/wasm/doc_wasm_bg.wasm").exists());

        // Run again: nothing to do, and the report says so rather than implying work.
        let again = run(&args(&["--from", &repo_source(), "--dir", path])).expect("browser");
        assert!(again.contains("(unchanged)"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn one_browser_can_be_named_by_the_name_a_person_uses() {
        let root = scratch("one");
        let path = root.to_str().expect("path");
        let report = run(&args(&[
            "--from",
            &repo_source(),
            "--dir",
            path,
            "--browser",
            "brave",
        ]))
        .expect("browser");
        assert!(report.contains("chromium"));
        assert!(root.join("chromium/manifest.json").exists());
        assert!(!root.join("firefox").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_unknown_browser_is_answered_with_the_ones_that_work() {
        let error = run(&args(&["--browser", "netscape"])).expect_err("should fail");
        assert!(error.contains("netscape"));
        assert!(error.contains("chrome"));
        assert!(error.contains("firefox"));
    }

    /// A source that is not an extension is named as such, rather than producing a directory
    /// a browser would load and then fail on.
    #[test]
    fn a_source_that_is_not_an_extension_is_refused_by_name() {
        let root = scratch("bad-source");
        std::fs::create_dir_all(&root).expect("scratch");
        let error = run(&args(&["--from", root.to_str().expect("path")])).expect_err("no source");
        assert!(error.contains("resolve.js"), "{error}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_report_says_who_has_to_do_the_last_step() {
        // Whether any browser is found depends on the machine, so the assertion is about the
        // shape of the answer: either how to load a directory that is here, or where to get
        // an extension this install does not carry.
        let root = scratch("steps");
        let path = root.to_str().expect("path");
        let report = run(&args(&["--from", &repo_source(), "--dir", path])).expect("browser");
        assert!(
            report.contains("Load unpacked")
                || report.contains("Load Temporary Add-on")
                || report.contains("no browser extension")
                || report.contains("No browser found"),
            "the report never says what to do next:\n{report}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn doctor_lines_cover_every_family() {
        let lines = status_lines();
        assert!(lines.iter().any(|line| line.contains("chromium")));
        assert!(lines.iter().any(|line| line.contains("firefox")));
    }
}
