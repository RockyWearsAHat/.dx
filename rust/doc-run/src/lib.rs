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
//! Execution is also gated on review: a block whose fingerprint this machine has never
//! approved is blocked pending review rather than run. Approval is local and only local —
//! an `::output` block carried by the document proves nothing, because the hand that wrote
//! the code wrote that record too. The gate is checked before the fingerprint cache, so a
//! document that arrives with a matching run record is reviewed like any other.
//! [`RunOptions::review_only`] shows what would run without running it,
//! [`RunOptions::approve`] records the current fingerprints into the [`approvals::Ledger`]
//! and runs, and [`RunOptions::force`] runs past the gate once, stamping [`FORCED_NOTICE`]
//! into the output it produces. Editing a block changes its fingerprint, so approval
//! expires with the edit.
//!
//! Execution is never implicit either: it happens only through `dx run` or the `dx_run`
//! tool, never while reading or rendering. `DX_NO_EXEC=1` disables it entirely.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::all)]

pub mod approvals;
pub mod confine;
pub mod plan;
pub mod process;
pub mod toolchain;

mod order;
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

/// The line stamped into a block's output when `--force` ran it past the approval gate.
///
/// Mirrors [`confine::UNCONFINED_NOTICE`]: a bypass announces itself in the record it
/// produces, so a document never carries output from unreviewed code without saying so.
pub const FORCED_NOTICE: &str =
    "--- ran without approval: --force bypassed the review gate for this block ---";

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
    /// Re-run every block even when its fingerprint is unchanged, and run past the
    /// approval gate — a block forced past it carries [`FORCED_NOTICE`] in its output.
    pub force: bool,
    /// Run only the block with this id, when set.
    pub only: Option<String>,
    /// If true, show which blocks would run and their source without executing.
    /// Used for code review before approval.
    pub review_only: bool,
    /// Record each runnable block's current fingerprint as approved, then execute.
    pub approve: bool,
    /// Order execution by the document's board edges instead of document order.
    ///
    /// An edge on a board says *this, then that*, and this flag takes it at its word: a
    /// runnable block waits for every runnable block with an edge path into it, through
    /// non-runnable nodes too (`setup -> note -> test` still runs `setup` before `test`).
    /// At every step the earliest ready block by document position runs next, so an edge
    /// that defers a block lets later blocks — on a board or not — run before it; ties
    /// always break by document order, which keeps the result deterministic. Document-order
    /// side-effect dependencies between blocks the boards leave unrelated are not
    /// preserved — state an edge if you need an order. A cycle among runnable blocks is an
    /// error naming its blocks. Default `false`: document order, exactly as before the
    /// flag existed.
    pub follow_board_edges: bool,
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
            approve: false,
            follow_board_edges: false,
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
    /// `ok`, `error`, `skipped`, `blocked`, or `review`.
    pub status: String,
    /// Process exit code (`0` for skipped blocks).
    pub exit: i32,
    /// Captured output, truncated to a readable length.
    pub output: String,
    /// Wall-clock milliseconds spent, `0` when skipped.
    pub duration_ms: u64,
}

impl BlockRun {
    /// Whether nothing went wrong: the block ran and succeeded, its cached result still
    /// stands, or it was inspected in review mode — which executes nothing and fails
    /// nothing.
    #[must_use]
    pub fn succeeded(&self) -> bool {
        self.status == "ok" || self.status == "skipped" || self.status == "review"
    }
}

/// The result of running a whole document.
#[derive(Debug, Clone)]
pub struct RunReport {
    /// The document source with `::output` blocks refreshed.
    pub source: String,
    /// One entry per runnable block, in the order they ran — document order, unless
    /// [`RunOptions::follow_board_edges`] reordered them.
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

    /// Number of blocks that actually executed — a skip, a refusal, and a review all
    /// launched nothing.
    #[must_use]
    pub fn executed(&self) -> usize {
        self.runs
            .iter()
            .filter(|run| run.status == "ok" || run.status == "error")
            .count()
    }
}

/// Run every runnable code block in `source` and return the updated document.
///
/// Blocks execute in document order, so a later block sees files an earlier one wrote.
/// [`RunOptions::follow_board_edges`] orders them by the document's board edges instead —
/// there an edge that defers a block lets later blocks (on a board or not) run before it,
/// so only the stated edges order side effects, not document position. The order is still
/// deterministic, and it is the only way this function fails: a cycle among the selected
/// runnable blocks is `Err` with a sentence naming the cycle, because a cycle states no
/// order. [`RunOptions::only`] narrows the graph before the order is computed, so a cycle
/// among blocks it does not select cannot refuse the one it does.
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
///
/// A block whose fingerprint is not approved in the [`approvals::Ledger`] is `blocked`
/// pending review — reported with the way forward, never silently skipped, its stale output
/// untouched. Only that local ledger approves: a document's own `::output` record is
/// content its author controls, so it can neither approve code nor suppress the gate.
/// See [`RunOptions`] for `review_only`, `approve`, and `force`.
pub fn run_document(
    source: &str,
    options: &RunOptions,
    resolver: &dyn Resolver,
) -> Result<RunReport, String> {
    // Review executes nothing and records nothing, so it cannot also approve or force —
    // accepting the pair and dropping half would be an option silently swallowed, and the
    // rule lives here so every surface refuses it identically.
    if options.review_only && options.approve {
        return Err("review shows code without executing and records nothing; \
             run approve separately, without review, to approve and execute it"
            .to_string());
    }
    if options.review_only && options.force {
        return Err("review shows code without executing and records nothing; \
             force executes — ask for one or the other"
            .to_string());
    }
    let document = parse(source);
    let mut hydrated = document.clone();
    let unresolved = resolve::hydrate(&mut hydrated, resolver);
    let ledger = approvals::Ledger::at(&options.cache_root);
    let mut runs: Vec<BlockRun> = Vec::new();
    let mut outputs: Vec<(String, Block)> = Vec::new();

    // `only` narrows the set here, ahead of the edge sort, so a cycle among blocks the
    // caller did not select cannot veto the one they did.
    let runnable: Vec<usize> = document
        .blocks
        .iter()
        .enumerate()
        .filter(|(_, block)| runnable_runner(block).is_some() && selected(block, options))
        .map(|(index, _)| index)
        .collect();
    let ordered = if options.follow_board_edges {
        order::edge_order(&document, &runnable)?
    } else {
        runnable
    };

    for index in ordered {
        let Some(runner) = runnable_runner(&document.blocks[index]) else {
            continue;
        };

        // Hydration edits blocks in place and only ever appends, so the indices agree.
        let block = &hydrated.blocks[index];
        if let Some(problem) = unresolved.iter().find(|entry| entry.block == block.id) {
            refuse(block, &problem.sentence, options, &mut runs, &mut outputs);
            continue;
        }

        let deps = parse_deps(&block.deps);
        let reads = match declared_reads(block, resolver) {
            Ok(reads) => reads,
            Err(sentence) => {
                refuse(block, &sentence, options, &mut runs, &mut outputs);
                continue;
            }
        };
        let fingerprint = fingerprint(runner, &block.text, &deps, &reads);
        let existing = existing_output(&document, index, &block.id);

        // Review mode: show what would run without executing — and without recording
        // anything, because reading never writes.
        if options.review_only {
            runs.push(BlockRun {
                id: block.id.clone(),
                language: block.language.clone(),
                status: "review".to_string(),
                exit: 0,
                output: review_text(&block.text, &fingerprint, ledger.is_approved(&fingerprint)),
                duration_ms: 0,
            });
            continue;
        }

        if options.approve {
            if let Err(sentence) = ledger.approve(&fingerprint) {
                runs.push(BlockRun {
                    id: block.id.clone(),
                    language: block.language.clone(),
                    status: "blocked".to_string(),
                    exit: BLOCKED_EXIT,
                    output: sentence,
                    duration_ms: 0,
                });
                continue;
            }
        }

        // Approval is this machine's own record and nothing else. The document's `::output`
        // block cannot vouch for the code above it: it is content the same hand wrote, and
        // its `hash=` is computable by whoever authored the block.
        let approved = ledger.is_approved(&fingerprint);

        // The gate stands ahead of the cache, so a document that arrives carrying a matching
        // run record is still reviewed rather than quietly accepted as already proven. The
        // refusal names its way forward, and the block's stale output is left as it was.
        if !approved && !options.force {
            runs.push(BlockRun {
                id: block.id.clone(),
                language: block.language.clone(),
                status: "blocked".to_string(),
                exit: BLOCKED_EXIT,
                output: pending_review(&fingerprint),
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

        // Reachable unapproved only through `--force`, which is the bypass that must say so.
        let bypassed = !approved;
        let started = Instant::now();
        let mut capture = execute(runner, block, &deps, &fingerprint, options);
        if bypassed {
            capture.output = format!("{FORCED_NOTICE}\n{}", capture.output);
        }
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
    Ok(RunReport {
        changed: rendered != stringify(&document),
        source: rendered,
        runs,
    })
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

/// Record a refusal decided before anything could run: an unresolved `src=` listing, or a
/// `reads=` file the document may not have.
///
/// The sentence is always reported to the caller. It is folded into the document as the
/// block's `::output` only outside review mode — a review executes nothing, so it also
/// writes nothing, and it fails nothing: what it found is reported as `review`.
fn refuse(
    block: &Block,
    sentence: &str,
    options: &RunOptions,
    runs: &mut Vec<BlockRun>,
    outputs: &mut Vec<(String, Block)>,
) {
    let (status, exit) = if options.review_only {
        ("review", 0)
    } else {
        ("blocked", BLOCKED_EXIT)
    };
    runs.push(BlockRun {
        id: block.id.clone(),
        language: block.language.clone(),
        status: status.to_string(),
        exit,
        output: sentence.to_string(),
        duration_ms: 0,
    });
    if !options.review_only {
        outputs.push((
            block.id.clone(),
            output_block(block, "blocked", BLOCKED_EXIT, "", sentence),
        ));
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
///
/// The full digest, untruncated: this value is what the approval ledger trusts, and a
/// truncated hash is one a hostile author could collide — a benign block the reader
/// approves and a payload sharing its shortened fingerprint.
fn fingerprint(runner: &str, code: &str, deps: &[String], reads: &[(String, String)]) -> String {
    let mut material = format!("{runner}\u{1f}{}\u{1f}{code}", deps.join(","));
    for (path, text) in reads {
        material.push('\u{1f}');
        material.push_str(path);
        material.push('\u{1f}');
        material.push_str(text);
    }
    sha256_hex(material.as_bytes())
}

/// What review mode reports for one block: the exact code that would run (hydrated, so
/// `src=` listings show the file's current text), its fingerprint, and where it stands
/// with the approval gate.
fn review_text(code: &str, fingerprint: &str, approved: bool) -> String {
    let standing = if approved {
        "approved — a plain run executes this code"
    } else {
        "not approved — a run with approve records it and executes"
    };
    format!(
        "fingerprint {fingerprint}\napproval {standing}\n--- code ---\n{code}\n--- end code ---"
    )
}

/// The sentence refusing an unapproved block, naming the way forward.
///
/// The options are named bare — `review`, `approve`, `force` — because the same words are
/// a CLI flag and an MCP parameter, and this sentence reaches both readers unchanged.
fn pending_review(fingerprint: &str) -> String {
    format!(
        "blocked pending review: this code (fingerprint {fingerprint}) has not been \
         approved on this machine. Ask for `review` to inspect it, then `approve` to \
         approve and run it; `force` runs it this once, and says so in the output."
    )
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
    } else if !capture.succeeded() {
        // A sandbox denial surfaces as the tool's own error — cargo's "failed to get
        // `tokio`" was really "no DNS in here" — and a reader who is not told about the
        // boundary debugs the project instead. Name it on the failures shaped like it.
        if let Some(hint) = sandbox_hint(&capture.output) {
            capture.output = format!("{}\n{hint}", capture.output);
        }
    }
    capture
}

/// The sentence appended to a failed block whose output looks like the sandbox, not the
/// project: a network fetch (denied while a block's own code runs) or a write outside the
/// block's own directory. `None` for every failure that does not carry those shapes — a
/// hint on an ordinary assertion failure would be noise.
fn sandbox_hint(output: &str) -> Option<&'static str> {
    const RESOLVER_SHAPED: &[&str] = &[
        "could not resolve host",
        "failure in name resolution",
        "name or service not known",
        "nodename nor servname provided",
        "failed to lookup address",
        "getaddrinfo",
        "enotfound",
        "eai_again",
        "dns error",
        "network is unreachable",
        "network is down",
    ];
    const WRITE_SHAPED: &[&str] = &["operation not permitted", "read-only file system"];

    let lowered = output.to_lowercase();
    if RESOLVER_SHAPED.iter().any(|mark| lowered.contains(mark)) {
        return Some(
            "note: dx runs a block's code in a sandbox with no network. Dependencies fetch \
             during setup — declare them with `deps=` — and everything else must already be \
             on disk.",
        );
    }
    if WRITE_SHAPED.iter().any(|mark| lowered.contains(mark)) {
        return Some(
            "note: the dx sandbox lets a block write only its own directory — $DX_SANDBOX, \
             where $HOME and $TMPDIR already point. /tmp and the document's folder stay \
             read-only.",
        );
    }
    None
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
    ///
    /// Approves what it runs: these tests exercise execution itself, and the approval
    /// gate has its own tests below.
    fn run_isolated(source: &str, label: &str) -> RunReport {
        let root = std::env::temp_dir().join(format!("dx-run-tests-{label}"));
        let _ = std::fs::create_dir_all(&root);
        run_document(
            source,
            &RunOptions {
                document_dir: root.clone(),
                cache_root: root.join("cache"),
                default_timeout: Duration::from_secs(60),
                approve: true,
                ..RunOptions::default()
            },
            &resolve::Nowhere,
        )
        .expect("document order never cycles")
    }

    /// Options over a fresh scratch cache with nothing approved.
    fn gate_options(label: &str) -> RunOptions {
        let root = std::env::temp_dir().join(format!("dx-gate-tests-{label}"));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::create_dir_all(&root);
        RunOptions {
            document_dir: root.clone(),
            cache_root: root.join("cache"),
            default_timeout: Duration::from_secs(60),
            ..RunOptions::default()
        }
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

    /// The field-report bug: a `cargo … | grep | head` pipeline whose first command hard-failed
    /// was recorded `ok`, because a pipeline's exit is its last command's. A failure anywhere
    /// in a pipeline is a failed block, never a success.
    #[test]
    fn a_failure_inside_a_pipeline_is_not_reported_as_success() {
        let source = "::code id=piped lang=bash run\nfalse | cat\n::end\n";
        let report = run_isolated(source, "pipefail");
        assert_eq!(report.runs[0].status, "error", "{}", report.runs[0].output);
        assert_ne!(report.runs[0].exit, 0);
        assert!(!report.all_succeeded());
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
                approve: true,
                ..RunOptions::default()
            },
            &Folder(root.clone()),
        )
        .expect("acyclic run");
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
                approve: true,
                ..RunOptions::default()
            },
            &resolve::Nowhere,
        )
        .expect("acyclic run");
        assert_eq!(report.runs.len(), 1);
        assert_eq!(report.runs[0].id, "b");
    }

    /// A document written backwards on purpose: `second` first, and a board stating
    /// `first -> second`.
    const EDGE_ORDERED: &str = "::code id=second lang=bash run hidden\necho ran-second\n::end\n\n\
::code id=first lang=bash run hidden\necho ran-first\n::end\n\n\
::board id=plan\n- first x=0 y=0 to=second\n- second x=0 y=200\n::end\n";

    #[test]
    fn follow_edges_runs_the_boards_order_not_the_documents() {
        let root = std::env::temp_dir().join("dx-run-tests-follow");
        let _ = std::fs::create_dir_all(&root);
        let report = run_document(
            EDGE_ORDERED,
            &RunOptions {
                document_dir: root.clone(),
                cache_root: root.join("cache"),
                approve: true,
                follow_board_edges: true,
                ..RunOptions::default()
            },
            &resolve::Nowhere,
        )
        .expect("acyclic run");
        let ids: Vec<&str> = report.runs.iter().map(|run| run.id.as_str()).collect();
        assert_eq!(ids, vec!["first", "second"]);
        assert!(report.all_succeeded());
    }

    #[test]
    fn without_follow_edges_a_board_changes_nothing_about_the_order() {
        let report = run_isolated(EDGE_ORDERED, "no-follow");
        let ids: Vec<&str> = report.runs.iter().map(|run| run.id.as_str()).collect();
        assert_eq!(ids, vec!["second", "first"], "default stays document order");
    }

    #[test]
    fn follow_edges_review_lists_blocks_in_the_order_they_would_run() {
        let mut options = gate_options("follow-review");
        options.review_only = true;
        options.follow_board_edges = true;
        let report = run_document(EDGE_ORDERED, &options, &resolve::Nowhere).expect("acyclic run");
        let ids: Vec<&str> = report.runs.iter().map(|run| run.id.as_str()).collect();
        assert_eq!(ids, vec!["first", "second"]);
        assert!(!report.changed, "review changed the document");
    }

    #[test]
    fn follow_edges_refuses_a_cycle_with_the_sentence() {
        let mut options = gate_options("follow-cycle");
        options.follow_board_edges = true;
        let sentence = run_document(
            "::code id=a lang=bash run hidden\necho a\n::end\n\n\
::code id=b lang=bash run hidden\necho b\n::end\n\n\
::board id=plan\n- a x=0 y=0 to=b\n- b x=0 y=200 to=a\n::end\n",
            &options,
            &resolve::Nowhere,
        )
        .expect_err("a cycle has no order");
        assert_eq!(
            sentence,
            "blocks a -> b -> a form a cycle; --follow-edges needs an order"
        );
    }

    #[test]
    fn only_with_follow_edges_is_not_refused_by_a_cycle_it_did_not_select() {
        // a and b form a cycle on the board; c is unrelated. `--only c` narrows the graph
        // before the order is computed, so the cycle cannot veto the selected block.
        let mut options = gate_options("only-follow-cycle");
        options.follow_board_edges = true;
        options.only = Some("c".to_string());
        options.approve = true;
        let report = run_document(
            "::code id=a lang=bash run hidden\necho a\n::end\n\n\
::code id=b lang=bash run hidden\necho b\n::end\n\n\
::code id=c lang=bash run hidden\necho c\n::end\n\n\
::board id=plan\n- a x=0 y=0 to=b\n- b x=0 y=200 to=a\n- c x=0 y=400\n::end\n",
            &options,
            &resolve::Nowhere,
        )
        .expect("an unselected cycle cannot veto --only");
        assert_eq!(report.runs.len(), 1);
        assert_eq!(report.runs[0].id, "c");
        assert_eq!(report.runs[0].status, "ok");
    }

    /// The real thing, end to end: a `lang=ts` block executes on the machine's own Node
    /// toolchain — npm installs `tsx` in setup, the annotated code runs offline, and the
    /// output folds into the document.
    #[test]
    fn a_typescript_block_executes_under_node() {
        if !(toolchain::have("node") && toolchain::have("npm")) {
            eprintln!("skipping: node/npm not installed");
            return;
        }
        let source =
            "::code id=t lang=ts run timeout=300\nconst n: number = 6 * 7;\nconsole.log(n);\n::end\n";
        let report = run_isolated(source, "typescript");
        assert_eq!(report.runs[0].status, "ok", "{}", report.runs[0].output);
        assert_eq!(report.runs[0].output, "42");
        assert!(report.source.contains("status=ok"));
    }

    /// One round trip per direct-toolchain language: compile in setup, run the artifact,
    /// capture the output. Each is guarded by its own toolchain's presence.
    #[test]
    fn compiled_language_blocks_round_trip_when_the_toolchain_exists() {
        let cases = [
            (
                "c",
                &["cc", "clang", "gcc"][..],
                "#include <stdio.h>\nint main(void) { printf(\"hello from c\"); return 0; }",
                "hello from c",
            ),
            (
                "cpp",
                &["c++", "clang++", "g++"][..],
                "#include <iostream>\nint main() { std::cout << \"hello from c++\"; }",
                "hello from c++",
            ),
            (
                "java",
                &["javac"][..],
                "public class Main {\n  public static void main(String[] args) {\n    System.out.println(\"hello from java\");\n  }\n}",
                "hello from java",
            ),
            (
                "swift",
                &["swiftc"][..],
                "print(\"hello from swift\")",
                "hello from swift",
            ),
        ];
        for (language, compilers, code, expected) in cases {
            let available = if language == "java" {
                plan::java_toolchain_present()
            } else {
                toolchain::first_available(compilers).is_some()
            };
            if !available {
                eprintln!("skipping {language}: no toolchain installed");
                continue;
            }
            let source =
                format!("::code id=hello lang={language} run timeout=300\n{code}\n::end\n");
            let report = run_isolated(&source, &format!("compiled-{language}"));
            assert_eq!(
                report.runs[0].status, "ok",
                "{language}: {}",
                report.runs[0].output
            );
            assert_eq!(report.runs[0].output, expected, "{language}");
        }
    }

    /// `deps=` on a language that cannot fetch libraries blocks with the sentence, and
    /// executes nothing.
    #[test]
    fn a_compiled_block_declaring_deps_is_blocked_with_the_sentence() {
        let source = "::code id=nope lang=c run deps=\"libcurl\"\nint main(){}\n::end\n";
        let report = run_isolated(source, "deps-refused");
        assert_eq!(report.runs[0].status, "blocked");
        assert!(report.runs[0].output.contains("deps="));
    }

    /// The field-report confusion: cargo's "failed to get `tokio`" was really "no DNS in the
    /// sandbox", and the agent debugged the project. A failure that looks like the boundary
    /// names the boundary.
    #[test]
    fn a_resolver_shaped_failure_names_the_sandbox() {
        let hint = sandbox_hint("curl: (6) Could not resolve host: index.crates.io")
            .expect("a resolver failure earns the hint");
        assert!(hint.contains("network"), "{hint}");
        assert!(hint.contains("deps="), "{hint}");
        assert!(
            sandbox_hint("error: failed to lookup address information").is_some(),
            "getaddrinfo failures are resolver-shaped too"
        );
        assert!(sandbox_hint("assertion failed: left == right").is_none());
    }

    #[test]
    fn a_write_denied_failure_names_the_write_grant() {
        let hint = sandbox_hint("touch: /tmp/probe: Operation not permitted")
            .expect("a denied write earns the hint");
        assert!(hint.contains("DX_SANDBOX"), "{hint}");
        assert!(hint.contains("HOME"), "{hint}");
        assert!(sandbox_hint("Read-only file system").is_some());
    }

    /// End to end on a machine with a boundary: a block that writes outside its own
    /// directory fails, and the failure explains the sandbox instead of impersonating a
    /// project defect.
    #[test]
    fn a_denied_write_explains_the_sandbox_in_the_recorded_output() {
        if confine::overridden() || !(cfg!(target_os = "macos") || cfg!(target_os = "linux")) {
            eprintln!("skipping: no boundary on this machine");
            return;
        }
        let source = "::code id=probe lang=bash run\necho x > /tmp/dx-hint-probe\n::end\n";
        let report = run_isolated(source, "write-hint");
        assert_eq!(report.runs[0].status, "error", "{}", report.runs[0].output);
        assert!(
            report.runs[0].output.contains("DX_SANDBOX"),
            "the record never named the sandbox: {}",
            report.runs[0].output
        );
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
                approve: true,
                ..RunOptions::default()
            },
            &resolve::Nowhere,
        )
        .expect("acyclic run");
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
        // The full digest: this value is the approval identity, and a truncated one is
        // a collision a hostile author could manufacture.
        assert_eq!(base.len(), 64);
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
        let report =
            run_document(source, &RunOptions::default(), &resolve::Nowhere).expect("acyclic run");
        assert_eq!(report.runs.len(), 1);
        assert_eq!(report.runs[0].status, "blocked");
        assert!(report.runs[0].output.contains("site/site.css"));
    }

    #[test]
    fn a_read_outside_the_folder_blocks_the_run() {
        let source = "::code id=check lang=python run reads=../secrets\nprint(1)\n::end\n";
        let report =
            run_document(source, &RunOptions::default(), &resolve::Nowhere).expect("acyclic run");
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

    #[test]
    fn an_unapproved_block_is_blocked_pending_review_with_the_way_forward() {
        let options = gate_options("plain");
        let report = run_document(
            "::code id=new lang=bash run\necho unreviewed\n::end\n",
            &options,
            &resolve::Nowhere,
        )
        .expect("acyclic run");
        assert_eq!(report.runs[0].status, "blocked");
        assert_eq!(report.runs[0].exit, BLOCKED_EXIT);
        assert!(report.runs[0].output.contains("blocked pending review"));
        assert!(report.runs[0].output.contains("`review`"));
        assert!(report.runs[0].output.contains("`approve`"));
        // Nothing ran and nothing was folded in: the document is exactly as it was.
        assert!(!report.changed);
        assert!(!report.source.contains("::output"));
    }

    #[test]
    fn only_plus_an_unapproved_block_is_blocked_with_the_sentence() {
        let mut options = gate_options("only");
        options.only = Some("b".to_string());
        let report = run_document(
            "::code id=a lang=bash run\necho a\n::end\n\n\
::code id=b lang=bash run\necho b\n::end\n",
            &options,
            &resolve::Nowhere,
        )
        .expect("acyclic run");
        assert_eq!(report.runs.len(), 1);
        assert_eq!(report.runs[0].id, "b");
        assert_eq!(report.runs[0].status, "blocked");
        assert!(report.runs[0].output.contains("blocked pending review"));
    }

    #[test]
    fn a_blocked_block_keeps_its_stale_output_untouched() {
        let options = gate_options("stale");
        let approved = run_document(
            "::code id=v lang=bash run\necho before\n::end\n",
            &RunOptions {
                approve: true,
                ..options.clone()
            },
            &resolve::Nowhere,
        )
        .expect("acyclic run");
        assert_eq!(approved.runs[0].status, "ok");

        // Editing the code invalidates the approval: the fingerprint changed.
        let edited = approved.source.replace("echo before", "echo after");
        let second = run_document(&edited, &options, &resolve::Nowhere).expect("acyclic run");
        assert_eq!(second.runs[0].status, "blocked");
        assert!(!second.changed);
        assert!(second.source.contains("before"), "stale output was touched");
        assert!(!second
            .source
            .contains("::output id=v-output for=v status=blocked"));
    }

    #[test]
    fn review_executes_nothing_records_nothing_and_shows_the_code() {
        let options = gate_options("review");
        let review = run_document(
            "::code id=peek lang=bash run\necho would-run\n::end\n",
            &RunOptions {
                review_only: true,
                ..options.clone()
            },
            &resolve::Nowhere,
        )
        .expect("acyclic run");
        assert_eq!(review.runs[0].status, "review");
        assert!(review.runs[0].output.contains("fingerprint "));
        assert!(review.runs[0].output.contains("not approved"));
        assert!(review.runs[0].output.contains("echo would-run"));
        assert!(!review.changed, "review changed the document");

        // Reading never writes: the review approved nothing, so a plain run still refuses.
        let after = run_document(
            "::code id=peek lang=bash run\necho would-run\n::end\n",
            &options,
            &resolve::Nowhere,
        )
        .expect("acyclic run");
        assert_eq!(after.runs[0].status, "blocked");
    }

    #[test]
    fn review_with_approve_or_force_is_refused_by_the_engine_itself() {
        // The rule lives here, not in the surfaces: any caller combining review with an
        // option review cannot honour gets the refusal, not a flag silently swallowed.
        let source = "::code id=x lang=bash run\necho conflict\n::end\n";
        for other in ["approve", "force"] {
            let mut options = gate_options(&format!("review-{other}"));
            options.review_only = true;
            match other {
                "approve" => options.approve = true,
                _ => options.force = true,
            }
            let sentence =
                run_document(source, &options, &resolve::Nowhere).expect_err("conflicting options");
            assert!(sentence.contains("records nothing"), "{sentence}");
        }
    }

    #[test]
    fn force_runs_unapproved_code_and_announces_the_bypass() {
        let options = gate_options("force");
        let report = run_document(
            "::code id=pushed lang=bash run\necho anyway\n::end\n",
            &RunOptions {
                force: true,
                ..options
            },
            &resolve::Nowhere,
        )
        .expect("acyclic run");
        assert_eq!(report.runs[0].status, "ok");
        assert!(report.runs[0].output.starts_with(FORCED_NOTICE));
        assert!(report.runs[0].output.contains("anyway"));
        // The notice is in the document's own record, not just the terminal report.
        assert!(report.source.contains(FORCED_NOTICE));
    }

    #[test]
    fn an_approval_survives_across_runs() {
        let options = gate_options("survive");
        let approved = run_document(
            "::code id=keep lang=bash run\necho kept\n::end\n",
            &RunOptions {
                approve: true,
                ..options.clone()
            },
            &resolve::Nowhere,
        )
        .expect("acyclic run");
        assert_eq!(approved.runs[0].status, "ok");

        // Strip the run record so the ledger alone must answer, then run plain.
        let source = "::code id=keep lang=bash run\necho kept\n::end\n";
        let again = run_document(source, &options, &resolve::Nowhere).expect("acyclic run");
        assert_eq!(again.runs[0].status, "ok");
        assert!(!again.runs[0].output.contains(FORCED_NOTICE));
    }

    /// A document carrying its own successful run record, hash and all — which is what
    /// every committed `.dx` looks like, and what a hostile one is trivially made to look
    /// like, since the fingerprint is a pure function of content its author controls.
    fn document_with_a_matching_run_record(code: &str) -> String {
        let body = format!("echo {code}");
        let hash = fingerprint("bash", &body, &[], &[]);
        format!(
            "::code id=forged lang=bash run\n{body}\n::end\n\n\
::output id=forged-output for=forged status=ok exit=0 hash={hash}\n{code}\n::end\n"
        )
    }

    #[test]
    fn a_run_record_in_the_document_does_not_approve_its_own_code() {
        let options = gate_options("forged-record");
        let report = run_document(
            &document_with_a_matching_run_record("forged-approval"),
            &options,
            &resolve::Nowhere,
        )
        .expect("acyclic run");
        // Not "skipped": the cached skip would hide unreviewed code behind its own record.
        assert_eq!(report.runs[0].status, "blocked");
        assert!(report.runs[0].output.contains("blocked pending review"));
        assert_eq!(report.executed(), 0);
        assert!(!report.changed);
    }

    #[test]
    fn force_over_a_document_supplied_run_record_still_announces_the_bypass() {
        let options = gate_options("forged-force");
        let report = run_document(
            &document_with_a_matching_run_record("forged-force"),
            &RunOptions {
                force: true,
                ..options
            },
            &resolve::Nowhere,
        )
        .expect("acyclic run");
        assert_eq!(report.runs[0].status, "ok");
        assert!(
            report.runs[0].output.starts_with(FORCED_NOTICE),
            "a bypass must announce itself: {}",
            report.runs[0].output
        );
        assert!(report.source.contains(FORCED_NOTICE));
    }

    #[test]
    fn review_records_nothing_even_for_a_block_it_must_refuse() {
        let mut options = gate_options("review-refusal");
        options.review_only = true;
        let source = "::code id=needy lang=bash run reads=missing.txt\necho hi\n::end\n";
        let report = run_document(source, &options, &resolve::Nowhere).expect("acyclic run");
        assert_eq!(report.runs.len(), 1);
        assert_eq!(report.runs[0].status, "review");
        assert!(report.runs[0].output.contains("missing.txt"));
        // Reading never writes: no ::output was folded in, so the document is unchanged.
        assert!(!report.changed, "review changed the document");
        assert!(!report.source.contains("::output"));
        assert_eq!(report.source, source);
    }

    #[test]
    fn an_unresolved_listing_is_refused_without_an_output_in_review() {
        let mut options = gate_options("review-unresolved");
        options.review_only = true;
        let source = "::code id=ref lang=bash run src=gone.sh\n::end\n";
        let report = run_document(source, &options, &resolve::Nowhere).expect("acyclic run");
        assert_eq!(report.runs[0].status, "review");
        assert!(!report.changed);
        assert!(!report.source.contains("::output"));
    }
}
