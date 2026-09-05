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
//! When the tree names its own build system — `Cargo.toml`, `package.json`'s own test
//! script, a pytest project, `go.mod`, a Ruby/Maven/.NET project, or a `Makefile` naming
//! a test target — the same command scaffolds `dev.dx`: the verify harness, with gates
//! written for the sandbox every block runs in — fresh HOME, no network — so the first
//! `dx run dev.dx --approve` is a review, not a debugging session. Gates are never
//! approved by scaffolding; the approval gate stands.

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
pub const SKIPPED_DIRECTORIES: &[&str] = &[
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

/// Extensions the survey counts lines for and searches for references — also the set
/// `dx trace` reads (`commands::trace`), so the two stay looking at the same tree.
pub(crate) const CODE_EXTENSIONS: &[&str] = &[
    "rs", "js", "mjs", "cjs", "ts", "tsx", "jsx", "py", "go", "rb", "java", "kt", "swift", "c",
    "h", "cpp", "hpp", "cc", "cs", "php", "sh", "bash", "lua", "sql", "mm", "metal", "S",
];

/// File names that are an area's front door regardless of what references them. Kept in
/// step with every name [`detect_entry`]'s conventions can seed — a filename the entry
/// detector treats as a real entry file earns the same "— entry point" label the cheap,
/// convention-agnostic scan already gives `main.rs` and its siblings; letting the two
/// drift apart is what report `d288e338` caught (`main.tsx`/`App.tsx` seeded by the
/// entry detector but never labeled by this list).
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
    "index.jsx",
    "app.js",
    "app.ts",
    "app.py",
    "main.ts",
    "main.tsx",
    "main.jsx",
    "App.tsx",
    "App.jsx",
    "manage.py",
    "wsgi.py",
    "asgi.py",
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

    body.push_str(&format!(
        "\n::heading level=2 id=trace\nTraced index\n::end\n\n\
         ::paragraph id=trace-note\n\
         `dx trace --brief` maps the tree into a real symbol + reference index — named \
         functions, structs, classes, and the files that reference each one — ranked by \
         fan-in, not the file-stem heuristic the areas below use. This run block \
         re-derives it whenever a file under its `reads=` tree changes; review it, then \
         approve like any other gate (`dx run {INDEX_FILE} --approve`).\n::end\n\n{}",
        trace_gate(&trace_reads(&directories, &loose_files))
    ));

    let mut total = loose_files.len();
    let mut areas = 0;
    let mut used_slugs = Vec::new();
    let entry = detect_entry(root);

    if !loose_files.is_empty() || entry.is_some() {
        areas += 1;
        body.push_str(&format!(
            "\n::heading level=2 id=area-root\n./ — {}\n::end\n\n",
            counted(loose_files.len(), "file", "files")
        ));
        if let Some(matched) = &entry {
            body.push_str(&format!(
                "::paragraph id=area-root-entry-note\n\
                 Entry point detected ({}) — mirrored live below, always the file's \
                 current text.\n::end\n\n",
                matched.note
            ));
            let multiple = matched.files.len() > 1;
            for (position, file) in matched.files.iter().enumerate() {
                let relative = file.strip_prefix(root).unwrap_or(file).display();
                let id = if multiple {
                    format!("area-root-entry-{}", position + 1)
                } else {
                    "area-root-entry".to_string()
                };
                body.push_str(&format!(
                    "::code id={id} src={relative} lang={}\n::end\n\n",
                    code_lang_for(file)
                ));
            }
        }
        if !loose_files.is_empty() {
            body.push_str(&format!(
                "::bulleted-list id=area-root-files\n{}::end\n",
                listing(&loose_files, root, &facts)
            ));
        }
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
/// Shared with `commands::trace`, which walks the identical tree — one walker, so a
/// directory the map skips is a directory the tracer skips too.
pub(crate) fn collect_files(directory: &Path, into: &mut Vec<PathBuf>) {
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
/// Cargo gets its own variant because it earns three gates (test, clippy, fmt) instead
/// of one; every other system is [`Build::Detected`] — a name plus the [`TestCommand`]
/// one of [`TEST_SYSTEMS`] discovered from the project's *own* declared test command, so
/// a new ecosystem is a detection function and a shell body, never a new variant.
#[derive(Debug, PartialEq, Eq)]
enum Build {
    Cargo(String),
    Detected(&'static str, TestCommand),
}

impl Build {
    /// The system's name, for the sentences the scaffold writes.
    fn name(&self) -> &'static str {
        match self {
            Build::Cargo(_) => "cargo",
            Build::Detected(name, _) => name,
        }
    }
}

/// One non-Cargo test system this scaffold can discover: a `detect` function that reads
/// the project's own manifest or convention for its *real* test command, rather than a
/// fixed command assumed for every project of that kind — generalizes report
/// `7f351075`'s "framework is a data row" fix to build systems.
struct TestSystem {
    /// Shown as [`Build::name`] and in the scaffold's harness sentence.
    name: &'static str,
    detect: fn(&Path) -> Option<TestCommand>,
}

/// What one gate needs to run a discovered test command: what it reads, what it writes,
/// and the shell body run after the `cd` into the project root.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TestCommand {
    reads: Vec<String>,
    writes: String,
    body: String,
}

/// Every non-Cargo system this scaffold recognizes, checked in this order — Make last,
/// since a Makefile can sit alongside any other manifest and its test-named target is
/// the most generic signal here.
const TEST_SYSTEMS: &[TestSystem] = &[
    TestSystem {
        name: "npm",
        detect: detect_node,
    },
    TestSystem {
        name: "pytest",
        detect: detect_python,
    },
    TestSystem {
        name: "go",
        detect: detect_go,
    },
    TestSystem {
        name: "rspec/rake",
        detect: detect_ruby,
    },
    TestSystem {
        name: "maven",
        detect: detect_maven,
    },
    TestSystem {
        name: "dotnet",
        detect: detect_dotnet,
    },
    TestSystem {
        name: "make",
        detect: detect_make,
    },
];

/// The build system the tree names, if any. Cargo is looked for at the root and one
/// directory down (the `rust/` monorepo layout) and always wins when present — richer
/// treatment (three gates) for a system this repository itself runs on. Every other
/// system in [`TEST_SYSTEMS`] is tried in table order and the first to detect its own
/// test command wins.
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
    TEST_SYSTEMS
        .iter()
        .find_map(|system| (system.detect)(root).map(|cmd| Build::Detected(system.name, cmd)))
}

/// The `test` script npm would run, from `package.json`'s own `scripts` — any key whose
/// name contains "test" (`test`, `test:unit`, `run-tests`), preferring the exact key
/// `test` when both exist, so a project that names its suite anything but the bare word
/// still gets a gate. A placeholder script (`npm init`'s "Error: no test specified") is
/// not a real test suite.
fn detect_node(root: &Path) -> Option<TestCommand> {
    let text = std::fs::read_to_string(root.join("package.json")).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let scripts = value.get("scripts")?.as_object()?;
    let mut named: Vec<&String> = scripts.keys().filter(|k| k.contains("test")).collect();
    named.sort();
    let key = if scripts.contains_key("test") {
        "test"
    } else {
        named.first()?.as_str()
    };
    let script = scripts.get(key)?.as_str()?;
    if script.contains("no test specified") {
        return None;
    }
    let run = if key == "test" {
        "npm test".to_string()
    } else {
        format!("npm run {key}")
    };
    Some(TestCommand {
        reads: existing(root, "", &["package.json", "src", "lib", "test", "tests"]),
        writes: "node_modules".to_string(),
        body: format!(
            "[ -d node_modules ] || {{ echo \"node_modules missing — install dependencies before running (the sandbox has no network)\"; exit 1; }}\n\
             log=\"$DX_SANDBOX/test.log\"\n\
             if {run} --silent >\"$log\" 2>&1; then tail -5 \"$log\"; else echo FAILED; tail -20 \"$log\"; exit 1; fi\n"
        ),
    })
}

/// A Python project's own pytest convention: a manifest naming it, or — the common case
/// with no manifest at all — `test_*.py`/`*_test.py` files under `tests/` or `test/`,
/// pytest's own default discovery rule.
fn detect_python(root: &Path) -> Option<TestCommand> {
    let present = root.join("pyproject.toml").exists()
        || root.join("pytest.ini").exists()
        || root.join("setup.py").exists()
        || holds_pytest_files(&root.join("tests"))
        || holds_pytest_files(&root.join("test"));
    if !present {
        return None;
    }
    Some(TestCommand {
        reads: existing(
            root,
            "",
            &["pyproject.toml", "pytest.ini", "setup.py", "src", "tests"],
        ),
        writes: ".pytest_cache".to_string(),
        body: "log=\"$DX_SANDBOX/test.log\"\n\
               if python3 -m pytest -q >\"$log\" 2>&1; then tail -3 \"$log\"; else echo FAILED; tail -20 \"$log\"; exit 1; fi\n"
            .to_string(),
    })
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

/// Go's own convention: a `go.mod` names the module, `go test ./...` finds everything
/// under it.
fn detect_go(root: &Path) -> Option<TestCommand> {
    if !root.join("go.mod").exists() {
        return None;
    }
    Some(TestCommand {
        reads: existing(root, "", &["go.mod", "go.sum", "cmd", "internal", "pkg"]),
        writes: String::new(),
        body: "export GOCACHE=\"$HOME/gocache\"\n\
               [ -d vendor ] && export GOFLAGS=-mod=vendor\n\
               log=\"$DX_SANDBOX/test.log\"\n\
               if go test ./... >\"$log\" 2>&1; then tail -5 \"$log\"; else echo FAILED; tail -20 \"$log\"; exit 1; fi\n"
            .to_string(),
    })
}

/// A Ruby project's own test command: `bundle exec rspec` when the project has an rspec
/// suite (a `.rspec` file, or `*_spec.rb` files under `spec/`), else `bundle exec rake
/// test` when a `Rakefile` names a test task — Ruby's two competing conventions, read
/// the way a person skimming the repo would tell them apart.
fn detect_ruby(root: &Path) -> Option<TestCommand> {
    if !root.join("Gemfile").exists() {
        return None;
    }
    let reads = existing(
        root,
        "",
        &["Gemfile", "Gemfile.lock", "lib", "spec", "test", "Rakefile"],
    );
    let uses_rspec = root.join(".rspec").exists() || holds_spec_files(&root.join("spec"));
    let body = if uses_rspec {
        "log=\"$DX_SANDBOX/test.log\"\n\
         if bundle exec rspec >\"$log\" 2>&1; then tail -5 \"$log\"; else echo FAILED; tail -20 \"$log\"; exit 1; fi\n"
    } else if std::fs::read_to_string(root.join("Rakefile")).is_ok_and(|t| t.contains("test")) {
        "log=\"$DX_SANDBOX/test.log\"\n\
         if bundle exec rake test >\"$log\" 2>&1; then tail -5 \"$log\"; else echo FAILED; tail -20 \"$log\"; exit 1; fi\n"
    } else {
        return None;
    };
    Some(TestCommand {
        reads,
        writes: String::new(),
        body: body.to_string(),
    })
}

/// Whether `dir` holds an rspec spec file.
fn holds_spec_files(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        entry
            .file_name()
            .to_str()
            .is_some_and(|n| n.ends_with("_spec.rb"))
    })
}

/// A Maven project's own convention: `pom.xml` at the root, `mvn test` offline.
fn detect_maven(root: &Path) -> Option<TestCommand> {
    if !root.join("pom.xml").exists() {
        return None;
    }
    Some(TestCommand {
        reads: existing(root, "", &["pom.xml", "src"]),
        writes: "target".to_string(),
        body: "log=\"$DX_SANDBOX/test.log\"\n\
               if mvn -o -q test >\"$log\" 2>&1; then tail -10 \"$log\"; else echo FAILED; tail -30 \"$log\"; exit 1; fi\n"
            .to_string(),
    })
}

/// A .NET project's own convention: a solution or project file at the root — `dotnet
/// test` resolves whichever one it finds without being told which. Assumes packages are
/// already restored, the same "no network at run time" contract every other gate holds.
fn detect_dotnet(root: &Path) -> Option<TestCommand> {
    let projects: Vec<String> = listed(root)
        .iter()
        .filter(|p| {
            p.extension()
                .is_some_and(|e| matches!(e.to_str(), Some("sln" | "csproj" | "fsproj")))
        })
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();
    if projects.is_empty() {
        return None;
    }
    let mut reads = projects;
    reads.extend(existing(root, "", &["src", "tests", "test"]));
    Some(TestCommand {
        reads,
        writes: String::new(),
        body: "log=\"$DX_SANDBOX/test.log\"\n\
               if dotnet test --no-restore >\"$log\" 2>&1; then tail -10 \"$log\"; else echo FAILED; tail -30 \"$log\"; exit 1; fi\n"
            .to_string(),
    })
}

/// The first Makefile target whose name contains "test" (`test:`, `unit-test:`,
/// `test-all:`) — Make has no fixed vocabulary for naming a test target, so matching
/// only the exact word `test` missed real suites by a name.
fn detect_make(root: &Path) -> Option<TestCommand> {
    let text = std::fs::read_to_string(root.join("Makefile")).ok()?;
    let target = text.lines().find_map(|line| {
        let (name, _) = line.split_once(':')?;
        let name = name.trim();
        let valid = !name.is_empty()
            && !name.starts_with('.')
            && !line.starts_with([' ', '\t'])
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '/'));
        (valid && name.to_lowercase().contains("test")).then(|| name.to_string())
    })?;
    Some(TestCommand {
        reads: vec!["Makefile".to_string()],
        writes: String::new(),
        body: format!(
            "log=\"$DX_SANDBOX/test.log\"\n\
             if make {target} >\"$log\" 2>&1; then tail -5 \"$log\"; else echo FAILED; tail -20 \"$log\"; exit 1; fi\n"
        ),
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
        Build::Detected(_, command) => {
            body.push_str(&gate(
                "gate-test",
                &command.reads,
                &command.writes,
                &command.body,
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

// ---------------------------------------------------------------------------------------
// The traced index: a run gate wired to `dx trace --brief`, and — when the tree
// unambiguously names one — a live-mirrored React entry point.
// ---------------------------------------------------------------------------------------

/// The `reads=` set for the `index-trace` gate: the mapped top-level directories, or —
/// when the tree has none, a single-directory project with code sitting at the root —
/// the root-level files the survey itself would read. Either way this names real,
/// existing paths, the same contract [`existing`] enforces for the cargo/npm/etc gates.
fn trace_reads(directories: &[PathBuf], loose_files: &[PathBuf]) -> Vec<String> {
    let mut reads: Vec<String> = directories
        .iter()
        .filter_map(|d| d.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();
    if reads.is_empty() {
        reads = loose_files
            .iter()
            .filter(|f| {
                f.extension()
                    .is_some_and(|e| CODE_EXTENSIONS.contains(&e.to_string_lossy().as_ref()))
            })
            .filter_map(|f| f.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect();
    }
    reads
}

/// The `index-trace` gate: `dx trace --brief .`, scaffolded but never approved — same
/// rule the harness gates already follow, "gates are never approved by scaffolding".
fn trace_gate(reads: &[String]) -> String {
    gate("index-trace", reads, "", "dx trace --brief .\n")
}

/// A project's real entry point, found without knowing what framework produced it —
/// only the same file-tree and content signals a person skimming the project would use.
/// A new framework built on these same conventions (a Vite-shaped entry script, an
/// `app.py`-shaped WSGI app) is detected with no new code here — generalizes report
/// `7f351075`'s "framework is a data row" fix to "no framework list at all."
struct EntryMatch {
    /// What signal found it, for the scaffold's note — e.g. "index.html's script tag",
    /// "app/layout.tsx + app/page.tsx file-router convention", "`createRoot(` bootstrap
    /// call", "`Flask(` app construction".
    note: String,
    /// The entry file(s) this match seeded, in the order they should be mirrored.
    files: Vec<PathBuf>,
}

/// Generic bootstrap-call substrings that mark a JS/TS file as the thing that actually
/// mounts an app, across frameworks: React's three render entry points, Vue/Svelte's
/// `createApp`/`new App`, Angular's `bootstrapApplication`, and any server's `.listen(`.
/// None of these name a framework — each is the real call a bootstrap file makes, which
/// is what tells it apart from a component or route handler that merely sits nearby.
const JS_BOOTSTRAP_MARKERS: &[&str] = &[
    "createRoot(",
    "ReactDOM.render(",
    "hydrateRoot(",
    "createApp(",
    "new App(",
    "bootstrapApplication(",
    ".listen(",
];

/// Generic Python web-app construction calls — the object every WSGI/ASGI framework's
/// entry file instantiates, regardless of which framework: Flask, FastAPI, Bottle,
/// Falcon, Sanic, Starlette, aiohttp.
const PY_BOOTSTRAP_MARKERS: &[&str] = &[
    "Flask(",
    "FastAPI(",
    "Bottle(",
    "falcon.App(",
    "Sanic(",
    "Starlette(",
    "web.Application(",
];

/// Candidate JS/TS entry filenames, most-conventional first — tried when no stronger
/// signal (an app-router pair, an `index.html` script tag) already answered.
const JS_ENTRY_CANDIDATES: &[&str] = &[
    "src/main.tsx",
    "src/main.ts",
    "src/main.jsx",
    "src/main.js",
    "src/index.tsx",
    "src/index.ts",
    "src/index.jsx",
    "src/index.js",
    "src/App.tsx",
    "src/App.jsx",
    "index.js",
    "index.mjs",
    "app.js",
    "app.ts",
];

/// Candidate Python entry filenames, most-conventional first.
const PY_ENTRY_CANDIDATES: &[&str] = &["wsgi.py", "asgi.py", "app.py", "main.py", "app/main.py"];

/// The project's entry point, tried by decreasing signal strength: a file-router
/// convention (needs no content check — the pair's existence is the whole signal), a
/// bundler's own declared entry, Django's generated bootstrap script, then a
/// conventional filename confirmed by an actual bootstrap call in its content. `None`
/// when nothing this cheap and this certain fired — never a guess.
fn detect_entry(root: &Path) -> Option<EntryMatch> {
    detect_app_router_entry(root)
        .or_else(|| detect_html_entry(root))
        .or_else(|| detect_js_marker_entry(root))
        .or_else(|| detect_manage_py_entry(root))
        .or_else(|| detect_python_marker_entry(root))
}

/// A file-based router convention: an `app/layout.*`+`app/page.*` pair (App Router) or a
/// `pages/_app.*`+`pages/index.*` pair (Pages Router), same extension — the shape
/// Next.js's own convention produces and nothing else is shaped like. No dependency
/// check needed: no other convention writes this exact pair.
fn detect_app_router_entry(root: &Path) -> Option<EntryMatch> {
    if !root.join("package.json").exists() {
        return None;
    }
    for ext in ["tsx", "jsx", "js"] {
        if let (Some(layout), Some(page)) = (
            existing_exact_case(root, &format!("app/layout.{ext}")),
            existing_exact_case(root, &format!("app/page.{ext}")),
        ) {
            return Some(EntryMatch {
                note: format!("app/layout.{ext} + app/page.{ext} file-router convention"),
                files: vec![layout, page],
            });
        }
    }
    for ext in ["tsx", "jsx", "js"] {
        if let (Some(app), Some(index)) = (
            existing_exact_case(root, &format!("pages/_app.{ext}")),
            existing_exact_case(root, &format!("pages/index.{ext}")),
        ) {
            return Some(EntryMatch {
                note: format!("pages/_app.{ext} + pages/index.{ext} file-router convention"),
                files: vec![app, index],
            });
        }
    }
    None
}

/// A bundler's own declared entry: `index.html`'s `<script type="module" src="...">` —
/// Vite's convention and the one nearly every framework-agnostic scaffold shares, so
/// reading it once covers React, Vue, Svelte, Solid, and plain-JS templates alike
/// without naming a single one of them.
fn detect_html_entry(root: &Path) -> Option<EntryMatch> {
    if !root.join("package.json").exists() {
        return None;
    }
    let html = std::fs::read_to_string(root.join("index.html")).ok()?;
    let src = html_script_src(&html)?;
    let relative = src.trim_start_matches("./").trim_start_matches('/');
    let path = existing_exact_case(root, relative)?;
    Some(EntryMatch {
        note: "index.html's script tag".to_string(),
        files: vec![path],
    })
}

/// The `src="..."` of the first `<script type="module" ...>` tag in `html` — a small,
/// deliberately narrow scan (module scripts only; a classic `<script src=jquery.js>` is
/// never a bundler entry) rather than a full HTML parser, the same "cheap, bounded,
/// deterministic" contract the rest of the survey holds itself to.
fn html_script_src(html: &str) -> Option<String> {
    let mut rest = html;
    loop {
        let start = rest.find("<script")?;
        let tag_end = rest[start..].find('>')? + start;
        let tag = &rest[start..=tag_end];
        if tag.contains("type=\"module\"") || tag.contains("type='module'") {
            if let Some(src) = attribute_value(tag, "src") {
                return Some(src);
            }
        }
        rest = &rest[tag_end + 1..];
    }
}

/// The quoted value of `name="..."` inside one HTML `tag`, `None` when the attribute is
/// absent or unquoted.
fn attribute_value(tag: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=");
    let at = tag.find(&needle)? + needle.len();
    let quote = *tag.as_bytes().get(at)?;
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    let value_start = at + 1;
    let value_end = tag[value_start..].find(quote as char)? + value_start;
    Some(tag[value_start..value_end].to_string())
}

/// The first [`JS_ENTRY_CANDIDATES`] file that exists **and** whose content contains a
/// [`JS_BOOTSTRAP_MARKERS`] call — the content check is load-bearing here, since without
/// a dependency manifest to confirm "this is a framework project," a bare candidate
/// filename with no bootstrap call is exactly as likely to be an unrelated script.
fn detect_js_marker_entry(root: &Path) -> Option<EntryMatch> {
    if !root.join("package.json").exists() {
        return None;
    }
    for candidate in JS_ENTRY_CANDIDATES {
        let Some(path) = existing_exact_case(root, candidate) else {
            continue;
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Some(marker) = JS_BOOTSTRAP_MARKERS.iter().find(|m| text.contains(**m)) {
            return Some(EntryMatch {
                note: format!("`{marker}` bootstrap call"),
                files: vec![path],
            });
        }
    }
    None
}

/// Django's own generated bootstrap script: `manage.py` invoking
/// `django.core.management` is machine-written boilerplate, near-identical across every
/// Django project — its presence and content are the whole signal, no dependency
/// manifest needed.
fn detect_manage_py_entry(root: &Path) -> Option<EntryMatch> {
    let path = existing_exact_case(root, "manage.py")?;
    let text = std::fs::read_to_string(&path).ok()?;
    if !text.contains("django.core.management") {
        return None;
    }
    Some(EntryMatch {
        note: "manage.py's Django bootstrap convention".to_string(),
        files: vec![path],
    })
}

/// The first [`PY_ENTRY_CANDIDATES`] file that exists **and** whose content contains a
/// [`PY_BOOTSTRAP_MARKERS`] call — the same content-confirms-convention rule
/// [`detect_js_marker_entry`] uses, so a plain `main.py` CLI script is never mistaken
/// for a web app's entry.
fn detect_python_marker_entry(root: &Path) -> Option<EntryMatch> {
    for candidate in PY_ENTRY_CANDIDATES {
        let Some(path) = existing_exact_case(root, candidate) else {
            continue;
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Some(marker) = PY_BOOTSTRAP_MARKERS.iter().find(|m| text.contains(**m)) {
            return Some(EntryMatch {
                note: format!("`{marker}` app construction"),
                files: vec![path],
            });
        }
    }
    None
}

/// `root.join(candidate)` when the file exists **and** its filename matches `candidate`'s
/// case exactly. `Path::is_file` alone answers "does something exist here," which is
/// true for `app.tsx` when `candidate` says `App.tsx` on a case-insensitive filesystem
/// (default macOS, Windows) — report `4c859012`. A mirror seeded from that mismatch
/// breaks the moment the tree is read somewhere case is enforced (Linux CI, most
/// containers), so this checks the parent directory's real listing instead of trusting
/// the OS to answer the question it was actually asked.
fn existing_exact_case(root: &Path, candidate: &str) -> Option<PathBuf> {
    let path = root.join(candidate);
    if !path.is_file() {
        return None;
    }
    let want = Path::new(candidate).file_name()?;
    let parent = path.parent()?;
    std::fs::read_dir(parent)
        .ok()?
        .filter_map(Result::ok)
        .any(|entry| entry.file_name() == want)
        .then_some(path)
}

/// The `lang=` a `::code src=` block should carry for a seeded entry file, by extension.
fn code_lang_for(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("tsx" | "ts") => "typescript",
        Some("py") => "python",
        _ => "javascript",
    }
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

        assert_eq!(detect_build(&root).map(|b| b.name()), Some("pytest"));

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

    /// A project that names its suite anything but the bare word `test` still gets a
    /// gate — the npm detector matches any `scripts` key containing "test", not only
    /// the exact key, and runs it with `npm run <key>`.
    #[test]
    fn an_npm_project_with_a_differently_named_test_script_still_gets_a_gate() {
        let root = project("npm-named");
        std::fs::write(
            root.join("package.json"),
            "{\"scripts\": {\"unit-test\": \"node --test\"}}",
        )
        .expect("file");
        let scaffold = write_scaffold(&root, false).expect("scaffold");
        let (harness, gates) = scaffold.harness.expect("harness written");
        assert_eq!(gates, 1);
        let text = workspace::read(&harness).expect("read");
        assert!(text.contains("npm run unit-test"), "{text}");
    }

    /// Make has no fixed vocabulary for a test target's name — `unit-test:` gets a gate
    /// exactly as `test:` would, not only the literal word.
    #[test]
    fn a_makefile_with_a_differently_named_test_target_gets_a_gate() {
        let root = project("make-named");
        std::fs::write(
            root.join("Makefile"),
            "build:\n\techo building\n\nunit-test:\n\techo testing\n",
        )
        .expect("file");
        let scaffold = write_scaffold(&root, false).expect("scaffold");
        let (harness, gates) = scaffold.harness.expect("harness written");
        assert_eq!(gates, 1);
        let text = workspace::read(&harness).expect("read");
        assert!(text.contains("make unit-test"), "{text}");
    }

    /// A Ruby project with an rspec suite gets `bundle exec rspec`.
    #[test]
    fn a_ruby_project_with_rspec_gets_a_gate() {
        let root = project("ruby-rspec");
        std::fs::write(root.join("Gemfile"), "source 'https://rubygems.org'\n").expect("file");
        std::fs::create_dir_all(root.join("spec")).expect("dirs");
        std::fs::write(
            root.join("spec/thing_spec.rb"),
            "RSpec.describe 'x' do end\n",
        )
        .expect("file");
        let scaffold = write_scaffold(&root, false).expect("scaffold");
        let (harness, gates) = scaffold.harness.expect("harness written");
        assert_eq!(gates, 1);
        assert!(workspace::read(&harness)
            .expect("read")
            .contains("bundle exec rspec"));
    }

    /// A Ruby project with a Rakefile `test` task and no rspec suite gets `bundle exec
    /// rake test` instead.
    #[test]
    fn a_ruby_project_with_only_a_rake_test_task_gets_a_gate() {
        let root = project("ruby-rake");
        std::fs::write(root.join("Gemfile"), "source 'https://rubygems.org'\n").expect("file");
        std::fs::write(
            root.join("Rakefile"),
            "require 'rake/testtask'\nRake::TestTask.new(:test)\n",
        )
        .expect("file");
        let scaffold = write_scaffold(&root, false).expect("scaffold");
        let (harness, _) = scaffold.harness.expect("harness written");
        assert!(workspace::read(&harness)
            .expect("read")
            .contains("bundle exec rake test"));
    }

    /// A Maven project (`pom.xml`) gets `mvn test`, run offline.
    #[test]
    fn a_maven_project_gets_a_gate() {
        let root = project("maven");
        std::fs::write(root.join("pom.xml"), "<project></project>\n").expect("file");
        let scaffold = write_scaffold(&root, false).expect("scaffold");
        let (harness, gates) = scaffold.harness.expect("harness written");
        assert_eq!(gates, 1);
        assert!(workspace::read(&harness)
            .expect("read")
            .contains("mvn -o -q test"));
    }

    /// A .NET project (a `.csproj` at the root) gets `dotnet test`.
    #[test]
    fn a_dotnet_project_gets_a_gate() {
        let root = project("dotnet");
        std::fs::write(root.join("app.csproj"), "<Project></Project>\n").expect("file");
        let scaffold = write_scaffold(&root, false).expect("scaffold");
        let (harness, gates) = scaffold.harness.expect("harness written");
        assert_eq!(gates, 1);
        assert!(workspace::read(&harness)
            .expect("read")
            .contains("dotnet test"));
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

    /// The scaffold always wires a traced index onto `dx trace --brief`, scaffolded like
    /// any other gate — a run block naming the mapped source directories in `reads=`, and
    /// never approved by the scaffold itself.
    #[test]
    fn the_scaffold_wires_a_traced_index_run_block() {
        let root = project("trace-wiring");
        let scaffold = write_scaffold(&root, false).expect("scaffold");
        let text = workspace::read(&scaffold.path).expect("read");

        assert!(
            text.contains("::heading level=2 id=trace\nTraced index"),
            "{text}"
        );
        // `node_modules/` never reaches `directories` — `listed()` skips it like every
        // other build/vendor directory — so the trace gate reads only `src`.
        assert!(
            text.contains("::code id=index-trace lang=bash run reads=src"),
            "{text}"
        );
        assert!(text.contains("dx trace --brief ."), "{text}");
        // Gates are never approved by scaffolding: no `::output` accompanies the block.
        assert!(!text.contains("::output id=index-trace-output"), "{text}");
    }

    /// A tree with no subdirectories still gets a meaningful `reads=`: the root-level
    /// code files themselves, the same fallback [`trace_reads`] documents.
    #[test]
    fn a_flat_tree_reads_its_own_root_files_for_the_trace_gate() {
        let root = std::env::temp_dir().join("dx-index-tests-trace-flat");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("dir");
        std::fs::write(root.join("main.py"), "def go():\n    pass\n").expect("file");

        let scaffold = write_scaffold(&root, false).expect("scaffold");
        let text = workspace::read(&scaffold.path).expect("read");
        assert!(text.contains("reads=main.py"), "{text}");
    }

    /// A JS project with a real bootstrap call (`createRoot(...)`) at a conventional
    /// path gets a live `::code src=` block for that file, seeded into the root area
    /// with a one-line note — no dependency on `react` being named anywhere: the
    /// content is the signal, so a framework this scaffold has never heard of, built on
    /// the same convention, is detected exactly the same way.
    #[test]
    fn a_js_project_gets_its_entry_point_mirrored_from_its_bootstrap_call() {
        let root = std::env::temp_dir().join("dx-index-tests-js-entry");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).expect("dirs");
        std::fs::write(root.join("package.json"), "{}").expect("manifest");
        std::fs::write(
            root.join("src/main.tsx"),
            "createRoot(document.getElementById('root')).render(null)\n",
        )
        .expect("entry");

        let scaffold = write_scaffold(&root, false).expect("scaffold");
        let text = workspace::read(&scaffold.path).expect("read");
        // Canonical stringify decides attribute order, not the order the scaffold wrote.
        assert!(
            text.contains("::code id=area-root-entry lang=typescript src=src/main.tsx"),
            "{text}"
        );
        assert!(
            text.contains("Entry point detected (`createRoot(` bootstrap call)"),
            "{text}"
        );
    }

    /// A conventionally-named file with no bootstrap call in it gets no entry block:
    /// without a dependency manifest to lean on, the content check is the whole signal,
    /// and a bare candidate filename is never enough to guess from.
    #[test]
    fn a_js_file_with_no_bootstrap_marker_gets_no_entry_block() {
        let root = std::env::temp_dir().join("dx-index-tests-plain-js");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).expect("dirs");
        std::fs::write(root.join("package.json"), "{}").expect("manifest");
        std::fs::write(root.join("src/main.tsx"), "export const x = 1;\n").expect("entry");

        let scaffold = write_scaffold(&root, false).expect("scaffold");
        let text = workspace::read(&scaffold.path).expect("read");
        assert!(!text.contains("area-root-entry"), "{text}");
        assert!(!text.contains("Entry point detected"), "{text}");
    }

    /// No conventional entry file on disk at all is still ambiguous — no guessing a
    /// path that does not exist.
    #[test]
    fn no_conventional_entry_file_seeds_nothing() {
        let root = std::env::temp_dir().join("dx-index-tests-no-entry");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("dir");
        std::fs::write(root.join("package.json"), "{}").expect("manifest");

        assert!(detect_entry(&root).is_none());
    }

    /// report `d288e338`: the entry detector seeds `main.tsx`/`App.tsx`, so the area
    /// listing's cheap "— entry point" label — driven by `ENTRY_POINTS`, entirely
    /// independent of entry detection — must know both names too, the same way it
    /// already knows `main.rs`.
    #[test]
    fn entry_point_labels_cover_the_conventional_js_filenames() {
        let root = std::env::temp_dir().join("dx-index-tests-entry-labels");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).expect("dirs");
        std::fs::write(root.join("src/main.tsx"), "console.log('boot')\n").expect("file");
        std::fs::write(
            root.join("src/App.tsx"),
            "export default function App() { return null }\n",
        )
        .expect("file");

        let scaffold = write_scaffold(&root, false).expect("scaffold");
        let text = workspace::read(&scaffold.path).expect("read");
        assert!(text.contains("src/main.tsx — entry point"), "{text}");
        assert!(text.contains("src/App.tsx — entry point"), "{text}");
    }

    /// report `df1d5589`, generalized: the file-router convention (`app/layout.*` +
    /// `app/page.*`) is Next.js's own shape, but detecting it needs no dependency named
    /// `next` at all — the pair's existence on disk is the whole signal, so a project
    /// naming no framework whatsoever, or an unrelated one, still gets it recognized.
    #[test]
    fn an_app_router_file_pair_is_detected_with_no_framework_named_in_the_manifest() {
        let root = std::env::temp_dir().join("dx-index-tests-app-router");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("app")).expect("dirs");
        std::fs::write(root.join("package.json"), "{\"dependencies\": {}}").expect("manifest");
        std::fs::write(
            root.join("app/layout.tsx"),
            "export default function RootLayout({children}) { return children }\n",
        )
        .expect("layout");
        std::fs::write(
            root.join("app/page.tsx"),
            "export default function Page() { return null }\n",
        )
        .expect("page");

        let scaffold = write_scaffold(&root, false).expect("scaffold");
        let text = workspace::read(&scaffold.path).expect("read");
        assert!(
            text.contains(
                "Entry point detected (app/layout.tsx + app/page.tsx file-router convention)"
            ),
            "{text}"
        );
        assert!(
            text.contains("::code id=area-root-entry-1 lang=typescript src=app/layout.tsx"),
            "{text}"
        );
        assert!(
            text.contains("::code id=area-root-entry-2 lang=typescript src=app/page.tsx"),
            "{text}"
        );
    }

    /// A bundler's own declared entry — `index.html`'s `<script type="module" src=>` —
    /// is detected directly, which is what makes this work for any Vite-shaped
    /// frontend regardless of which UI library it renders with.
    #[test]
    fn an_index_html_module_script_is_the_entry() {
        let root = std::env::temp_dir().join("dx-index-tests-html-entry");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).expect("dirs");
        std::fs::write(root.join("package.json"), "{}").expect("manifest");
        std::fs::write(
            root.join("index.html"),
            "<!doctype html><html><body><div id=\"root\"></div>\n\
             <script type=\"module\" src=\"/src/main.ts\"></script></body></html>\n",
        )
        .expect("html");
        std::fs::write(root.join("src/main.ts"), "console.log('boot')\n").expect("entry");

        let scaffold = write_scaffold(&root, false).expect("scaffold");
        let text = workspace::read(&scaffold.path).expect("read");
        assert!(
            text.contains("Entry point detected (index.html's script tag)"),
            "{text}"
        );
        assert!(
            text.contains("::code id=area-root-entry lang=typescript src=src/main.ts"),
            "{text}"
        );
    }

    /// report `4c859012` (case): `App.tsx` as a literal candidate must never resolve to
    /// a real file that is actually named `app.tsx` — [`existing_exact_case`] checks the
    /// directory's own listing rather than trusting `Path::is_file`, which answers yes
    /// on a case-insensitive filesystem regardless of the case asked for.
    #[test]
    fn a_lowercase_file_on_disk_never_satisfies_an_uppercase_candidate() {
        let root = std::env::temp_dir().join("dx-index-tests-case-exact");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).expect("dirs");
        std::fs::write(root.join("src/app.tsx"), "export function renderApp() {}\n").expect("file");

        assert!(existing_exact_case(&root, "src/App.tsx").is_none());
        assert!(existing_exact_case(&root, "src/app.tsx").is_some());
    }

    /// report `4c859012` (priority): a default Vite frontend ships both `App.tsx` (the
    /// component) and `main.tsx` (the file that actually bootstraps — calls
    /// `createRoot(...).render(...)`). `main.tsx` must win, never `App.tsx` merely
    /// because it sits earlier in the candidate list — and no dependency named `react`
    /// is needed anywhere for that to hold.
    #[test]
    fn main_tsx_wins_over_app_tsx_when_both_exist() {
        let root = std::env::temp_dir().join("dx-index-tests-vite-priority");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).expect("dirs");
        std::fs::write(root.join("package.json"), "{}").expect("manifest");
        std::fs::write(
            root.join("src/main.tsx"),
            "import App from './App'\ncreateRoot(document.getElementById('root')).render(App)\n",
        )
        .expect("main");
        std::fs::write(
            root.join("src/App.tsx"),
            "function App() { return null }\nexport default App\n",
        )
        .expect("app");

        let scaffold = write_scaffold(&root, false).expect("scaffold");
        let text = workspace::read(&scaffold.path).expect("read");
        assert!(
            text.contains("::code id=area-root-entry lang=typescript src=src/main.tsx"),
            "{text}"
        );
        assert!(
            !text.contains("::code id=area-root-entry lang=typescript src=src/App.tsx"),
            "{text}"
        );
    }

    /// The bootstrap-marker preference is content-based, not merely positional: even a
    /// hand-rolled layout where the file that actually calls the render/mount function
    /// sits *later* in the candidate list must still win over an earlier, marker-less
    /// file of the same shape.
    #[test]
    fn a_later_candidate_wins_when_it_alone_carries_the_bootstrap_marker() {
        let root = std::env::temp_dir().join("dx-index-tests-marker-override");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).expect("dirs");
        std::fs::write(root.join("package.json"), "{}").expect("manifest");
        // main.tsx exists and is checked first, but carries no bootstrap call.
        std::fs::write(root.join("src/main.tsx"), "// placeholder, unused\n").expect("main");
        // App.tsx, checked later, is the file that actually bootstraps here.
        std::fs::write(
            root.join("src/App.tsx"),
            "createRoot(document.getElementById('root')).render(null)\n",
        )
        .expect("app");

        let scaffold = write_scaffold(&root, false).expect("scaffold");
        let text = workspace::read(&scaffold.path).expect("read");
        assert!(
            text.contains("::code id=area-root-entry lang=typescript src=src/App.tsx"),
            "{text}"
        );
    }

    /// report `7f351075`, generalized further: a Python web entry needs no dependency
    /// manifest at all — `app.py` naming a `Flask(` construction is exactly as
    /// mechanically identifiable with no `requirements.txt` in sight, which is what
    /// makes an unlisted framework using the same shape detected just the same.
    #[test]
    fn a_python_web_entry_is_detected_from_its_app_construction_alone() {
        let root = std::env::temp_dir().join("dx-index-tests-flask");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("dir");
        std::fs::write(
            root.join("app.py"),
            "from flask import Flask\napp = Flask(__name__)\n\
             @app.route(\"/\")\ndef index():\n    return \"hi\"\n",
        )
        .expect("app");

        let scaffold = write_scaffold(&root, false).expect("scaffold");
        let text = workspace::read(&scaffold.path).expect("read");
        assert!(
            text.contains("Entry point detected (`Flask(` app construction)"),
            "{text}"
        );
        assert!(
            text.contains("::code id=area-root-entry lang=python src=app.py"),
            "{text}"
        );
    }

    /// A plain `main.py` with no web-app construction call is not treated as a web
    /// entry — the same "content confirms convention" rule that keeps an ordinary CLI
    /// script from being mistaken for one.
    #[test]
    fn a_plain_main_py_with_no_app_construction_seeds_nothing() {
        let root = std::env::temp_dir().join("dx-index-tests-plain-py");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("dir");
        std::fs::write(root.join("main.py"), "print('hello')\n").expect("file");

        assert!(detect_entry(&root).is_none());
    }

    /// Django's `manage.py` is detected from its own generated boilerplate — no
    /// `requirements.txt` or `pyproject.toml` naming `django` needed, since the script
    /// itself is the signal.
    #[test]
    fn a_django_project_is_detected_from_manage_py_alone() {
        let root = std::env::temp_dir().join("dx-index-tests-django");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("dir");
        std::fs::write(
            root.join("manage.py"),
            "#!/usr/bin/env python\nimport os\nimport sys\n\n\
             def main():\n    os.environ.setdefault('DJANGO_SETTINGS_MODULE', 'proj.settings')\n    \
             from django.core.management import execute_from_command_line\n    \
             execute_from_command_line(sys.argv)\n",
        )
        .expect("manage");

        let scaffold = write_scaffold(&root, false).expect("scaffold");
        let text = workspace::read(&scaffold.path).expect("read");
        assert!(
            text.contains("Entry point detected (manage.py's Django bootstrap convention)"),
            "{text}"
        );
        assert!(
            text.contains("::code id=area-root-entry lang=python src=manage.py"),
            "{text}"
        );
    }

    #[test]
    fn code_extensions_includes_objective_cpp_metal_and_assembly() {
        assert!(
            CODE_EXTENSIONS.contains(&"mm"),
            "CODE_EXTENSIONS must include 'mm' for Objective-C++"
        );
        assert!(
            CODE_EXTENSIONS.contains(&"metal"),
            "CODE_EXTENSIONS must include 'metal' for Metal GPU"
        );
        assert!(
            CODE_EXTENSIONS.contains(&"S"),
            "CODE_EXTENSIONS must include 'S' for Assembly"
        );
    }
}
