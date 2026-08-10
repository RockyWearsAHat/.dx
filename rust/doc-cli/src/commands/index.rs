//! `dx index` — scaffold a project index document, and the harness it stands on.
//!
//! Writes `index.dx` at the mapped root: a *precursor* index built from the file tree and
//! a cheap static survey — one section per top-level area holding the area's contents
//! ranked by how load-bearing each file looks (entry points, fan-in, size), a lead seeded
//! from the project's own README, and the method skeleton (Now, Findings, recipes,
//! verification). The scaffold is still deliberately shallow: its job is to be read whole
//! and improved by whoever ran it — replace each TODO with what the area actually does,
//! add `::code src=` blocks for the load-bearing files — so every later reader orients
//! for the price of one read.
//!
//! When the tree names its own build system (`Cargo.toml`, `package.json`, a pytest
//! project, `go.mod`, a `Makefile` with a `test:` target), the same command scaffolds
//! `dev.dx`: the verify harness, with gates written for the sandbox every block runs in —
//! fresh HOME, no network — so the first `dx run dev.dx --approve` is a review, not a
//! debugging session. Gates are never approved by scaffolding; the approval gate stands.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::args::Args;
use crate::workspace;

/// File name the scaffold is written to, at the mapped root.
pub const INDEX_FILE: &str = "index.dx";

/// File name the verify harness is written to, at the mapped root.
pub const HARNESS_FILE: &str = "dev.dx";

/// Directories the map never descends into: version control, stores, caches, build
/// output, and test fixtures — none of them orient a reader.
const SKIPPED_DIRECTORIES: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    "out",
    "vendor",
    "venv",
    "__pycache__",
    "fixture",
    "fixtures",
];

/// Most immediate entries one area lists; the rest are counted rather than named.
const AREA_ENTRY_CAP: usize = 20;

/// Past this many files the survey stops reading content: the map still lists everything,
/// it just stops ranking — an unbounded read of a monorepo is not orientation.
const SURVEY_CAP: usize = 600;

/// Largest file the survey reads; anything bigger is listed without facts.
const SURVEY_READ_CAP: u64 = 512 * 1024;

/// Extensions the survey counts lines for and searches for references.
const CODE_EXTENSIONS: &[&str] = &[
    "rs", "js", "mjs", "cjs", "ts", "tsx", "jsx", "py", "go", "rb", "java", "kt", "swift", "c",
    "h", "cpp", "hpp", "cc", "cs", "php", "sh", "bash", "lua", "sql",
];

/// File names that are an area's front door regardless of what references them.
const ENTRY_POINTS: &[&str] = &[
    "main.rs",
    "lib.rs",
    "mod.rs",
    "main.go",
    "main.py",
    "__init__.py",
    "index.js",
    "index.mjs",
    "index.ts",
    "index.tsx",
    "app.js",
    "app.ts",
    "app.py",
];

/// Stems too generic to mean "this file is referenced" when they appear in other files.
const GENERIC_STEMS: &[&str] = &[
    "main", "lib", "mod", "index", "test", "tests", "util", "utils", "types", "common", "app",
    "core",
];

/// What [`write_scaffold`] produced.
#[derive(Debug)]
pub struct Scaffold {
    /// Where the index document was written.
    pub path: PathBuf,
    /// Top-level areas mapped (root files count as one).
    pub areas: usize,
    /// Files counted across the whole tree.
    pub files: usize,
    /// The verify harness, when a build system was detected and `dev.dx` was written —
    /// the path and how many gates it carries. `None` when no build system was found or
    /// a `dev.dx` already exists (an existing harness is presumed improved and kept).
    pub harness: Option<(PathBuf, usize)>,
}

/// `dx index [dir] [--force]`.
pub fn run(args: &Args) -> Result<String, String> {
    let root = PathBuf::from(args.positional(0).unwrap_or("."));
    let scaffold = write_scaffold(&root, args.present("force"))?;
    let mut message = format!(
        "Wrote {} — {} area(s), {} file(s), ranked from the tree alone.",
        scaffold.path.display(),
        scaffold.areas,
        scaffold.files
    );
    if let Some((path, gates)) = &scaffold.harness {
        message.push_str(&format!(
            " Wrote {} — {} for the detected build system: review them, then \
             `dx run dev.dx --approve` records the first green.",
            path.display(),
            counted(*gates, "gate", "gates"),
        ));
    }
    message.push_str(
        " Read the index whole and improve it before other work: replace each TODO with \
         what the area does, and add ::code src= blocks for the load-bearing files.\n",
    );
    Ok(message)
}

/// Write the scaffold index (and, when the tree names a build system, the `dev.dx`
/// harness) for `root`, refusing to overwrite an existing index unless `force`.
///
/// The refusal matters: an existing `index.dx` is presumed *improved*, and a scaffold
/// silently replacing it would trade a real map for a file listing. An existing `dev.dx`
/// is kept on the same grounds, even under `force` — `force` re-scaffolds the map, never
/// a harness someone reviewed and approved.
pub fn write_scaffold(root: &Path, force: bool) -> Result<Scaffold, String> {
    if !root.is_dir() {
        return Err(format!("{} is not a directory", root.display()));
    }
    let path = root.join(INDEX_FILE);
    if path.exists() && !force {
        return Err(format!(
            "{} already exists — improve it instead, or pass --force to rewrite the \
             scaffold over it.",
            path.display()
        ));
    }

    let build = detect_build(root);
    let harness_path = root.join(HARNESS_FILE);
    let harness = match &build {
        Some(build) if !harness_path.exists() => {
            let (source, gates) = harness_source(root, build);
            workspace::save(&harness_path, &doc_core::format::parse(&source))?;
            Some((harness_path, gates))
        }
        _ => None,
    };

    let harness_present = harness.is_some() || root.join(HARNESS_FILE).exists();
    let (source, areas, files) = scaffold_source(root, build.as_ref(), harness_present);
    let document = doc_core::format::parse(&source);
    workspace::save(&path, &document)?;
    Ok(Scaffold {
        path,
        areas,
        files,
        harness,
    })
}

/// The scaffold's DOCSRC, plus how many areas and files it mapped.
fn scaffold_source(root: &Path, build: Option<&Build>, harness: bool) -> (String, usize, usize) {
    let name = root
        .canonicalize()
        .ok()
        .and_then(|real| real.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "project".to_string());

    let mut directories = Vec::new();
    let mut loose_files = Vec::new();
    for entry in listed(root) {
        if entry.is_dir() {
            directories.push(entry);
        } else if entry
            .file_name()
            .is_some_and(|n| n != INDEX_FILE && n != HARNESS_FILE)
        {
            loose_files.push(entry);
        }
    }

    let facts = survey(root);

    let mut body = format!("::heading level=1 id=index\n{name} — project index\n::end\n\n");
    if let Some(lead) = readme_lead(root) {
        body.push_str(&format!(
            "::paragraph id=index-lead\nFrom the project's own README: {lead}\n::end\n\n"
        ));
    }
    body.push_str(
        "::paragraph id=index-purpose\n\
         Scaffold from the file tree, written by `dx index`. Improve it: replace each \
         TODO with what the area does and how it connects to the rest, and add \
         `::code src=` blocks for the load-bearing files — they render as the file's \
         current text, never a stale copy. Files below are ranked by how load-bearing \
         they look (entry points, how many files reference them, size); trust the \
         ranking as a place to start, not as the truth. Keep this document true as the \
         code changes.\n\
         ::end\n\n\
         ::heading level=2 id=now\nNow\n::end\n\n\
         ::paragraph id=now-note\n\
         The working section — the designated first read of every turn, and its last \
         write. The worklist below is the task's program counter (found / fixing / \
         verified), which makes the task the unit of work instead of the session: it \
         survives compaction, crashes, and handoffs, and a second agent can pick up an \
         unclaimed line. Thoughts land here the moment they form (`dx_append`, \
         `dx_check`) and are promoted into the sections below when they harden. Keep it \
         small — everything else in this document is the archive.\n\
         ::end\n\n\
         ::checklist id=now-worklist\n\
         [ ] Improve this index: replace each TODO with what the area does\n\
         ::end\n\n\
         ::heading level=2 id=findings\nFindings\n::end\n\n\
         ::paragraph id=findings-notes\n\
         A ledger of open defects and sharp edges. A claim that can be checked \
         mechanically is written as a `::code run` block that checks it, declaring the \
         files it judges with `reads=` — then the recorded verdict goes stale the moment \
         the described code changes, and every read re-runs it, so this ledger cannot \
         lie the way a hand-kept list does. A check that must build or test the project \
         grants its build directory with `writes=` (for example \
         `writes=target`). Prose bullets are only for what no check can hold: what \
         breaks, where, what was tried. Delete findings when fixed.\n\
         ::end\n\n\
         ::heading level=2 id=recipes\nHow a change flows\n::end\n\n\
         ::paragraph id=recipes-note\n\
         TODO: recipes — for each common kind of change (a new flag, a new module, a \
         fix), the ordered list of files it touches. Write the first recipe the moment \
         your first change lands; recipes are what make the second change cheap.\n\
         ::end\n",
    );

    if harness {
        let system = build.map(Build::name).unwrap_or("detected");
        body.push_str(&format!(
            "\n::heading level=2 id=verification\nVerification — what green means\n::end\n\n\
             ::paragraph id=verification-note\n\
             `dx run dev.dx` runs the gates — scaffolded from the {system} project — \
             inside the sandbox and records each verdict in the document; a verdict goes \
             stale exactly when the files its gate reads change. The recorded verdicts \
             are the proof of done. First run: review the gates, then \
             `dx run dev.dx --approve`.\n::end\n"
        ));
    }

    let mut total = loose_files.len();
    let mut areas = 0;
    let mut used_slugs = Vec::new();

    if !loose_files.is_empty() {
        areas += 1;
        body.push_str(&format!(
            "\n::heading level=2 id=area-root\n./ — {}\n::end\n\n\
             ::bulleted-list id=area-root-files\n{}::end\n",
            counted(loose_files.len(), "file", "files"),
            listing(&loose_files, root, &facts)
        ));
    }

    for directory in &directories {
        let files = file_count(directory);
        total += files;
        areas += 1;
        let label = directory
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let slug = unique_slug(&label, &mut used_slugs);
        let children = listed(directory);
        let entries = entry_points(&children);
        let doors = if entries.is_empty() {
            String::new()
        } else {
            format!(" Entry points seen: {}.", entries.join(", "))
        };
        body.push_str(&format!(
            "\n::heading level=2 id=area-{slug}\n{label}/ — {}\n::end\n\n\
             ::paragraph id=area-{slug}-notes\n\
             TODO: what {label}/ is for, its entry points, and how it connects to the \
             rest.{doors}\n::end\n\n\
             ::bulleted-list id=area-{slug}-files\n{}::end\n",
            counted(files, "file", "files"),
            listing(&children, root, &facts)
        ));
    }

    (body, areas, total)
}

/// `1 file`, `3 files` — the count with its noun, singular exactly at one.
fn counted(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// One `- ` line per entry, relative to `root`, capped at [`AREA_ENTRY_CAP`] with the
/// remainder counted. A directory line carries its recursive file count; a surveyed
/// file carries its facts — entry point, fan-in, size — and files sort most
/// load-bearing first (directories keep name order, ahead of files).
fn listing(entries: &[PathBuf], root: &Path, facts: &HashMap<PathBuf, FileFacts>) -> String {
    let mut directories: Vec<&PathBuf> = entries.iter().filter(|e| e.is_dir()).collect();
    directories.sort();
    let mut files: Vec<&PathBuf> = entries.iter().filter(|e| !e.is_dir()).collect();
    files.sort_by_key(|path| {
        let f = facts.get(*path);
        (
            std::cmp::Reverse(f.map(|f| f.fan_in).unwrap_or(0)),
            std::cmp::Reverse(f.map(|f| f.lines).unwrap_or(0)),
            (*path).clone(),
        )
    });

    let mut lines = String::new();
    for entry in directories.into_iter().chain(files).take(AREA_ENTRY_CAP) {
        let relative = entry.strip_prefix(root).unwrap_or(entry).display();
        if entry.is_dir() {
            lines.push_str(&format!(
                "- {relative}/ ({})\n",
                counted(file_count(entry), "file", "files")
            ));
        } else {
            let mut notes = Vec::new();
            if entry
                .file_name()
                .is_some_and(|n| ENTRY_POINTS.contains(&n.to_string_lossy().as_ref()))
            {
                notes.push("entry point".to_string());
            }
            if let Some(f) = facts.get(entry) {
                notes.push(counted(f.lines, "line", "lines"));
                if f.fan_in > 0 {
                    notes.push(format!(
                        "referenced by {}",
                        counted(f.fan_in, "file", "files")
                    ));
                }
            }
            if notes.is_empty() {
                lines.push_str(&format!("- {relative}\n"));
            } else {
                lines.push_str(&format!("- {relative} — {}\n", notes.join(", ")));
            }
        }
    }
    if entries.len() > AREA_ENTRY_CAP {
        lines.push_str(&format!(
            "- … and {} more entries\n",
            entries.len() - AREA_ENTRY_CAP
        ));
    }
    lines
}

/// The entry-point file names present among `entries`, in name order.
fn entry_points(entries: &[PathBuf]) -> Vec<String> {
    let mut found: Vec<String> = entries
        .iter()
        .filter(|e| !e.is_dir())
        .filter_map(|e| e.file_name().map(|n| n.to_string_lossy().into_owned()))
        .filter(|n| ENTRY_POINTS.contains(&n.as_str()))
        .collect();
    found.sort();
    found
}

// ---------------------------------------------------------------------------------------
// The survey: cheap, bounded, deterministic facts about each file, from one read of the
// tree. No parser, no language server — identifiers only. The ranking it produces is a
// place to start reading, and the scaffold says so.
// ---------------------------------------------------------------------------------------

/// What the survey learned about one file.
struct FileFacts {
    /// Line count.
    lines: usize,
    /// How many *other* surveyed files mention this file's stem as an identifier.
    fan_in: usize,
}

/// Survey every code file under `root` (bounded by [`SURVEY_CAP`] files and
/// [`SURVEY_READ_CAP`] bytes each): line counts, and fan-in by identifier reference.
fn survey(root: &Path) -> HashMap<PathBuf, FileFacts> {
    let mut files = Vec::new();
    collect_files(root, &mut files);
    if files.len() > SURVEY_CAP {
        return HashMap::new();
    }

    let readable: Vec<(PathBuf, String)> = files
        .into_iter()
        .filter(|path| {
            path.extension()
                .is_some_and(|e| CODE_EXTENSIONS.contains(&e.to_string_lossy().as_ref()))
        })
        .filter(|path| std::fs::metadata(path).is_ok_and(|meta| meta.len() <= SURVEY_READ_CAP))
        .filter_map(|path| std::fs::read_to_string(&path).ok().map(|text| (path, text)))
        .collect();

    let tokens: Vec<HashSet<String>> = readable.iter().map(|(_, text)| identifiers(text)).collect();

    let mut facts = HashMap::new();
    for (at, (path, text)) in readable.iter().enumerate() {
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let counts = stem.len() >= 3 && !GENERIC_STEMS.contains(&stem.as_str());
        let fan_in = if counts {
            tokens
                .iter()
                .enumerate()
                .filter(|(other, set)| *other != at && set.contains(&stem))
                .count()
        } else {
            0
        };
        facts.insert(
            path.clone(),
            FileFacts {
                lines: text.lines().count(),
                fan_in,
            },
        );
    }
    facts
}

/// Every file under `directory`, recursively, honouring the same skip rules as the map.
fn collect_files(directory: &Path, into: &mut Vec<PathBuf>) {
    for entry in listed(directory) {
        if entry.is_dir() {
            collect_files(&entry, into);
        } else {
            into.push(entry);
        }
    }
}

/// The lowercased identifier tokens of `text` (ASCII alphanumeric and `_` runs).
fn identifiers(text: &str) -> HashSet<String> {
    let mut set = HashSet::new();
    let mut current = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            current.push(c.to_ascii_lowercase());
        } else if !current.is_empty() {
            set.insert(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        set.insert(current);
    }
    set
}

/// The first prose paragraph of the project's README, badges and headings skipped,
/// capped to a sentence-sized lead. `None` when there is no README or no prose.
fn readme_lead(root: &Path) -> Option<String> {
    let text = ["README.md", "README", "README.rst", "README.txt"]
        .iter()
        .find_map(|name| std::fs::read_to_string(root.join(name)).ok())?;
    let mut lead = String::new();
    for line in text.lines() {
        let line = line.trim();
        let noise = line.starts_with('#')
            || line.starts_with('<')
            || line.starts_with('[')
            || line.starts_with("![")
            || line.starts_with("---");
        if line.is_empty() || noise {
            if lead.is_empty() {
                continue;
            }
            break;
        }
        if !lead.is_empty() {
            lead.push(' ');
        }
        lead.push_str(line);
    }
    if lead.is_empty() {
        return None;
    }
    if lead.chars().count() > 400 {
        lead = lead.chars().take(400).collect::<String>() + "…";
    }
    Some(lead)
}

// ---------------------------------------------------------------------------------------
// The harness: dev.dx, written for the sandbox the gates will actually run in. Every
// block runs with a fresh HOME and no network, so a naive `cargo test` dies reaching
// crates.io — the gates below carry the working pattern instead of leaving the first
// author to rediscover it by failing.
// ---------------------------------------------------------------------------------------

/// A build system the tree names, and where its manifest lives relative to the root
/// (empty for the root itself — the common case; `rust` for a monorepo's crate folder).
#[derive(Debug, PartialEq, Eq)]
enum Build {
    Cargo(String),
    Node,
    Python,
    Go,
    Make,
}

impl Build {
    /// The system's name, for the sentences the scaffold writes.
    fn name(&self) -> &'static str {
        match self {
            Build::Cargo(_) => "cargo",
            Build::Node => "npm",
            Build::Python => "pytest",
            Build::Go => "go",
            Build::Make => "make",
        }
    }
}

/// The build system the tree names, if any. Cargo is looked for at the root and one
/// directory down (the `rust/` monorepo layout); everything else at the root only.
fn detect_build(root: &Path) -> Option<Build> {
    if root.join("Cargo.toml").exists() {
        return Some(Build::Cargo(String::new()));
    }
    if let Some(dir) = listed(root)
        .into_iter()
        .filter(|e| e.is_dir())
        .find(|dir| dir.join("Cargo.toml").exists())
    {
        return Some(Build::Cargo(
            dir.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        ));
    }
    if let Ok(package) = std::fs::read_to_string(root.join("package.json")) {
        if package.contains("\"test\"") && !package.contains("no test specified") {
            return Some(Build::Node);
        }
    }
    if root.join("pyproject.toml").exists()
        || root.join("pytest.ini").exists()
        || root.join("setup.py").exists()
        || holds_pytest_files(&root.join("tests"))
        || holds_pytest_files(&root.join("test"))
    {
        return Some(Build::Python);
    }
    if root.join("go.mod").exists() {
        return Some(Build::Go);
    }
    if std::fs::read_to_string(root.join("Makefile"))
        .is_ok_and(|text| text.lines().any(|l| l.starts_with("test:")))
    {
        return Some(Build::Make);
    }
    None
}

/// Whether `dir` holds files pytest would collect.
///
/// A manifest is the tidy signal and it is not the common one: plenty of real projects are a
/// `src/` and a `tests/` with no `pyproject.toml` anywhere, and a scaffold that skips the
/// harness for those hands a new project a map with nothing to prove it. The convention this
/// reads — `test_*.py` or `*_test.py` under `tests/` — is pytest's own default discovery rule.
fn holds_pytest_files(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        entry.file_name().to_str().is_some_and(|name| {
            name.ends_with(".py") && (name.starts_with("test_") || name.ends_with("_test.py"))
        })
    })
}

/// The lines every cargo gate starts with: find the rustup toolchain on PATH and point
/// cargo at the real toolchain home, which the sandbox exposes read-only — the sandbox's
/// own HOME is empty, and without this cargo reaches for the network and dies.
const CARGO_PRELUDE: &str = r#"CARGO_BIN="$(type -ap cargo 2>/dev/null | grep "/\.cargo/bin/cargo" | head -1)"
[ -n "$CARGO_BIN" ] || { echo "rustup cargo not found on PATH"; exit 1; }
export CARGO_HOME="${CARGO_BIN%/bin/cargo}"
export RUSTUP_HOME="${CARGO_HOME%/.cargo}/.rustup"
export PATH="${CARGO_BIN%/cargo}:$PATH"
"#;

/// The harness DOCSRC for `root` under `build`, plus how many gates it carries.
fn harness_source(root: &Path, build: &Build) -> (String, usize) {
    let name = root
        .canonicalize()
        .ok()
        .and_then(|real| real.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "project".to_string());

    let mut body = format!(
        "::heading level=1 id=dev\n{name} — development harness\n::end\n\n\
         ::paragraph id=dev-intro\n\
         `dx run dev.dx` is the fast loop: every gate below runs inside the sandbox \
         (fresh HOME, no network, the repository read-only except its stated `writes=` \
         grants) and records its verdict in this document. A verdict goes stale exactly \
         when the files its gate reads change, so a green document is a current proof, \
         not a memory. Scaffolded by `dx index` from the detected {} project — review \
         each gate, then `dx run dev.dx --approve` records the approval and runs them. \
         Adjust `reads=`/`writes=` as the project grows.\n::end\n\n\
         ::heading level=2 id=gates\nGates\n::end\n\n",
        build.name()
    );

    let gates = match build {
        Build::Cargo(dir) => cargo_gates(root, dir, &mut body),
        Build::Node => {
            let reads = existing(root, "", &["package.json", "src", "lib", "test", "tests"]);
            body.push_str(&gate(
                "gate-test",
                &reads,
                "node_modules",
                "[ -d node_modules ] || { echo \"node_modules missing — install dependencies before running (the sandbox has no network)\"; exit 1; }\n\
                 log=\"$DX_SANDBOX/test.log\"\n\
                 if npm test --silent >\"$log\" 2>&1; then tail -5 \"$log\"; else echo FAILED; tail -20 \"$log\"; exit 1; fi\n",
            ));
            1
        }
        Build::Python => {
            let reads = existing(
                root,
                "",
                &["pyproject.toml", "pytest.ini", "setup.py", "src", "tests"],
            );
            body.push_str(&gate(
                "gate-test",
                &reads,
                ".pytest_cache",
                "log=\"$DX_SANDBOX/test.log\"\n\
                 if python3 -m pytest -q >\"$log\" 2>&1; then tail -3 \"$log\"; else echo FAILED; tail -20 \"$log\"; exit 1; fi\n",
            ));
            1
        }
        Build::Go => {
            let reads = existing(root, "", &["go.mod", "go.sum", "cmd", "internal", "pkg"]);
            body.push_str(&gate(
                "gate-test",
                &reads,
                "",
                "export GOCACHE=\"$HOME/gocache\"\n\
                 [ -d vendor ] && export GOFLAGS=-mod=vendor\n\
                 log=\"$DX_SANDBOX/test.log\"\n\
                 if go test ./... >\"$log\" 2>&1; then tail -5 \"$log\"; else echo FAILED; tail -20 \"$log\"; exit 1; fi\n",
            ));
            1
        }
        Build::Make => {
            body.push_str(&gate(
                "gate-test",
                &["Makefile".to_string()],
                "",
                "log=\"$DX_SANDBOX/test.log\"\n\
                 if make test >\"$log\" 2>&1; then tail -5 \"$log\"; else echo FAILED; tail -20 \"$log\"; exit 1; fi\n",
            ));
            1
        }
    };

    (body, gates)
}

/// The three cargo gates — test, clippy, fmt — written into `body`; returns the count.
/// `dir` is the crate folder relative to the root, empty when the root is the crate.
fn cargo_gates(root: &Path, dir: &str, body: &mut String) -> usize {
    let enter = if dir.is_empty() {
        String::new()
    } else {
        format!("cd {dir}\n")
    };
    let reads = existing(
        root,
        dir,
        &["src", "tests", "benches", "Cargo.toml", "Cargo.lock"],
    );
    let target = if dir.is_empty() {
        "target".to_string()
    } else {
        format!("{dir}/target")
    };

    body.push_str(&gate(
        "gate-test",
        &reads,
        &target,
        &format!(
            "{CARGO_PRELUDE}{enter}log=\"$DX_SANDBOX/test.log\"\n\
             if cargo test --offline --locked --quiet >\"$log\" 2>&1; then\n  \
             grep \"^test result\" \"$log\" | awk -F'[ ;]+' '{{p+=$4}} END {{print \"ok - \" p \" tests passed\"}}'\n\
             else\n  echo FAILED; tail -8 \"$log\"; exit 1\nfi\n"
        ),
    ));
    body.push_str(&gate(
        "gate-clippy",
        &reads,
        &target,
        &format!(
            "{CARGO_PRELUDE}{enter}log=\"$DX_SANDBOX/clippy.log\"\n\
             if cargo clippy --all-targets --offline --locked --quiet -- -D warnings >\"$log\" 2>&1; then\n  \
             echo \"ok - clippy clean\"\n\
             else\n  echo FAILED; tail -8 \"$log\"; exit 1\nfi\n"
        ),
    ));
    let mut fmt_reads = reads.clone();
    let fmt_config = if dir.is_empty() {
        "rustfmt.toml".to_string()
    } else {
        format!("{dir}/rustfmt.toml")
    };
    if root.join(&fmt_config).exists() {
        fmt_reads.push(fmt_config);
    }
    body.push_str(&gate(
        "gate-fmt",
        &fmt_reads,
        &target,
        &format!("{CARGO_PRELUDE}{enter}cargo fmt --check && echo \"ok - fmt clean\"\n"),
    ));
    3
}

/// One `::code … run` gate block. Empty `reads`/`writes` serialize to nothing.
fn gate(id: &str, reads: &[String], writes: &str, code: &str) -> String {
    let mut header = format!("::code id={id} lang=bash run");
    if !reads.is_empty() {
        header.push_str(&format!(" reads={}", reads.join(",")));
    }
    if !writes.is_empty() {
        header.push_str(&format!(" writes={writes}"));
    }
    header.push_str(" timeout=900");
    format!("{header}\n{code}::end\n\n")
}

/// The `candidates` that exist under `root` (each prefixed with `dir/` when `dir` is
/// non-empty), as the relative paths a `reads=` declaration names.
fn existing(root: &Path, dir: &str, candidates: &[&str]) -> Vec<String> {
    candidates
        .iter()
        .map(|c| {
            if dir.is_empty() {
                (*c).to_string()
            } else {
                format!("{dir}/{c}")
            }
        })
        .filter(|relative| root.join(relative).exists())
        .collect()
}

/// The entries of `directory` the map shows: sorted by name, hidden and skipped
/// directories left out.
fn listed(directory: &Path) -> Vec<PathBuf> {
    let Ok(reader) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut entries: Vec<PathBuf> = reader
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name().is_some_and(|name| {
                let name = name.to_string_lossy();
                let skipped = path.is_dir() && SKIPPED_DIRECTORIES.contains(&name.as_ref());
                !name.starts_with('.') && !skipped
            })
        })
        .collect();
    entries.sort();
    entries
}

/// Recursive file count under `directory`, honouring the same skip rules as [`listed`].
fn file_count(directory: &Path) -> usize {
    listed(directory)
        .iter()
        .map(|entry| if entry.is_dir() { file_count(entry) } else { 1 })
        .sum()
}

/// A block-id-safe slug of `label`, made unique against `used` by numbering repeats.
fn unique_slug(label: &str, used: &mut Vec<String>) -> String {
    let base: String = label
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let base = base.trim_matches('-').to_string();
    let base = if base.is_empty() {
        "area".to_string()
    } else {
        base
    };

    let mut slug = base.clone();
    let mut counter = 2;
    while used.contains(&slug) {
        slug = format!("{base}-{counter}");
        counter += 1;
    }
    used.push(slug.clone());
    slug
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dx-index-tests-{label}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src/deep")).expect("dirs");
        std::fs::write(root.join("README.md"), "hello").expect("file");
        std::fs::write(root.join("src/main.rs"), "fn main() {}").expect("file");
        std::fs::write(root.join("src/deep/util.rs"), "// util").expect("file");
        std::fs::create_dir_all(root.join("node_modules/junk")).expect("dirs");
        std::fs::write(root.join("node_modules/junk/x.js"), "junk").expect("file");
        root
    }

    #[test]
    fn the_scaffold_maps_areas_and_skips_build_junk() {
        let root = project("map");
        let scaffold = write_scaffold(&root, false).expect("scaffold");
        assert_eq!(scaffold.areas, 2); // ./ and src/
        assert_eq!(scaffold.files, 3);

        let text = workspace::read(&scaffold.path).expect("read");
        assert!(text.contains("src/ — 2 files"), "{text}");
        assert!(text.contains("- src/deep/ (1 file)"), "{text}");
        assert!(text.contains("- README.md"), "{text}");
        assert!(text.contains("TODO: what src/ is for"), "{text}");
        assert!(!text.contains("node_modules"), "{text}");
        // The index never lists itself as a loose file.
        assert!(!text.contains("- index.dx"), "{text}");
        // The working section is scaffolded in: the `now` worklist is the designated
        // first read of a turn and its last write — the program counter that makes the
        // task, not the session, the unit of work.
        assert!(text.contains("::heading level=2 id=now"), "{text}");
        assert!(text.contains("::checklist id=now-worklist"), "{text}");
        assert!(text.contains("program counter"), "{text}");
        // The method skeleton: recipes are scaffolded as a section to fill.
        assert!(text.contains("How a change flows"), "{text}");
    }

    /// A `src/` and a `tests/` with no manifest anywhere is an ordinary Python project, and
    /// a scaffold that skips the harness there hands a new project a map with nothing to
    /// prove it. pytest's own discovery rule is the signal.
    #[test]
    fn a_tests_directory_alone_is_enough_to_scaffold_the_harness() {
        let root = project("pytest-by-convention");
        std::fs::create_dir_all(root.join("tests")).expect("dirs");
        std::fs::write(root.join("tests/test_money.py"), "def test_it(): pass\n").expect("file");

        assert_eq!(detect_build(&root), Some(Build::Python));

        let scaffold = write_scaffold(&root, false).expect("scaffold");
        let (harness_path, gates) = scaffold.harness.expect("a detected project gets a harness");
        assert_eq!(gates, 1);
        assert!(workspace::read(&harness_path)
            .expect("read")
            .contains("pytest"));
    }

    /// The convention is a rule, not a guess: a directory of ordinary modules is not a
    /// test suite, and claiming a harness for it would scaffold a gate that cannot pass.
    #[test]
    fn a_directory_of_plain_modules_is_not_a_test_suite() {
        let root = project("no-pytest");
        std::fs::create_dir_all(root.join("tests")).expect("dirs");
        std::fs::write(root.join("tests/helpers.py"), "VALUE = 1\n").expect("file");

        assert_eq!(detect_build(&root), None);
    }

    #[test]
    fn the_lead_comes_from_the_readme_and_files_carry_facts() {
        let root = project("facts");
        std::fs::write(
            root.join("README.md"),
            "# Title\n\n[![badge](x)](y)\n\nA tool that does the thing.\nAcross two lines.\n\nMore later.\n",
        )
        .expect("file");
        // walk.rs is referenced by main.rs, so it outranks quiet.rs and says why.
        std::fs::write(
            root.join("src/main.rs"),
            "mod walk;\nfn main() { walk::go() }\n",
        )
        .expect("file");
        std::fs::write(root.join("src/walk.rs"), "pub fn go() {}\n").expect("file");
        std::fs::write(root.join("src/quiet.rs"), "// nothing refers to this\n").expect("file");

        let scaffold = write_scaffold(&root, false).expect("scaffold");
        let text = workspace::read(&scaffold.path).expect("read");
        assert!(
            text.contains(
                "From the project's own README: A tool that does the thing. Across two lines."
            ),
            "{text}"
        );
        assert!(text.contains("- src/main.rs — entry point"), "{text}");
        assert!(
            text.contains("- src/walk.rs — 1 line, referenced by 1 file"),
            "{text}"
        );
        let walk = text.find("- src/walk.rs").expect("ranked");
        let quiet = text.find("- src/quiet.rs").expect("listed");
        assert!(walk < quiet, "referenced file should rank first: {text}");
        assert!(text.contains("Entry points seen: main.rs."), "{text}");
    }

    #[test]
    fn a_cargo_project_gets_a_reviewable_harness() {
        let root = project("cargo");
        std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"x\"\n").expect("file");
        std::fs::write(root.join("Cargo.lock"), "").expect("file");

        let scaffold = write_scaffold(&root, false).expect("scaffold");
        let (harness, gates) = scaffold.harness.expect("harness written");
        assert_eq!(gates, 3);

        let text = workspace::read(&harness).expect("read");
        // The sandbox pattern is in the gate, not left for the author to rediscover:
        // rustup toolchain resolved from PATH, cargo held offline.
        assert!(text.contains("CARGO_HOME"), "{text}");
        assert!(text.contains("--offline --locked"), "{text}");
        assert!(text.contains("writes=target"), "{text}");
        assert!(text.contains("reads=src,Cargo.toml,Cargo.lock"), "{text}");
        assert!(text.contains("id=gate-clippy"), "{text}");
        assert!(text.contains("id=gate-fmt"), "{text}");
        // The index tells the reader what green means and how to earn it.
        let index = workspace::read(&scaffold.path).expect("read");
        assert!(index.contains("dx run dev.dx --approve"), "{index}");
        // The harness never lists itself or the index as project files.
        assert!(!index.contains("- dev.dx"), "{index}");
    }

    #[test]
    fn a_monorepo_crate_folder_is_found_one_level_down() {
        let root = project("monorepo");
        std::fs::create_dir_all(root.join("rust/src")).expect("dirs");
        std::fs::write(root.join("rust/Cargo.toml"), "[package]\n").expect("file");
        std::fs::write(root.join("rust/src/lib.rs"), "").expect("file");

        let scaffold = write_scaffold(&root, false).expect("scaffold");
        let (harness, _) = scaffold.harness.expect("harness written");
        let text = workspace::read(&harness).expect("read");
        assert!(text.contains("cd rust"), "{text}");
        assert!(text.contains("writes=rust/target"), "{text}");
        assert!(text.contains("reads=rust/src,rust/Cargo.toml"), "{text}");
    }

    #[test]
    fn a_plain_tree_gets_no_harness_and_an_existing_one_is_kept() {
        let root = project("plain");
        let scaffold = write_scaffold(&root, false).expect("scaffold");
        assert!(scaffold.harness.is_none());
        assert!(!root.join(HARNESS_FILE).exists());

        // A reviewed harness is never rewritten, even by --force.
        std::fs::write(root.join("Cargo.toml"), "[package]\n").expect("file");
        std::fs::write(root.join(HARNESS_FILE), "improved by hand").expect("file");
        let again = write_scaffold(&root, true).expect("forced rewrite");
        assert!(again.harness.is_none());
        let kept = std::fs::read_to_string(root.join(HARNESS_FILE)).expect("read");
        assert_eq!(kept, "improved by hand");
    }

    #[test]
    fn an_npm_project_with_real_tests_gets_a_gate() {
        let root = project("npm");
        std::fs::write(
            root.join("package.json"),
            "{\"scripts\": {\"test\": \"node --test\"}}",
        )
        .expect("file");
        let scaffold = write_scaffold(&root, false).expect("scaffold");
        let (harness, gates) = scaffold.harness.expect("harness written");
        assert_eq!(gates, 1);
        let text = workspace::read(&harness).expect("read");
        assert!(text.contains("npm test"), "{text}");
        assert!(text.contains("node_modules missing"), "{text}");

        // The npm placeholder script is not a test suite.
        let bare = project("npm-bare");
        std::fs::write(
            bare.join("package.json"),
            "{\"scripts\": {\"test\": \"echo \\\"Error: no test specified\\\" && exit 1\"}}",
        )
        .expect("file");
        assert!(write_scaffold(&bare, false)
            .expect("scaffold")
            .harness
            .is_none());
    }

    #[test]
    fn an_existing_index_is_kept_unless_forced() {
        let root = project("keep");
        write_scaffold(&root, false).expect("first");
        let refused = write_scaffold(&root, false).expect_err("should refuse");
        assert!(refused.contains("--force"), "{refused}");
        write_scaffold(&root, true).expect("forced rewrite");
    }

    #[test]
    fn slugs_are_id_safe_and_unique() {
        let mut used = Vec::new();
        assert_eq!(unique_slug("My Dir!", &mut used), "my-dir");
        assert_eq!(unique_slug("My Dir!", &mut used), "my-dir-2");
        assert_eq!(unique_slug("---", &mut used), "area");
    }

    #[test]
    fn a_missing_directory_is_a_clear_error() {
        let error = write_scaffold(Path::new("/dx/nowhere"), false).expect_err("should fail");
        assert!(error.contains("not a directory"));
    }
}
