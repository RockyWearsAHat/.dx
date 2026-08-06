//! `doc-run` — run the code inside a `.dx` document and fold the results back in.
//!
//! A `.dx` file is a notepad you can execute. Mark a code block `run`, name the libraries
//! it needs, and the captured output becomes part of the document:
//!
//! ```text
//! ::code id=sizes lang=python run deps="requests"
//! import requests
//! print(len(requests.get("https://example.com").text))
//! ::end
//!
//! ::output id=sizes-output for=sizes status=ok hash=9f2c…
//! 1256
//! ::end
//! ```
//!
//! The `::output` block is written by [`run_document`], so the result is stored in the
//! document itself — readable by a person, by an agent, and by `git diff`, with no
//! sidecar state and no kernel to keep alive.
//!
//! # What makes a re-run cheap
//! Every run records a fingerprint of the code plus its dependencies. Running the document
//! again skips any block whose fingerprint still matches, so `dx run` over a large document
//! only executes what actually changed. A block whose code reads sibling files declares
//! them (`reads=site/site.css`), and their current text is part of the fingerprint too —
//! so editing a declared file re-runs the block, and the record never claims "no changes"
//! about content it read.
//!
//! # Safety
//! Running a document runs code someone else wrote, so it does not run with the reader's
//! authority. Every block executes inside a kernel-imposed sandbox — read widely, write only
//! its own directory, reach no network — described in full in [`confine`]. A machine that
//! cannot impose that boundary does not run the block; it reports it as blocked and says
//! why.
//!
//! Execution is never implicit either: it happens only through `dx run` or the `dx_run`
//! tool, never while reading or rendering. `DX_NO_EXEC=1` disables it entirely.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::all)]

pub mod confine;
pub mod plan;
pub mod process;
pub mod toolchain;

mod workdir;

/// Serializes tests that touch process environment variables (`HOME`, `DX_UNCONFINED`,
/// `DX_CACHE_DIR`).
///
/// The environment is process-global, so a test that mutates it races every concurrently
/// running sibling that reads it. Any test that sets, removes, or asserts on one of these
/// variables must hold this lock for its whole body.
#[cfg(test)]
pub(crate) fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());
    // Poison-tolerant: a failed sibling must not cascade into every later env test.
    ENV.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

use std::path::PathBuf;
use std::time::{Duration, Instant};

use doc_core::digest::sha256_hex;
use doc_core::format::{parse, stringify};
use doc_core::model::{runner_for_language, Block, Document};
use doc_core::resolve::{self, Resolver};

use confine::Grant;
use plan::parse_deps;
use process::Capture;

/// Seconds a block may run when it does not set its own `timeout`.
pub const DEFAULT_TIMEOUT_SECONDS: u64 = 60;

/// Longest output kept in a document; anything past this is truncated with a notice.
const MAX_OUTPUT_CHARS: usize = 20_000;

/// Exit code recorded when execution is disabled or no toolchain exists.
const BLOCKED_EXIT: i32 = 126;

/// How to run a document.
#[derive(Debug, Clone)]
pub struct RunOptions {
    /// Directory the document lives in; blocks run here so relative paths resolve.
    ///
    /// Readable, and — this is the point of the sandbox — not writable. A block opens the
    /// spreadsheet next to the document; it does not get to replace it.
    pub document_dir: PathBuf,
    /// Root of the per-block working directories and the shared toolchain caches.
    pub cache_root: PathBuf,
    /// Timeout for blocks that do not set their own.
    pub default_timeout: Duration,
    /// Re-run every block even when its fingerprint is unchanged.
    pub force: bool,
    /// Run only the block with this id, when set.
    pub only: Option<String>,
    /// If true, show which blocks would run and their source without executing.
    /// Used for code review before approval.
    pub review_only: bool,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            document_dir: PathBuf::from("."),
            cache_root: workdir::default_cache_root(),
            default_timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECONDS),
            force: false,
            only: None,
            review_only: false,
        }
    }
}

/// What happened to one runnable block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockRun {
    /// Id of the code block.
    pub id: String,
    /// Language as the author wrote it.
    pub language: String,
    /// `ok`, `error`, `skipped`, or `blocked`.
    pub status: String,
    /// Process exit code (`0` for skipped blocks).
    pub exit: i32,
    /// Captured output, truncated to a readable length.
    pub output: String,
    /// Wall-clock milliseconds spent, `0` when skipped.
    pub duration_ms: u64,
}

impl BlockRun {
    /// Whether this block ran and succeeded.
    #[must_use]
    pub fn succeeded(&self) -> bool {
        self.status == "ok" || self.status == "skipped"
    }
}

/// The result of running a whole document.
#[derive(Debug, Clone)]
pub struct RunReport {
    /// The document source with `::output` blocks refreshed.
    pub source: String,
    /// One entry per runnable block, in document order.
    pub runs: Vec<BlockRun>,
    /// Whether `source` differs from the input.
    pub changed: bool,
}

impl RunReport {
    /// Whether every block ran without error.
    #[must_use]
    pub fn all_succeeded(&self) -> bool {
        self.runs.iter().all(BlockRun::succeeded)
    }

    /// Number of blocks that actually executed (not skipped).
    #[must_use]
    pub fn executed(&self) -> usize {
        self.runs
            .iter()
            .filter(|run| run.status != "skipped")
            .count()
    }
}

/// Run every runnable code block in `source` and return the updated document.
///
/// Blocks execute in document order, so a later block sees files an earlier one wrote.
/// A block that fails does not stop the rest — the failure is recorded in its `::output`
/// and the document keeps going, which is what makes the result readable as a report.
///
/// `resolver` fills in `::code src=` listings before anything runs: the text executed is
/// the referenced file's current text, and its fingerprint tracks the file, so editing
/// the file makes the recorded output stale exactly like editing an inline body would.
/// Execution reads from a hydrated copy while `::output` blocks fold into the document
/// as written — the saved source keeps the reference, never a snapshot of the file. A
/// listing whose file cannot be resolved is `blocked`, not run: the body standing in its
/// place is a sentence about the missing file, and executing a sentence helps nobody.
/// Callers with no folder to resolve against pass [`resolve::Nowhere`].
#[must_use]
pub fn run_document(source: &str, options: &RunOptions, resolver: &dyn Resolver) -> RunReport {
    let document = parse(source);
    let mut hydrated = document.clone();
    let unresolved = resolve::hydrate(&mut hydrated, resolver);
    let mut runs: Vec<BlockRun> = Vec::new();
    let mut outputs: Vec<(String, Block)> = Vec::new();

    for (index, block) in document.blocks.iter().enumerate() {
        let Some(runner) = runnable_runner(block) else {
            continue;
        };
        if !selected(block, options) {
            continue;
        }

        // Hydration edits blocks in place and only ever appends, so the indices agree.
        let block = &hydrated.blocks[index];
        if let Some(problem) = unresolved.iter().find(|entry| entry.block == block.id) {
            runs.push(BlockRun {
                id: block.id.clone(),
                language: block.language.clone(),
                status: "blocked".to_string(),
                exit: BLOCKED_EXIT,
                output: problem.sentence.clone(),
                duration_ms: 0,
            });
            outputs.push((
                block.id.clone(),
                output_block(block, "blocked", BLOCKED_EXIT, "", &problem.sentence),
            ));
            continue;
        }

        let deps = parse_deps(&block.deps);
        let reads = match declared_reads(block, resolver) {
            Ok(reads) => reads,
            Err(sentence) => {
                runs.push(BlockRun {
                    id: block.id.clone(),
                    language: block.language.clone(),
                    status: "blocked".to_string(),
                    exit: BLOCKED_EXIT,
                    output: sentence.clone(),
                    duration_ms: 0,
                });
                outputs.push((
                    block.id.clone(),
                    output_block(block, "blocked", BLOCKED_EXIT, "", &sentence),
                ));
                continue;
            }
        };
        let fingerprint = fingerprint(runner, &block.text, &deps, &reads);
        let existing = existing_output(&document, index, &block.id);

        // Review mode: show what would run without executing
        if options.review_only {
            runs.push(BlockRun {
                id: block.id.clone(),
                language: block.language.clone(),
                status: "review".to_string(),
                exit: 0,
                output: format!("--- Code to review ---\n{}\n--- End code ---", block.text),
                duration_ms: 0,
            });
            continue;
        }

        if !options.force && existing.is_some_and(|output| output.hash == fingerprint) {
            runs.push(BlockRun {
                id: block.id.clone(),
                language: block.language.clone(),
                status: "skipped".to_string(),
                exit: 0,
                output: existing
                    .map(|output| output.text.clone())
                    .unwrap_or_default(),
                duration_ms: 0,
            });
            continue;
        }

        let started = Instant::now();
        let capture = execute(runner, block, &deps, &fingerprint, options);
        let elapsed = started.elapsed().as_millis() as u64;
        let output = truncate(&capture.output);
        let status = status_of(&capture);

        runs.push(BlockRun {
            id: block.id.clone(),
            language: block.language.clone(),
            status: status.clone(),
            exit: capture.exit,
            output: output.clone(),
            duration_ms: elapsed,
        });
        outputs.push((
            block.id.clone(),
            output_block(block, &status, capture.exit, &fingerprint, &output),
        ));
    }

    let updated = fold_outputs(&document, &runs, &outputs);
    let rendered = stringify(&updated);
    RunReport {
        changed: rendered != stringify(&document),
        source: rendered,
        runs,
    }
}

/// The runner for a block, when the block is executable code in a supported language.
fn runnable_runner(block: &Block) -> Option<&'static str> {
    if block.kind != "code" || !block.run {
        return None;
    }
    runner_for_language(&block.language)
}

/// Whether `block` is covered by the caller's `only` filter.
fn selected(block: &Block, options: &RunOptions) -> bool {
    match &options.only {
        Some(wanted) => block
            .id
            .eq_ignore_ascii_case(wanted.trim().trim_start_matches('#')),
        None => true,
    }
}

/// The `::output` block that already belongs to the code block at `index`, if any.
fn existing_output<'a>(document: &'a Document, index: usize, id: &str) -> Option<&'a Block> {
    document
        .blocks
        .get(index + 1)
        .filter(|block| block.kind == "output" && block.for_block == id)
}

/// The files a block declares it reads (`reads=`), each resolved to its current text.
///
/// Paths are comma-separated and obey the reference path law. A path the law refuses, or
/// a file the resolver cannot produce, is an error sentence — a fingerprint that silently
/// omitted a missing input would let the record claim "no changes" about content it never
/// saw, which is the lie `reads=` exists to prevent.
fn declared_reads(block: &Block, resolver: &dyn Resolver) -> Result<Vec<(String, String)>, String> {
    let mut reads = Vec::new();
    for path in block
        .reads
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        let Some(confined) = resolve::confined(path) else {
            return Err(format!(
                "{path} is not a path this block may declare — a `reads=` file stays \
                 inside the document's own folder, relative and walking downward."
            ));
        };
        let Some(text) = resolver.file(confined) else {
            return Err(format!(
                "{confined} could not be read here — the block declares it with `reads=`, \
                 and its content is part of what decides whether the recorded output is \
                 still current. Check the path against the document's folder."
            ));
        };
        reads.push((confined.to_string(), text));
    }
    Ok(reads)
}

/// Fingerprint the inputs that decide a block's output: runner, code, dependencies, and
/// the current text of every file the block declares it reads.
fn fingerprint(runner: &str, code: &str, deps: &[String], reads: &[(String, String)]) -> String {
    let mut material = format!("{runner}\u{1f}{}\u{1f}{code}", deps.join(","));
    for (path, text) in reads {
        material.push('\u{1f}');
        material.push_str(path);
        material.push('\u{1f}');
        material.push_str(text);
    }
    sha256_hex(material.as_bytes())[..16].to_string()
}

/// Execute one block, returning a capture even when nothing could be started.
fn execute(
    runner: &str,
    block: &Block,
    deps: &[String],
    fingerprint: &str,
    options: &RunOptions,
) -> Capture {
    if execution_disabled() {
        return blocked("execution is disabled (DX_NO_EXEC is set); no code was run");
    }

    let dirs = plan::Dirs {
        block: options.cache_root.join(runner).join(fingerprint),
        toolchains: options.cache_root.join("toolchains"),
    };
    let prepared = match plan::build(runner, &block.text, deps, &dirs) {
        Ok(prepared) => prepared,
        Err(message) => return blocked(&message),
    };

    let timeout = if block.timeout > 0 {
        Duration::from_secs(u64::from(block.timeout))
    } else {
        options.default_timeout
    };

    // Installing declared libraries is the one phase that may reach the network, and it is
    // confined in every other way — an `npm install` runs the package's own scripts.
    let writable = vec![dirs.block.clone(), dirs.toolchains.clone()];
    let installing = Grant::offline(writable.clone()).with_network();
    if let Err(message) = workdir::prepare(&dirs.block, &prepared, &installing, timeout) {
        return blocked(&message);
    }

    // The block's own code: the same directories writable, and no network at all.
    let command = match confine::confine(
        &home_in_block(&prepared.run, block, &dirs),
        &Grant::offline(writable),
    ) {
        Ok(command) => command,
        Err(message) => return blocked(&message),
    };

    let mut capture = process::run(&command, &options.document_dir, timeout);
    if confine::overridden() {
        capture.output = format!("{}\n{}", confine::UNCONFINED_NOTICE, capture.output);
    }
    capture
}

/// Point the block's home, temp, and cache directories at its own working directory.
///
/// Not decoration: the sandbox makes the reader's home read-only, and a toolchain whose
/// first act is to write `~/.matplotlib` or `~/.cache` would fail on a line the author never
/// wrote. Redirecting them means the ordinary libraries work *and* their scratch files land
/// somewhere the block is allowed to put them.
fn home_in_block(
    run: &process::CommandSpec,
    block: &Block,
    dirs: &plan::Dirs,
) -> process::CommandSpec {
    let block_dir = dirs.block.to_string_lossy().into_owned();
    run.clone()
        .with_env("HOME", block_dir.clone())
        .with_env("TMPDIR", block_dir.clone())
        .with_env("TEMP", block_dir.clone())
        .with_env(
            "XDG_CACHE_HOME",
            dirs.toolchains.to_string_lossy().into_owned(),
        )
        .with_env("DX_BLOCK_ID", block.id.clone())
        .with_env("DX_SANDBOX", block_dir)
}

/// Whether the `DX_NO_EXEC` kill switch is set.
fn execution_disabled() -> bool {
    std::env::var("DX_NO_EXEC").is_ok_and(|value| value != "0" && !value.is_empty())
}

/// A capture standing in for a block that could not be attempted.
fn blocked(message: &str) -> Capture {
    Capture {
        output: message.to_string(),
        exit: BLOCKED_EXIT,
        timed_out: false,
    }
}

/// Classify a capture into the status recorded on the `::output` block.
fn status_of(capture: &Capture) -> String {
    if capture.exit == BLOCKED_EXIT {
        "blocked".to_string()
    } else if capture.succeeded() {
        "ok".to_string()
    } else {
        "error".to_string()
    }
}

/// Clip output that is too long to belong in a document, saying so where it was cut.
fn truncate(output: &str) -> String {
    if output.chars().count() <= MAX_OUTPUT_CHARS {
        return output.to_string();
    }
    let head: String = output.chars().take(MAX_OUTPUT_CHARS).collect();
    format!("{head}\n--- output truncated at {MAX_OUTPUT_CHARS} characters ---")
}

/// Build the `::output` block recording one run.
///
/// The code block's `format` is carried across so a block that drew an SVG renders as a
/// picture rather than as quoted markup.
fn output_block(source: &Block, status: &str, exit: i32, fingerprint: &str, output: &str) -> Block {
    Block {
        kind: "output".to_string(),
        id: format!("{}-output", source.id),
        for_block: source.id.clone(),
        format: source.format.clone(),
        status: status.to_string(),
        exit,
        hash: fingerprint.to_string(),
        text: if output.is_empty() {
            "(no output)".to_string()
        } else {
            output.to_string()
        },
        ..Block::default()
    }
}

/// Rebuild the document with refreshed `::output` blocks in place.
///
/// Each freshly produced output replaces the one that followed its code block; outputs for
/// blocks that were skipped or not selected are left exactly as they were.
fn fold_outputs(document: &Document, runs: &[BlockRun], outputs: &[(String, Block)]) -> Document {
    let refreshed: Vec<&String> = outputs.iter().map(|(id, _)| id).collect();
    let mut blocks: Vec<Block> = Vec::with_capacity(document.blocks.len() + outputs.len());

    for block in &document.blocks {
        // Drop the stale output; the replacement is appended with its code block below.
        if block.kind == "output" && refreshed.iter().any(|id| **id == block.for_block) {
            continue;
        }
        blocks.push(block.clone());
        if let Some((_, output)) = outputs.iter().find(|(id, _)| *id == block.id) {
            blocks.push(output.clone());
        }
    }

    let _ = runs;
    Document {
        blocks,
        ..document.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run `source` in a scratch directory, isolated from any real cache.
    fn run_isolated(source: &str, label: &str) -> RunReport {
        let root = std::env::temp_dir().join(format!("dx-run-tests-{label}"));
        let _ = std::fs::create_dir_all(&root);
        run_document(
            source,
            &RunOptions {
                document_dir: root.clone(),
                cache_root: root.join("cache"),
                default_timeout: Duration::from_secs(60),
                ..RunOptions::default()
            },
            &resolve::Nowhere,
        )
    }

    #[test]
    fn a_block_without_run_is_left_alone() {
        let source = "::code id=c lang=python\nprint(1)\n::end\n";
        let report = run_isolated(source, "no-run");
        assert!(report.runs.is_empty());
        assert!(!report.changed);
        assert_eq!(report.source, source);
    }

    #[test]
    fn a_language_with_no_runner_is_not_executed() {
        let source = "::code id=c lang=css run\nbody {}\n::end\n";
        assert!(run_isolated(source, "no-runner").runs.is_empty());
    }

    #[test]
    fn shell_output_is_captured_into_the_document() {
        let source = "::code id=greet lang=bash run\necho hello from dx\n::end\n";
        let report = run_isolated(source, "shell");
        assert_eq!(report.runs.len(), 1);
        assert_eq!(report.runs[0].status, "ok");
        assert_eq!(report.runs[0].output, "hello from dx");
        assert!(report
            .source
            .contains("::output id=greet-output for=greet status=ok"));
        assert!(report.source.contains("hello from dx"));
        assert!(report.changed);
    }

    #[test]
    fn a_failing_block_records_its_exit_code_and_keeps_going() {
        let source = "::code id=bad lang=bash run\necho oops 1>&2; exit 3\n::end\n\n\
::code id=good lang=bash run\necho fine\n::end\n";
        let report = run_isolated(source, "failure");
        assert_eq!(report.runs.len(), 2);
        assert_eq!(report.runs[0].status, "error");
        assert_eq!(report.runs[0].exit, 3);
        assert_eq!(report.runs[1].status, "ok");
        assert!(report.source.contains("status=error exit=3"));
        assert!(!report.all_succeeded());
    }

    /// A plain-folder resolver, standing in for the CLI's store-aware one.
    struct Folder(PathBuf);

    impl Resolver for Folder {
        fn file(&self, path: &str) -> Option<String> {
            std::fs::read_to_string(self.0.join(path)).ok()
        }
        fn document(&self, path: &str) -> Option<String> {
            std::fs::read_to_string(self.0.join(path)).ok()
        }
    }

    #[test]
    fn a_listing_whose_file_is_missing_is_blocked_never_executed() {
        let source = "::code id=listing src=src/gone.sh lang=bash run\n::end\n";
        let report = run_isolated(source, "missing-src");
        assert_eq!(report.runs.len(), 1);
        assert_eq!(report.runs[0].status, "blocked");
        assert_eq!(report.runs[0].exit, BLOCKED_EXIT);
        assert!(report.runs[0].output.contains("src/gone.sh"));
        // The failure is on the page, and the saved source keeps the reference untouched.
        assert!(report.source.contains("status=blocked"));
        assert!(report.source.contains("src=src/gone.sh"));
        assert!(!report.all_succeeded());
    }

    #[test]
    fn a_listing_runs_its_files_current_text_and_saves_the_reference_not_a_copy() {
        let root = std::env::temp_dir().join("dx-run-tests-src");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).expect("scene");
        std::fs::write(root.join("src/greet.sh"), "echo from the file\n").expect("fixture");
        let report = run_document(
            "::code id=greet src=src/greet.sh lang=bash run\n::end\n",
            &RunOptions {
                document_dir: root.clone(),
                cache_root: root.join("cache"),
                ..RunOptions::default()
            },
            &Folder(root.clone()),
        );
        assert_eq!(report.runs[0].status, "ok");
        assert_eq!(report.runs[0].output, "from the file");
        // The reference survives the save with its body empty: a reference, not a copy.
        assert!(report
            .source
            .contains("::code id=greet lang=bash src=src/greet.sh run\n\n::end"));
    }

    #[test]
    fn an_unchanged_block_is_skipped_on_the_second_run() {
        let source = "::code id=once lang=bash run\necho stable\n::end\n";
        let first = run_isolated(source, "skip");
        assert_eq!(first.runs[0].status, "ok");

        let second = run_isolated(&first.source, "skip");
        assert_eq!(second.runs[0].status, "skipped");
        assert_eq!(second.executed(), 0);
        assert!(!second.changed);
        assert_eq!(second.source, first.source);
    }

    #[test]
    fn editing_a_block_invalidates_its_recorded_output() {
        let first = run_isolated("::code id=v lang=bash run\necho one\n::end\n", "invalidate");
        let edited = first.source.replace("echo one", "echo two");
        let second = run_isolated(&edited, "invalidate");
        assert_eq!(second.runs[0].status, "ok");
        assert_eq!(second.runs[0].output, "two");
        assert!(!second.source.contains("one"));
    }

    #[test]
    fn output_replaces_rather_than_accumulates() {
        let source = "::code id=r lang=bash run\necho x\n::end\n";
        let once = run_isolated(source, "replace");
        let twice = run_isolated(&once.source.replace("echo x", "echo y"), "replace");
        assert_eq!(twice.source.matches("::output").count(), 1);
    }

    #[test]
    fn only_runs_the_requested_block() {
        let source = "::code id=a lang=bash run\necho a\n::end\n\n\
::code id=b lang=bash run\necho b\n::end\n";
        let root = std::env::temp_dir().join("dx-run-tests-only");
        let _ = std::fs::create_dir_all(&root);
        let report = run_document(
            source,
            &RunOptions {
                document_dir: root.clone(),
                cache_root: root.join("cache"),
                only: Some("b".to_string()),
                ..RunOptions::default()
            },
            &resolve::Nowhere,
        );
        assert_eq!(report.runs.len(), 1);
        assert_eq!(report.runs[0].id, "b");
    }

    #[test]
    fn a_block_that_overruns_its_timeout_is_killed() {
        let source = "::code id=slow lang=bash run timeout=1\nsleep 30\n::end\n";
        let report = run_isolated(source, "timeout");
        assert_eq!(report.runs[0].status, "error");
        assert!(report.runs[0].output.contains("timed out"));
    }

    #[test]
    fn blocks_run_in_the_documents_directory() {
        let root = std::env::temp_dir().join("dx-run-tests-cwd");
        let _ = std::fs::create_dir_all(&root);
        std::fs::write(root.join("beside.txt"), "found me").expect("write fixture");
        let report = run_document(
            "::code id=read lang=bash run\ncat beside.txt\n::end\n",
            &RunOptions {
                document_dir: root.clone(),
                cache_root: root.join("cache"),
                ..RunOptions::default()
            },
            &resolve::Nowhere,
        );
        assert_eq!(report.runs[0].output, "found me");
    }

    #[test]
    fn long_output_is_truncated_with_a_notice() {
        let clipped = truncate(&"x".repeat(MAX_OUTPUT_CHARS + 500));
        assert!(clipped.contains("output truncated"));
        assert!(clipped.chars().count() < MAX_OUTPUT_CHARS + 200);
    }

    #[test]
    fn empty_output_is_recorded_explicitly() {
        let report = run_isolated("::code id=quiet lang=bash run\ntrue\n::end\n", "quiet");
        assert!(report.source.contains("(no output)"));
    }

    #[test]
    fn fingerprints_change_with_code_and_dependencies() {
        let base = fingerprint("python", "print(1)", &[], &[]);
        assert_ne!(base, fingerprint("python", "print(2)", &[], &[]));
        assert_ne!(
            base,
            fingerprint("python", "print(1)", &["rich".into()], &[])
        );
        assert_ne!(base, fingerprint("node", "print(1)", &[], &[]));
        assert_eq!(base.len(), 16);
    }

    #[test]
    fn fingerprints_change_with_declared_reads() {
        let base = fingerprint("python", "print(1)", &[], &[]);
        let read = fingerprint(
            "python",
            "print(1)",
            &[],
            &[("site.css".into(), "body{}".into())],
        );
        assert_ne!(base, read);
        // The same file with different content is a different fingerprint — that is the
        // whole point of declaring it.
        assert_ne!(
            read,
            fingerprint(
                "python",
                "print(1)",
                &[],
                &[("site.css".into(), "body{color:red}".into())],
            )
        );
        // A renamed file is a different fingerprint even with identical content.
        assert_ne!(
            read,
            fingerprint(
                "python",
                "print(1)",
                &[],
                &[("other.css".into(), "body{}".into())],
            )
        );
    }

    #[test]
    fn a_missing_declared_read_blocks_the_run() {
        let source = "::code id=check lang=python run reads=site/site.css\nprint(1)\n::end\n";
        let report = run_document(source, &RunOptions::default(), &resolve::Nowhere);
        assert_eq!(report.runs.len(), 1);
        assert_eq!(report.runs[0].status, "blocked");
        assert!(report.runs[0].output.contains("site/site.css"));
    }

    #[test]
    fn a_read_outside_the_folder_blocks_the_run() {
        let source = "::code id=check lang=python run reads=../secrets\nprint(1)\n::end\n";
        let report = run_document(source, &RunOptions::default(), &resolve::Nowhere);
        assert_eq!(report.runs.len(), 1);
        assert_eq!(report.runs[0].status, "blocked");
        assert!(report.runs[0].output.contains("../secrets"));
    }

    #[test]
    fn editing_a_declared_file_stales_the_recorded_output() {
        let mut provided = resolve::Provided::new();
        provided.add_file("site.css", "body{}");
        let block = Block {
            kind: "code".into(),
            id: "check".into(),
            language: "python".into(),
            run: true,
            reads: "site.css".into(),
            text: "print(1)".into(),
            ..Block::default()
        };
        let before = declared_reads(&block, &provided).expect("resolves");
        let recorded = fingerprint("python", &block.text, &[], &before);

        let mut edited = resolve::Provided::new();
        edited.add_file("site.css", "body{color:red}");
        let after = declared_reads(&block, &edited).expect("resolves");
        assert_ne!(recorded, fingerprint("python", &block.text, &[], &after));
    }
}
