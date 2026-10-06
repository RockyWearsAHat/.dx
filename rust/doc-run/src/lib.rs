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
//! its own directory plus what its reviewed `writes=` grant names, reach no network —
//! described in full in [`confine`]. A machine that cannot impose that boundary does not
//! run the block; it reports it as blocked and says why.
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

#![warn(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::all)]

pub mod approvals;
pub mod confine;
pub mod plan;
pub mod process;
pub mod toolchain;

mod live;
mod live_actions;
mod live_proxy;
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

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use doc_core::digest::sha256_hex;
use doc_core::format::{parse, stringify};
use doc_core::model::{runner_for_language, Block, Document};
use doc_core::resolve::{self, Resolver};

use confine::Grant;
use plan::parse_deps;
use process::Capture;

/// Seconds a block may run when it does not set its own `timeout`.
pub const DEFAULT_TIMEOUT_SECONDS: u64 = 600;

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

/// The line stamped into the output of an approved `confine=host` block.
///
/// The per-block, reviewed counterpart of [`confine::UNCONFINED_NOTICE`]: the block ran
/// outside the sandbox because its header asked to and that header was approved with the
/// code, and the record says so.
pub const HOST_NOTICE: &str = "--- ran on the host: this block declares confine=host \
     (reviewed), so it had your own permissions ---";

/// How to run a document.
#[derive(Debug, Clone)]
pub struct RunOptions {
    /// Directory the document lives in; blocks run here so relative paths resolve.
    ///
    /// Readable, and — this is the point of the sandbox — not writable, except for the
    /// folders a block's own `writes=` grant names, which review sees because the grant
    /// is part of the fingerprint. A block opens the spreadsheet next to the document; it
    /// does not get to replace it.
    pub document_dir: PathBuf,
    /// Root of the per-block working directories and the shared toolchain caches.
    pub cache_root: PathBuf,
    /// Timeout for blocks that do not set their own.
    pub default_timeout: Duration,
    /// A timeout that overrides every block's own `timeout=` — `dx run --timeout S`. `None`
    /// (the default) leaves each block to its own header, and the blocks without one to
    /// [`RunOptions::default_timeout`]: the header governs unless the caller says otherwise.
    pub timeout_override: Option<Duration>,
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
            timeout_override: None,
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
    /// `ok`, `error`, `interrupted`, `skipped`, `blocked`, or `review`.
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
    /// One entry per runnable block, in run order — document order, unless
    /// [`RunOptions::follow_board_edges`] reordered them — even when independent blocks
    /// executed concurrently and finished in another order.
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
            .filter(|run| run.status == "ok" || run.status == "error" || run.status == INTERRUPTED)
            .count()
    }
}

/// Run every runnable code block in `source` and return the updated document.
///
/// Blocks execute in document order, so a later block sees files an earlier one wrote.
/// Independent blocks overlap: consecutive blocks run concurrently unless a later one
/// declares a `reads=` path an earlier one's `writes=` covers, and a block with no `reads=`
/// at all runs alone, in its place (see `order::waves`). At most `DX_RUN_JOBS` blocks
/// execute at once — by default half the machine's parallelism, capped at four — and
/// `DX_RUN_JOBS=1` is the plain serial run. Results are reported and folded in run order
/// whatever order the executions finish in, so the document comes out the same either way.
/// [`RunOptions::follow_board_edges`] orders them by the document's board edges instead —
/// there an edge that defers a block lets later blocks (on a board or not) run before it,
/// so only the stated edges order side effects, not document position; a block starts once
/// its edge predecessors finish, beside any other ready block. The order is still
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

    // `only` narrows the set here, ahead of the edge sort, so a cycle among blocks the
    // caller did not select cannot veto the one they did.
    let runnable: Vec<usize> = document
        .blocks
        .iter()
        .enumerate()
        .filter(|(_, block)| runnable_runner(block).is_some() && selected(block, options))
        .map(|(index, _)| index)
        .collect();
    let (ordered, predecessors) = if options.follow_board_edges {
        let (ordered, constraints) = order::edge_schedule(&document, &runnable)?;
        let predecessors = edge_predecessors(&ordered, &constraints);
        (ordered, predecessors)
    } else {
        let predecessors = wave_predecessors(&document, &runnable);
        (runnable, predecessors)
    };

    // Each position in `ordered` fills its own slot, so results are reported and folded in
    // run order however the concurrent executions happen to finish.
    let mut slots: Vec<Slot> = vec![Slot::default(); ordered.len()];
    schedule(
        &ordered,
        &predecessors,
        run_jobs(),
        |position, slot: &mut Slot| {
            prepare(
                ordered[position],
                &document,
                &hydrated,
                &unresolved,
                resolver,
                &ledger,
                options,
                slot,
            )
        },
        |job: &Job<'_>| run_job(job, options),
        |job, (capture, elapsed), slot: &mut Slot| finish(job, capture, elapsed, slot),
        &mut slots,
    );
    let mut runs: Vec<BlockRun> = Vec::new();
    let mut outputs: Vec<(String, Block)> = Vec::new();
    for slot in slots {
        runs.extend(slot.runs);
        outputs.extend(slot.outputs);
    }

    let updated = fold_outputs(&document, &runs, &outputs);
    let rendered = stringify(&updated);
    Ok(RunReport {
        changed: rendered != stringify(&document),
        source: rendered,
        runs,
    })
}

/// What one runnable block produced: its report entry and, when it ran or was refused,
/// the `::output` block to fold in.
#[derive(Default, Clone)]
struct Slot {
    runs: Vec<BlockRun>,
    outputs: Vec<(String, Block)>,
}

/// One block cleared to execute: everything [`execute`] needs, decided before it starts.
struct Job<'a> {
    block: &'a Block,
    runner: &'static str,
    deps: Vec<String>,
    writes: Vec<String>,
    read_paths: Vec<String>,
    host: bool,
    approved: bool,
    fingerprint: String,
}

/// How many blocks may execute at once: `DX_RUN_JOBS`, else half the machine's
/// parallelism capped at four, and never fewer than one. `DX_RUN_JOBS=1` is the serial run.
fn run_jobs() -> usize {
    let parallelism = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    jobs_from(std::env::var("DX_RUN_JOBS").ok().as_deref(), parallelism)
}

/// [`run_jobs`] over its inputs: an unreadable `DX_RUN_JOBS` falls back to the default.
fn jobs_from(requested: Option<&str>, parallelism: usize) -> usize {
    match requested.and_then(|value| value.trim().parse::<usize>().ok()) {
        Some(jobs) => jobs.max(1),
        None => (parallelism / 2).clamp(1, 4),
    }
}

/// Document order's predecessors, as positions in `runnable`: every block of a wave waits
/// for every block of the wave before it (see [`order::waves`]).
fn wave_predecessors(document: &Document, runnable: &[usize]) -> Vec<Vec<usize>> {
    let footprints: Vec<(usize, Option<order::Footprint>)> = runnable
        .iter()
        .enumerate()
        .map(|(position, &index)| {
            let block = &document.blocks[index];
            // A header the run will refuse anyway keeps document order: it runs alone.
            let footprint = match (declared_read_paths(&block.reads), declared_writes(block)) {
                (Ok(reads), Ok(writes)) => Some(order::Footprint { reads, writes }),
                _ => None,
            };
            (position, footprint)
        })
        .collect();
    let mut predecessors = vec![Vec::new(); runnable.len()];
    let mut previous: Vec<usize> = Vec::new();
    for wave in order::waves(&footprints) {
        for &position in &wave {
            predecessors[position].clone_from(&previous);
        }
        previous = wave;
    }
    predecessors
}

/// Edge order's predecessors, as positions in `ordered`: a block waits for exactly the
/// blocks with a direct edge constraint into it.
fn edge_predecessors(
    ordered: &[usize],
    constraints: &std::collections::HashSet<(usize, usize)>,
) -> Vec<Vec<usize>> {
    let position: std::collections::HashMap<usize, usize> = ordered
        .iter()
        .enumerate()
        .map(|(position, &index)| (index, position))
        .collect();
    let mut predecessors = vec![Vec::new(); ordered.len()];
    for (before, after) in constraints {
        if let (Some(&before), Some(&after)) = (position.get(before), position.get(after)) {
            predecessors[after].push(before);
        }
    }
    for list in &mut predecessors {
        list.sort_unstable();
    }
    predecessors
}

/// Run `ordered` with at most `jobs` executions at once, each starting once all its
/// `predecessors` (positions in `ordered`) have finished.
///
/// `prepare` decides a ready block on this thread — refusals, review, the approval gate,
/// and the cache all finish there — and hands back a [`Job`] only when the block must
/// execute; `execute` runs on a scoped thread; `finish` records the result on this thread.
/// Among ready blocks the earliest position goes first, so with `jobs` at one this is
/// exactly the serial run, and edge order's earliest-ready rule is kept.
fn schedule<'a, T: Send>(
    ordered: &[usize],
    predecessors: &[Vec<usize>],
    jobs: usize,
    mut prepare: impl FnMut(usize, &mut Slot) -> Option<Job<'a>>,
    execute: impl Fn(&Job<'a>) -> T + Sync,
    mut finish: impl FnMut(Job<'a>, T, &mut Slot),
    slots: &mut [Slot],
) {
    let count = ordered.len();
    let mut waiting: Vec<usize> = predecessors.iter().map(Vec::len).collect();
    let mut successors: Vec<Vec<usize>> = vec![Vec::new(); count];
    for (position, list) in predecessors.iter().enumerate() {
        for &before in list {
            successors[before].push(position);
        }
    }
    // Keyed by document index: with follow-edges the earliest ready block by document
    // position goes next, and in document order position and index agree.
    let mut ready: std::collections::BTreeSet<(usize, usize)> = (0..count)
        .filter(|&position| waiting[position] == 0)
        .map(|position| (ordered[position], position))
        .collect();
    let mut release = |position: usize, ready: &mut std::collections::BTreeSet<(usize, usize)>| {
        for &after in &successors[position] {
            waiting[after] -= 1;
            if waiting[after] == 0 {
                ready.insert((ordered[after], after));
            }
        }
    };

    let execute = &execute;
    std::thread::scope(|scope| {
        let (sender, receiver) = std::sync::mpsc::channel();
        let mut running = 0;
        let mut done = 0;
        while done < count {
            while running < jobs.max(1) {
                let Some((_, position)) = ready.pop_first() else {
                    break;
                };
                match prepare(position, &mut slots[position]) {
                    Some(job) => {
                        running += 1;
                        let sender = sender.clone();
                        scope.spawn(move || {
                            let result =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    execute(&job)
                                }));
                            let _ = sender.send((position, job, result));
                        });
                    }
                    None => {
                        done += 1;
                        release(position, &mut ready);
                    }
                }
            }
            if running == 0 {
                // Nothing executing and nothing ready: the predecessors were acyclic by
                // construction, so this only ends a finished run.
                if ready.is_empty() {
                    break;
                }
                continue;
            }
            let (position, job, result) = receiver
                .recv()
                .expect("a running block always reports back");
            running -= 1;
            match result {
                Ok(result) => finish(job, result, &mut slots[position]),
                Err(panic) => std::panic::resume_unwind(panic),
            }
            done += 1;
            release(position, &mut ready);
        }
    });
}

/// Decide one ready block on the scheduling thread: everything up to execution.
///
/// Refusals, review, the approval gate, and a still-current cached result are recorded
/// into `slot` here and return `None`; a block that must execute returns its [`Job`].
#[allow(clippy::too_many_arguments)]
fn prepare<'a>(
    index: usize,
    document: &Document,
    hydrated: &'a Document,
    unresolved: &[resolve::Unresolved],
    resolver: &dyn Resolver,
    ledger: &approvals::Ledger,
    options: &RunOptions,
    slot: &mut Slot,
) -> Option<Job<'a>> {
    let Slot { runs, outputs } = slot;
    let runner = runnable_runner(&document.blocks[index])?;

    // Hydration edits blocks in place and only ever appends, so the indices agree.
    let block = &hydrated.blocks[index];
    if let Some(problem) = unresolved.iter().find(|entry| entry.block == block.id) {
        refuse(block, &problem.sentence, options, runs, outputs);
        return None;
    }

    let deps = parse_deps(&block.deps);
    let writes = match declared_writes(block) {
        Ok(writes) => writes,
        Err(sentence) => {
            refuse(block, &sentence, options, runs, outputs);
            return None;
        }
    };
    let host = match declared_host(block) {
        Ok(host) => host,
        Err(sentence) => {
            refuse(block, &sentence, options, runs, outputs);
            return None;
        }
    };
    // Approval names the *declared* paths, never a directory's current expansion —
    // a file appearing under a declared folder is new data, not a new power.
    let read_paths = match declared_read_paths(&block.reads) {
        Ok(paths) => paths,
        Err(sentence) => {
            refuse(block, &sentence, options, runs, outputs);
            return None;
        }
    };
    let material = approval_material(block);
    let approval = approval_fingerprint(runner, &material, &deps, &read_paths, &writes, host);
    let reads = match declared_reads(block, resolver, &writes, Some(&options.document_dir)) {
        Ok(reads) => reads,
        Err(sentence) if options.review_only => {
            // A grant that cannot be satisfied must not hide the code: review still
            // shows what would run, with the problem named above it.
            runs.push(BlockRun {
                id: block.id.clone(),
                language: block.language.clone(),
                status: "review".to_string(),
                exit: 0,
                output: format!(
                    "{sentence}\n{}",
                    review_text(
                        &material,
                        &approval,
                        ledger.is_approved(&approval),
                        &read_paths,
                        &writes,
                        host,
                        &options.document_dir,
                    )
                ),
                duration_ms: 0,
            });
            return None;
        }
        Err(sentence) => {
            refuse(block, &sentence, options, runs, outputs);
            return None;
        }
    };
    let fingerprint = fingerprint(
        runner,
        &material,
        &deps,
        &reads,
        &writes,
        host,
        block.timeout,
    );
    let existing = existing_output(document, index, &block.id);

    // Review mode: show what would run without executing — and without recording
    // anything, because reading never writes.
    if options.review_only {
        runs.push(BlockRun {
            id: block.id.clone(),
            language: block.language.clone(),
            status: "review".to_string(),
            exit: 0,
            output: review_text(
                &material,
                &approval,
                ledger.is_approved(&approval),
                &read_paths,
                &writes,
                host,
                &options.document_dir,
            ),
            duration_ms: 0,
        });
        return None;
    }

    if options.approve {
        if let Err(sentence) = ledger.approve(&approval) {
            runs.push(BlockRun {
                id: block.id.clone(),
                language: block.language.clone(),
                status: "blocked".to_string(),
                exit: BLOCKED_EXIT,
                output: sentence,
                duration_ms: 0,
            });
            return None;
        }
    }

    // Approval is this machine's own record and nothing else. The document's `::output`
    // block cannot vouch for the code above it: it is content the same hand wrote, and
    // its `hash=` is computable by whoever authored the block. Approval names the code
    // and its powers — not the current text of its `reads=` files — so editing an input
    // re-runs reviewed code instead of re-opening review of a program nobody changed.
    let approved = ledger.is_approved(&approval);

    // The gate stands ahead of the cache, so a document that arrives carrying a matching
    // run record is still reviewed rather than quietly accepted as already proven. The
    // refusal names its way forward, and the block's stale output is left as it was.
    if !approved && !options.force {
        runs.push(BlockRun {
            id: block.id.clone(),
            language: block.language.clone(),
            status: "blocked".to_string(),
            exit: BLOCKED_EXIT,
            output: pending_review(&approval),
            duration_ms: 0,
        });
        return None;
    }

    if !options.force
        && existing.is_some_and(|output| output.hash == fingerprint && recorded_pass(output))
    {
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
        return None;
    }

    Some(Job {
        block,
        runner,
        deps,
        writes,
        read_paths,
        host,
        approved,
        fingerprint,
    })
}

/// Execute one prepared block, off the scheduling thread.
fn run_job(job: &Job<'_>, options: &RunOptions) -> (Capture, u64) {
    let Job {
        block,
        runner,
        ref deps,
        ref writes,
        ref read_paths,
        host,
        approved,
        ref fingerprint,
    } = *job;
    // Reachable unapproved only through `--force`, which is the bypass that must say so.
    let bypassed = !approved;
    let started = Instant::now();
    // The host run is part of what was approved, so only an approved block gets it: a
    // block forced past review runs inside the sandbox like any other.
    let mut capture = execute(
        runner,
        block,
        deps,
        &Powers {
            writes,
            reads: read_paths,
            host: host && approved,
        },
        fingerprint,
        options,
    );
    if bypassed {
        capture.output = format!("{FORCED_NOTICE}\n{}", capture.output);
    }
    let elapsed = started.elapsed().as_millis() as u64;
    (capture, elapsed)
}

/// Record one executed block's result into its slot, on the scheduling thread.
fn finish(job: Job<'_>, capture: Capture, elapsed: u64, slot: &mut Slot) {
    let Slot { runs, outputs } = slot;
    let Job {
        block, fingerprint, ..
    } = job;
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
/// Paths are comma-separated and obey the reference path law; a path may name a file or
/// a folder. A folder expands to every file under it ([`Resolver::files_under`] — sorted,
/// hidden entries and build caches left out), minus anything under the block's own
/// `writes=` grant: what a block writes is its result, and a result that joined the
/// fingerprint would stale the block's own verdict forever. A path the law refuses, or
/// one the resolver can produce neither as file nor folder, is an error sentence — a
/// fingerprint that silently omitted a missing input would let the record claim
/// "no changes" about content it never saw, which is the lie `reads=` exists to prevent.
fn declared_reads(
    block: &Block,
    resolver: &dyn Resolver,
    writes: &[String],
    document_dir: Option<&Path>,
) -> Result<Vec<(String, String)>, String> {
    let mut reads = Vec::new();
    for confined in declared_read_paths(&block.reads)? {
        if let Some(text) = resolver.file(&confined) {
            reads.push((confined, text));
        } else if let Some(digest) = binary_read(resolver, &confined) {
            reads.push((confined, digest));
        } else if let Some(tree) = resolver.files_under(&confined) {
            let granted = |path: &str| {
                writes
                    .iter()
                    .any(|w| path == w || path.starts_with(&format!("{w}/")))
            };
            reads.extend(tree.into_iter().filter(|(path, _)| !granted(path)));
        } else if let Some(found) =
            document_dir.and_then(|dir| read_from_root(&confined, resolver, dir))
        {
            reads.extend(found);
        } else if document_dir.is_some_and(|dir| on_disk(&confined, dir)) {
            return Err(format!(
                "{confined} is on disk but could not be read — the block declares it with \
                 `reads=`, and its content is part of what decides whether the recorded \
                 output is still current. A `.dx` pointer whose version this workspace's \
                 store does not hold reads this way: `dx sync` (or restoring \
                 .doc/repo.dxcp) brings it back."
            ));
        } else {
            return Err(format!(
                "{confined} could not be read here — the block declares it with `reads=`, \
                 and its content is part of what decides whether the recorded output is \
                 still current. Check the path against the document's folder and the \
                 repository root."
            ));
        }
    }
    Ok(reads)
}

/// A declared file that is not text — a built binary, an image — read as its bytes' digest.
///
/// A folder walk already contributes such a file this way ([`Resolver::files_under`]); a
/// file named on its own was refused instead, because the text read is the only one tried
/// first. Only bytes that are not UTF-8 answer here: a text file the resolver could not
/// produce (a `.dx` pointer the store cannot resolve) must stay a refusal, never be
/// fingerprinted as the pointer it stands behind.
fn binary_read(resolver: &dyn Resolver, path: &str) -> Option<String> {
    let bytes = resolver.binary(path)?;
    std::str::from_utf8(&bytes)
        .is_err()
        .then(|| sha256_hex(&bytes))
}

/// Whether a declared read names something on disk, under the document's folder or the
/// workspace root — what tells "the path is wrong" apart from "the path is there and its
/// content could not be produced", which need different fixes.
fn on_disk(confined: &str, document_dir: &Path) -> bool {
    if document_dir.join(confined).exists() {
        return true;
    }
    !confined.starts_with("..")
        && climb_to_root(document_dir)
            .is_some_and(|up| document_dir.join(up).join(confined).exists())
}

/// Marks a fingerprint input that was resolved against the workspace root.
const ROOT_BASE: &str = "root:";

/// The `../` path that climbs from `document_dir` to the workspace root, or `None` when
/// the document already lives at the root (nothing to fall back to).
fn climb_to_root(document_dir: &Path) -> Option<String> {
    let doc = document_dir
        .canonicalize()
        .unwrap_or_else(|_| document_dir.to_path_buf());
    let root = confine::repo_root(document_dir);
    let depth = doc.strip_prefix(&root).ok()?.components().count();
    (depth > 0).then(|| "../".repeat(depth))
}

/// A `reads=` path the document's folder does not hold, found under the workspace root
/// instead. Each input is keyed `root:<path>` so the fingerprint records which base was
/// used: the same text under another base is a different input. A path that itself climbs
/// (`..`) never falls back — it already chose its own base.
fn read_from_root(
    confined: &str,
    resolver: &dyn Resolver,
    document_dir: &Path,
) -> Option<Vec<(String, String)>> {
    if confined == ".." || confined.starts_with("../") {
        return None;
    }
    let up = climb_to_root(document_dir)?;
    let via = format!("{up}{confined}");
    if let Some(text) = resolver.file(&via).or_else(|| binary_read(resolver, &via)) {
        return Some(vec![(format!("{ROOT_BASE}{confined}"), text)]);
    }
    let tree = resolver.files_under(&via)?;
    Some(
        tree.into_iter()
            .map(|(path, text)| {
                let rest = path.strip_prefix(&up).unwrap_or(&path);
                (format!("{ROOT_BASE}{rest}"), text)
            })
            .collect(),
    )
}

/// The directory a declared read path is joined onto: the document's folder, or the
/// workspace root when the folder does not hold the path but the root does.
fn read_anchor(path: &str, document_dir: &Path) -> PathBuf {
    let doc = document_dir
        .canonicalize()
        .unwrap_or_else(|_| document_dir.to_path_buf());
    if !(doc.join(path).exists() || path == ".." || path.starts_with("../")) {
        let root = confine::repo_root(document_dir);
        if root != doc && root.join(path).exists() {
            return root;
        }
    }
    doc
}

/// The lawful paths of a `reads=` declaration, confined and in declaration order.
///
/// This is the path list both identities agree on: [`fingerprint`] pairs each with its
/// current text, [`approval_fingerprint`] takes the paths alone — so the two can never
/// disagree about *which* files a block declared.
///
/// Unlike write paths, read paths may use `..` to reference parent directories and siblings,
/// as long as they stay within the repository scope.
fn declared_read_paths(reads: &str) -> Result<Vec<String>, String> {
    reads
        .split(',')
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(|path| {
            resolve::confined_with_parent_refs(path)
                .map(str::to_string)
                .ok_or_else(|| {
                    format!(
                        "{path} is not a path this block may declare — a `reads=` path must be \
                     relative, may not start with / ~ or contain absolute components, and must \
                     not contain escape sequences like :// or backslashes."
                    )
                })
        })
        .collect()
}

/// The text a block's fingerprint and review actually cover.
///
/// A `capture` block's `target=`, `setup=`, and `actions` decide what it does exactly as
/// much as its script body does — where it reaches, what it runs to get there before the
/// script ever evaluates, and whether that body is raw JavaScript or the `actions`
/// shorthand compiled into JavaScript behind the reviewer's back — so they are prepended
/// the same way [`fingerprint`] prepends a `writes=` grant: reviewing the code reviews the
/// whole power, not just the part written as a script. Toggling `actions` with the body
/// left untouched must re-open review too, or an approved raw-JS fingerprint would stay
/// "approved" once reinterpreted as a wholly different compiled program. Every other
/// block's `target` and `setup` are always empty and `actions` is always false, so this
/// returns their body completely unchanged — no fingerprint computed before this field
/// existed moves.
fn approval_material(block: &Block) -> std::borrow::Cow<'_, str> {
    if block.target.is_empty() && block.setup.is_empty() && !block.actions {
        std::borrow::Cow::Borrowed(block.text.as_str())
    } else {
        std::borrow::Cow::Owned(format!(
            "target={}\u{1f}setup={}\u{1f}actions={}\u{1f}{}",
            block.target, block.setup, block.actions, block.text
        ))
    }
}

/// Fingerprint the inputs that decide a block's output: runner, code, dependencies, the
/// current text of every file the block declares it reads, and the write grant it asks for.
///
/// This is the *staleness* identity — the `hash=` a run records — and any of its inputs
/// changing means the recorded output no longer describes what the block would do now.
///
/// The full digest, untruncated: a truncated hash is one a hostile author could collide —
/// a benign block the reader approves and a payload sharing its shortened fingerprint.
///
/// The grant is *prepended*, never appended: the material's tail is a `reads=` file's
/// text, which the document's author controls, so a suffix could be forged into an
/// ungranted block's material. No runner name starts with `writes=`, so a block with a
/// grant and a block without one can never share material — approving one never approves
/// the other. A `confine=host` declaration is prepended the same way, so approving the
/// code approves the host run and a block toggled to the host re-opens review.
fn fingerprint(
    runner: &str,
    code: &str,
    deps: &[String],
    reads: &[(String, String)],
    writes: &[String],
    host: bool,
    timeout: u32,
) -> String {
    let mut material = format!("{runner}\u{1f}{}\u{1f}{code}", deps.join(","));
    for (path, text) in reads {
        material.push('\u{1f}');
        material.push_str(path);
        material.push('\u{1f}');
        material.push_str(text);
    }
    if !writes.is_empty() {
        material = format!("writes={}\u{1e}{material}", writes.join(","));
    }
    if host {
        material = format!("confine=host\u{1e}{material}");
    }
    if timeout > 0 {
        material = format!("timeout={}\u{1e}{material}", timeout);
    }
    sha256_hex(material.as_bytes())
}

/// The identity an approval names: the code and its powers.
///
/// Runner, dependencies, the exact code, the *paths* it declares it reads, and the
/// folders its `writes=` grant opens — everything a reviewer weighs when deciding
/// whether this program may run. The current text of the `reads=` files is deliberately
/// absent: that text is the block's *data*, and editing data stales the recorded output
/// (the run [`fingerprint`] changes) without re-opening review of a program nobody
/// touched — reviewed code re-runs over new inputs, which is what a verify block is for.
/// Changing *which* files are read edits the block's header, so it lands here.
///
/// The material opens with its own domain tag, so no run fingerprint's material can
/// collide into an approval's; the grant prepends for the same reason as in
/// [`fingerprint`].
fn approval_fingerprint(
    runner: &str,
    code: &str,
    deps: &[String],
    read_paths: &[String],
    writes: &[String],
    host: bool,
) -> String {
    let mut material = format!(
        "approval\u{1e}{runner}\u{1f}{}\u{1f}{code}\u{1f}reads={}",
        deps.join(","),
        read_paths.join(",")
    );
    if !writes.is_empty() {
        material = format!("writes={}\u{1e}{material}", writes.join(","));
    }
    if host {
        material = format!("confine=host\u{1e}{material}");
    }
    sha256_hex(material.as_bytes())
}

/// Whether a block declares the host run (`confine=host`), the one value `confine=` takes.
///
/// Any other value is refused with a sentence rather than read as the sandbox: a typo in a
/// header that asks for more power must not silently run with less, nor the other way round.
fn declared_host(block: &Block) -> Result<bool, String> {
    match block.confine.trim() {
        "" => Ok(false),
        "host" => Ok(true),
        other => Err(format!(
            "confine={other} is not a value this block may declare — the one accepted \
             value is `confine=host`, which runs the reviewed block on the host with your \
             own permissions instead of inside the sandbox."
        )),
    }
}

/// The folders a block declares it may write (`writes=`), validated but not yet resolved.
///
/// Paths are comma-separated and obey the reference path law, with two more refusals the
/// law does not cover: a control character (which could forge the fingerprint's grant
/// section), and the `.doc` store — a block that could rewrite the packs could make the
/// resolver hand back a different document, silently. The grant is part of the block's
/// fingerprint, so review always sees the current grant and an edit to it re-opens review.
fn declared_writes(block: &Block) -> Result<Vec<String>, String> {
    let mut writes = Vec::new();
    for path in block
        .writes
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        let Some(confined) = resolve::confined(path) else {
            return Err(format!(
                "{path} is not a path this block may be granted — a `writes=` folder stays \
                 inside the document's own folder, relative and walking downward."
            ));
        };
        if confined.chars().any(char::is_control) {
            return Err(format!(
                "a `writes=` path may not contain control characters: {path:?}"
            ));
        }
        if confined == ".doc" || confined.starts_with(".doc/") {
            return Err(
                "the .doc store cannot be granted with `writes=` — a block that could \
                 rewrite the packs could change what every document here says."
                    .to_string(),
            );
        }
        writes.push(confined.to_string());
    }
    Ok(writes)
}

/// Resolve a validated write grant against the document's folder, refusing escapes.
///
/// The path law already refused `..` and absolute paths; what it cannot see is a symlink
/// inside the folder pointing out of it, so each granted path — after creating it if it
/// does not exist yet — must canonicalize to somewhere under the folder's own canonical
/// path. Creation checks the deepest existing ancestor *first*, so a symlinked parent
/// cannot make `create_dir_all` build directories outside the folder either.
fn granted_writes(writes: &[String], document_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let root = document_dir
        .canonicalize()
        .map_err(|error| format!("the document's folder could not be resolved: {error}"))?;
    let mut granted = Vec::new();
    for path in writes {
        let stated = document_dir.join(path);
        let mut existing = stated.clone();
        while !existing.exists() {
            match existing.parent() {
                Some(parent) => existing = parent.to_path_buf(),
                None => break,
            }
        }
        let escape = format!(
            "writes={path} resolves outside the document's folder — a symlink on the way \
             takes the grant somewhere the review never saw, so the block was not run."
        );
        if !existing
            .canonicalize()
            .map_err(|error| format!("writes={path} could not be resolved: {error}"))?
            .starts_with(&root)
        {
            return Err(escape);
        }
        if !stated.exists() {
            std::fs::create_dir_all(&stated)
                .map_err(|error| format!("writes={path} could not be created: {error}"))?;
        }
        let resolved = stated
            .canonicalize()
            .map_err(|error| format!("writes={path} could not be resolved: {error}"))?;
        if !resolved.starts_with(&root) {
            return Err(escape);
        }
        granted.push(resolved);
    }
    Ok(granted)
}

/// Resolve `reads=` paths to their canonical form for the sandbox grant.
///
/// Paths are joined with the document directory and normalized. They will be added to the
/// sandbox's readable roots, allowing the block to read them. Explicit `reads=` entries may
/// resolve anywhere on the machine (outside the repository), but are validated to refuse
/// paths under sensitive directories (.ssh, .gnupg, .aws, Library/Keychains) and .env* files.
/// Existence validation happens in [`declared_reads`] which uses the resolver.
fn granted_reads(reads: &[String], document_dir: &Path) -> Result<Vec<PathBuf>, String> {
    // Determine the home directory for secret path validation
    let home = std::env::var_os("HOME").map(PathBuf::from);

    let mut granted = Vec::new();

    for path in reads {
        // Join the path with its base: the document's folder, or the workspace root when
        // only the root holds it (see [`read_anchor`])
        let base = read_anchor(path, document_dir);
        let resolved = base.join(path);

        // Normalize the path by removing any `.` or `..` components using the path APIs
        // For paths that exist, canonicalize; for those that don't, normalize manually
        let normalized = if resolved.exists() {
            resolved
                .canonicalize()
                .map_err(|error| format!("reads={path} could not be resolved: {error}"))?
        } else {
            // Normalize non-existent paths by collecting components
            let mut normalized_path = base.clone();
            for component in path.split('/').filter(|s| !s.is_empty()) {
                if component == ".." {
                    normalized_path.pop();
                } else if component != "." {
                    normalized_path.push(component);
                }
            }
            normalized_path
        };

        // Refuse .env* files
        if let Some(filename) = normalized.file_name() {
            let filename_str = filename.to_string_lossy();
            if filename_str == ".env" || filename_str.starts_with(".env.") {
                return Err(format!(
                    "reads={path} cannot be granted — .env files contain secrets"
                ));
            }
        }

        // Refuse paths under sensitive directories
        if let Some(home_dir) = &home {
            let sensitive_dirs = vec![
                home_dir.join(".ssh"),
                home_dir.join(".gnupg"),
                home_dir.join(".aws"),
                home_dir.join("Library/Keychains"),
            ];
            for secret_dir in sensitive_dirs {
                if normalized.starts_with(&secret_dir) {
                    return Err(format!(
                        "reads={path} cannot be granted — the path is in a sensitive directory"
                    ));
                }
            }
        }

        granted.push(normalized);
    }
    Ok(granted)
}

/// Check if a declared read path is outside the repository scope.
fn is_read_outside_repo(path: &str, document_dir: &Path) -> bool {
    let canonical_doc_dir = document_dir
        .canonicalize()
        .unwrap_or_else(|_| document_dir.to_path_buf());
    let resolved = canonical_doc_dir.join(path);
    let normalized = if resolved.exists() {
        resolved.canonicalize().unwrap_or(resolved)
    } else {
        let mut normalized_path = canonical_doc_dir.clone();
        for component in path.split('/').filter(|s| !s.is_empty()) {
            if component == ".." {
                normalized_path.pop();
            } else if component != "." {
                normalized_path.push(component);
            }
        }
        normalized_path
    };

    let repo_scope = confine::read_scope(document_dir);
    !repo_scope.iter().any(|scope| normalized.starts_with(scope))
}

/// Resolve a read path to its absolute form for display.
fn resolve_read_path_for_display(path: &str, document_dir: &Path) -> String {
    let canonical_doc_dir = read_anchor(path, document_dir);
    let resolved = canonical_doc_dir.join(path);
    let normalized = if resolved.exists() {
        resolved.canonicalize().unwrap_or(resolved)
    } else {
        let mut normalized_path = canonical_doc_dir.clone();
        for component in path.split('/').filter(|s| !s.is_empty()) {
            if component == ".." {
                normalized_path.pop();
            } else if component != "." {
                normalized_path.push(component);
            }
        }
        normalized_path
    };
    normalized.to_string_lossy().to_string()
}

/// What review mode reports for one block: the exact code that would run (hydrated, so
/// `src=` listings show the file's current text), its fingerprint, the read and write grants it
/// asks for, and where it stands with the approval gate.
fn review_text(
    code: &str,
    fingerprint: &str,
    approved: bool,
    reads: &[String],
    writes: &[String],
    host: bool,
    document_dir: &Path,
) -> String {
    let standing = if approved {
        "approved — a plain run executes this code"
    } else {
        "not approved — a run with approve records it and executes"
    };
    let mut grants = Vec::new();
    if !reads.is_empty() {
        let read_displays: Vec<String> = reads
            .iter()
            .map(|path| {
                let absolute = resolve_read_path_for_display(path, document_dir);
                if is_read_outside_repo(path, document_dir) {
                    format!("{} → {} (outside repository)", path, absolute)
                } else {
                    format!("{} → {}", path, absolute)
                }
            })
            .collect();
        grants.push(format!(
            "reads {} — approving grants this code read access to these paths",
            read_displays.join(", ")
        ));
    }
    if !writes.is_empty() {
        grants.push(format!(
            "writes {} — approving grants this code write access to these folders",
            writes.join(", ")
        ));
    }
    if host {
        grants.push(
            "confine=host — approving runs this code on the host with your own permissions, \
             outside the sandbox"
                .to_string(),
        );
    }
    let grant_text = if grants.is_empty() {
        String::new()
    } else {
        format!("{}\n", grants.join("\n"))
    };
    format!(
        "fingerprint {fingerprint}\napproval {standing}\n{grant_text}--- code ---\n{code}\n--- end code ---"
    )
}

/// Record this machine's approval of `block` as the edit that produced it left it —
/// because a local edit *is* the review.
///
/// The gate exists so code nobody on this machine has looked at cannot run. A person or
/// agent who just rewrote a block here is exactly the reviewer it wants: they are looking
/// at the code they typed, the same way the editing surface's run control treats the
/// click beside a block as the review of that block. Editing surfaces call this after
/// saving, so the next run — or the next live read — executes without asking again.
/// Adoption and sync must never call it: bringing a stranger's document into the
/// workspace is not an edit, and its code stays unreviewed.
///
/// Returns the approval fingerprint recorded, or `None` when there is nothing to
/// approve: a block that is not runnable code, one whose code lives in a `src=` file
/// (the run resolves that text; the edit did not touch it), or one declaring an unlawful
/// `reads=`/`writes=` path — the run will refuse that block with its own sentence, and
/// recording a decision about it here would approve a block that can never run.
///
/// # Errors
/// Returns a sentence when the ledger itself could not be written — a decision the
/// caller made must never be dropped silently.
pub fn approve_edited_block(block: &Block, cache_root: &Path) -> Result<Option<String>, String> {
    let Some(runner) = runnable_runner(block) else {
        return Ok(None);
    };
    if !block.src.is_empty() {
        return Ok(None);
    }
    let deps = parse_deps(&block.deps);
    let (Ok(read_paths), Ok(writes), Ok(host)) = (
        declared_read_paths(&block.reads),
        declared_writes(block),
        declared_host(block),
    ) else {
        return Ok(None);
    };
    let approval = approval_fingerprint(runner, &block.text, &deps, &read_paths, &writes, host);
    approvals::Ledger::at(cache_root).approve(&approval)?;
    Ok(Some(approval))
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

/// What a block's header grants it at run time: the folders it may write, the paths it
/// declared it reads, and whether its approved `confine=host` takes it out of the sandbox.
struct Powers<'a> {
    writes: &'a [String],
    reads: &'a [String],
    host: bool,
}

/// Execute one block, returning a capture even when nothing could be started.
fn execute(
    runner: &str,
    block: &Block,
    deps: &[String],
    powers: &Powers<'_>,
    fingerprint: &str,
    options: &RunOptions,
) -> Capture {
    let Powers {
        writes,
        reads,
        host,
    } = *powers;
    if execution_disabled() {
        return blocked("execution is disabled (DX_NO_EXEC is set); no code was run");
    }

    let timeout = block_timeout(block, options);

    // `capture` reaches a live `target=`, on purpose — see [`live`]'s module doc for why
    // that is not the `plan`/`confine` pipeline every other runner goes through below.
    if runner == "capture" {
        return match granted_writes(writes, &options.document_dir) {
            Ok(granted) => live::execute(block, &granted, &options.document_dir, timeout),
            Err(message) => blocked(&message),
        };
    }

    let dirs = plan::Dirs {
        block: options.cache_root.join(runner).join(fingerprint),
        toolchains: options.cache_root.join("toolchains"),
    };
    let prepared = match plan::build(runner, &block.text, deps, &dirs) {
        Ok(prepared) => prepared,
        Err(message) => return blocked(&message),
    };

    // Installing declared libraries is the one phase that may reach the network, and it is
    // confined in every other way — an `npm install` runs the package's own scripts. The
    // block's `writes=` grant is not part of it: an install with the network and the
    // document's folders writable would be a fetch that can edit the project, and no
    // install needs that.
    let writable = vec![dirs.block.clone(), dirs.toolchains.clone()];
    // What a block may read: the repository its document belongs to (plus that repository's
    // other worktrees, if it has any — see `confine::read_scope`), the run caches, and any
    // additional paths declared with `reads=`. Everything else of the user's is outside the
    // boundary — see `confine`.
    let mut readable = confine::read_scope(&options.document_dir);
    readable.push(options.cache_root.clone());

    // Add any explicitly declared read paths to the readable scope
    let declared_read_paths = match granted_reads(reads, &options.document_dir) {
        Ok(paths) => paths,
        Err(message) => return blocked(&message),
    };
    readable.extend(declared_read_paths.clone());

    let installing = Grant::offline(writable.clone())
        .reading(readable.clone())
        .with_network();
    if let Err(message) = workdir::prepare(&dirs.block, &prepared, &installing, timeout) {
        return blocked(&message);
    }

    // The block's own code: the same directories writable — plus the folders its reviewed
    // `writes=` grant names, resolved and escape-checked — and no network at all.
    let granted = match granted_writes(writes, &options.document_dir) {
        Ok(granted) => granted,
        Err(message) => return blocked(&message),
    };
    let mut writable = writable;
    writable.extend(granted);
    // An approved `confine=host` block runs as the reader would have run it: no sandbox,
    // and the reader's own environment — `HOME`, `TMPDIR`, `USER` unchanged — with only the
    // `DX_*` variables every block gets. The sandbox's redirections exist for the sandbox.
    let command = if host {
        with_dx_variables(prepared.run.clone(), block, &dirs, timeout).on_host()
    } else {
        let run = home_in_block(&prepared.run, block, &dirs, timeout);
        match confine::confine(&run, &Grant::offline(writable).reading(readable)) {
            Ok(command) => command,
            Err(message) => return blocked(&message),
        }
    };

    let mut capture = process::run(&command, &options.document_dir, timeout);
    if host {
        capture.output = format!("{HOST_NOTICE}\n{}", capture.output);
    } else if confine::overridden() {
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

/// How long `block` may run: the caller's explicit override when there is one, else the
/// block's own `timeout=`, else the run's default. A slow host gate (`timeout=900`) gets its
/// 900 s from its header alone — no command-line flag is needed (dx report-3d99fcfc).
fn block_timeout(block: &Block, options: &RunOptions) -> Duration {
    if let Some(timeout) = options.timeout_override {
        timeout
    } else if block.timeout > 0 {
        Duration::from_secs(u64::from(block.timeout))
    } else {
        options.default_timeout
    }
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
            "note: the dx sandbox scopes a block to its project. Reads reach the \
             document's own repository, the run caches, and the system toolchains — never \
             the rest of the machine. Writes land only in the block's own directory — \
             $DX_SANDBOX, where $HOME and $TMPDIR already point — plus the folders granted \
             with `writes=` on the block (`writes=target,generated`): folders inside the \
             document's own folder, created if missing, and the grant is part of the \
             fingerprint, so it is reviewed exactly like the code. It grants folders, \
             never loose files — a tool that rewrites one beside the document (cargo's \
             Cargo.lock) needs the flag that tells it not to (`cargo test --locked`).",
        );
    }
    None
}

/// Get Rust toolchain environment variables to pass into the sandbox.
///
/// Derives RUSTUP_HOME and CARGO_HOME from the current environment or defaults,
/// and reads the default toolchain from ~/.rustup/settings.toml if available.
/// These must be passed into the sandbox because home_in_block redirects HOME to
/// the block's directory, which would otherwise hide the real toolchain locations.
fn rust_toolchain_env() -> Vec<(String, String)> {
    use std::path::PathBuf;

    let mut env = Vec::new();

    // Determine the actual home directory before it gets redirected.
    let home = match std::env::var_os("HOME") {
        Some(h) => PathBuf::from(h),
        None => return env, // No HOME, skip toolchain env setup.
    };

    // Get RUSTUP_HOME, defaulting to ~/.rustup
    let rustup_home = std::env::var("RUSTUP_HOME")
        .unwrap_or_else(|_| home.join(".rustup").to_string_lossy().into_owned());
    env.push(("RUSTUP_HOME".to_string(), rustup_home.clone()));

    // Get CARGO_HOME, defaulting to ~/.cargo
    let cargo_home = std::env::var("CARGO_HOME")
        .unwrap_or_else(|_| home.join(".cargo").to_string_lossy().into_owned());
    env.push(("CARGO_HOME".to_string(), cargo_home));

    // Try to read the default toolchain from ~/.rustup/settings.toml
    let settings_path = home.join(".rustup/settings.toml");
    if let Ok(content) = std::fs::read_to_string(&settings_path) {
        for line in content.lines() {
            if let Some(value) = line.strip_prefix("default_toolchain = ") {
                // The value is quoted: "stable" or "1.xx.x", etc.
                let trimmed = value.trim().trim_matches('"');
                if !trimmed.is_empty() {
                    env.push(("RUSTUP_TOOLCHAIN".to_string(), trimmed.to_string()));
                    break;
                }
            }
        }
    }

    env
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
    timeout: Duration,
) -> process::CommandSpec {
    let block_dir = dirs.block.to_string_lossy().into_owned();
    let redirected = run
        .clone()
        .with_env("HOME", block_dir.clone())
        .with_env("TMPDIR", block_dir.clone())
        .with_env("TEMP", block_dir)
        .with_env(
            "XDG_CACHE_HOME",
            dirs.toolchains.to_string_lossy().into_owned(),
        );
    let mut cmd = with_dx_variables(redirected, block, dirs, timeout);

    // Pass Rust toolchain environment variables into the sandbox.
    for (key, value) in rust_toolchain_env() {
        cmd = cmd.with_env(&key, value);
    }

    cmd
}

/// The `DX_*` variables every block gets, sandboxed or on the host: its id, its own
/// scratch directory (`$DX_SANDBOX`), and the whole seconds dx will let it run
/// (`$DX_BLOCK_TIMEOUT`: the override, else its `timeout=`, else the default — rounded up),
/// so a wrapper with a limit of its own (pcrun) can take dx's instead of silently
/// cutting the run shorter.
fn with_dx_variables(
    run: process::CommandSpec,
    block: &Block,
    dirs: &plan::Dirs,
    timeout: Duration,
) -> process::CommandSpec {
    let seconds = timeout.as_secs() + u64::from(timeout.subsec_nanos() > 0);
    run.with_env("DX_BLOCK_ID", block.id.clone())
        .with_env("DX_SANDBOX", dirs.block.to_string_lossy().into_owned())
        .with_env("DX_BLOCK_TIMEOUT", seconds.to_string())
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
        signaled: false,
    }
}

/// Status of a block whose process was killed (signal or timeout): not a verdict.
const INTERRUPTED: &str = "interrupted";

/// Whether an exit code and output read as a kill: a shell's 128 + signal, with the
/// shell's own "Terminated" / "Killed" marker.
fn looks_killed(exit: i32, output: &str) -> bool {
    (128..=159).contains(&exit) && (output.contains("Terminated") || output.contains("Killed"))
}

/// Whether a recorded `::output` is a pass: only passes are cache hits. A failure re-runs
/// every time, and an interrupted run (signal or timeout, including an older document's
/// `error` with exit 128..=159 and a kill marker) was never a verdict.
fn recorded_pass(output: &Block) -> bool {
    output.status == "ok" && output.exit == 0
}

/// Classify a capture into the status recorded on the `::output` block.
fn status_of(capture: &Capture) -> String {
    if capture.exit == BLOCKED_EXIT {
        "blocked".to_string()
    } else if capture.succeeded() {
        "ok".to_string()
    } else if capture.timed_out || capture.signaled || looks_killed(capture.exit, &capture.output) {
        INTERRUPTED.to_string()
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
    fn review_prints_the_write_grant_beside_the_code_it_widens() {
        let mut options = gate_options("review-grant");
        options.review_only = true;
        let source = "::code id=build lang=bash run writes=target,gen\nmake\n::end\n\n\
::code id=plain lang=bash run\necho hi\n::end\n";
        let report = run_document(source, &options, &resolve::Nowhere).expect("acyclic run");
        assert!(
            report.runs[0].output.contains("writes target, gen"),
            "the reviewer must see what approval grants: {}",
            report.runs[0].output
        );
        assert!(
            !report.runs[1].output.contains("writes "),
            "an ungranted block claims no grant: {}",
            report.runs[1].output
        );
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

    /// Run `source` from a fresh folder holding `files`, with `DX_RUN_JOBS` set to `jobs`
    /// for the run, returning the report and its wall time.
    fn run_with_jobs(
        source: &str,
        label: &str,
        files: &[&str],
        jobs: &str,
        follow_board_edges: bool,
    ) -> (RunReport, Duration, PathBuf) {
        let root = std::env::temp_dir().join(format!("dx-run-tests-jobs-{label}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scene");
        for file in files {
            std::fs::write(root.join(file), *file).expect("fixture");
        }
        let _env = env_lock();
        let previous = std::env::var("DX_RUN_JOBS").ok();
        std::env::set_var("DX_RUN_JOBS", jobs);
        let started = Instant::now();
        let report = run_document(
            source,
            &RunOptions {
                document_dir: root.clone(),
                cache_root: root.join("cache"),
                default_timeout: Duration::from_secs(60),
                approve: true,
                follow_board_edges,
                ..RunOptions::default()
            },
            &Folder(root.clone()),
        );
        let elapsed = started.elapsed();
        match previous {
            Some(value) => std::env::set_var("DX_RUN_JOBS", value),
            None => std::env::remove_var("DX_RUN_JOBS"),
        }
        (report.expect("acyclic run"), elapsed, root)
    }

    /// Three blocks that each sleep a second and declare disjoint `reads=`.
    const THREE_SLEEPERS: &str =
        "::code id=a lang=bash run reads=a.txt\nsleep 1; echo a\n::end\n\n\
::code id=b lang=bash run reads=b.txt\nsleep 1; echo b\n::end\n\n\
::code id=c lang=bash run reads=c.txt\nsleep 1; echo c\n::end\n";

    fn ids(report: &RunReport) -> Vec<&str> {
        report.runs.iter().map(|run| run.id.as_str()).collect()
    }

    #[test]
    fn independent_declared_blocks_run_concurrently() {
        let files = ["a.txt", "b.txt", "c.txt"];
        let (parallel, elapsed, _) = run_with_jobs(THREE_SLEEPERS, "par3", &files, "3", false);
        assert!(parallel.all_succeeded(), "{:?}", parallel.runs);
        assert_eq!(parallel.executed(), 3);
        assert!(
            elapsed < Duration::from_millis(2500),
            "three one-second blocks with DX_RUN_JOBS=3 took {elapsed:?}"
        );
        assert_eq!(
            ids(&parallel),
            vec!["a", "b", "c"],
            "reported in document order"
        );

        let (serial, elapsed, _) = run_with_jobs(THREE_SLEEPERS, "par1", &files, "1", false);
        assert!(serial.all_succeeded(), "{:?}", serial.runs);
        assert!(
            elapsed >= Duration::from_secs(3),
            "DX_RUN_JOBS=1 must run them one at a time, took {elapsed:?}"
        );
        assert_eq!(ids(&serial), vec!["a", "b", "c"]);
        // The document comes out identical whichever way the blocks were scheduled.
        assert_eq!(parallel.source, serial.source);
    }

    #[test]
    fn a_block_reading_another_blocks_writes_runs_after_it() {
        let source = "::code id=make lang=bash run reads=seed.txt writes=out\n\
sleep 1; echo made > out/made.txt\n::end\n\n\
::code id=use lang=bash run reads=out/made.txt\ncat out/made.txt\n::end\n";
        let (report, _, root) = run_with_jobs(source, "raw", &["seed.txt"], "4", false);
        assert_eq!(ids(&report), vec!["make", "use"]);
        assert_eq!(report.runs[0].status, "ok", "{}", report.runs[0].output);
        assert_eq!(report.runs[1].status, "ok", "{}", report.runs[1].output);
        assert_eq!(
            report.runs[1].output, "made",
            "the reader saw the writer's file"
        );
        assert!(root.join("out/made.txt").exists());
    }

    #[test]
    fn blocks_without_reads_stay_serial_and_ordered() {
        // Each appends to one log; were any two overlapped, the slow first block would
        // land its line after the quick ones.
        let source =
            "::code id=first lang=bash run writes=log\nsleep 1; echo first >> log/order\n::end\n\n\
::code id=second lang=bash run writes=log\necho second >> log/order\n::end\n\n\
::code id=third lang=bash run writes=log\necho third >> log/order\n::end\n";
        let (report, elapsed, root) = run_with_jobs(source, "undeclared", &[], "4", false);
        assert!(report.all_succeeded(), "{:?}", report.runs);
        assert_eq!(ids(&report), vec!["first", "second", "third"]);
        assert_eq!(
            std::fs::read_to_string(root.join("log/order")).expect("log"),
            "first\nsecond\nthird\n"
        );
        assert!(elapsed >= Duration::from_secs(1));
    }

    #[test]
    fn follow_edges_starts_a_block_after_its_predecessor_and_overlaps_the_rest() {
        // Document order is use, make, aside; the board states make -> use. aside is on
        // no board, so it runs beside make; use waits for make's file.
        let source = "::code id=use lang=bash run hidden\ncat out/made.txt\n::end\n\n\
::code id=make lang=bash run hidden writes=out\nsleep 1; echo made > out/made.txt\n::end\n\n\
::code id=aside lang=bash run hidden\nsleep 1; echo aside\n::end\n\n\
::board id=plan\n- make x=0 y=0 to=use\n- use x=0 y=200\n::end\n";
        let (report, elapsed, _) = run_with_jobs(source, "edges", &[], "4", true);
        assert!(report.all_succeeded(), "{:?}", report.runs);
        assert_eq!(ids(&report), vec!["make", "use", "aside"]);
        assert_eq!(report.runs[1].output, "made");
        assert!(
            elapsed < Duration::from_millis(1900),
            "aside overlaps make, took {elapsed:?}"
        );
    }

    #[test]
    fn run_jobs_reads_the_variable_and_defaults_to_half_the_cores_capped_at_four() {
        assert_eq!(jobs_from(Some("1"), 16), 1);
        assert_eq!(jobs_from(Some(" 6 "), 2), 6);
        assert_eq!(jobs_from(Some("0"), 16), 1);
        assert_eq!(jobs_from(Some("lots"), 16), 4);
        assert_eq!(jobs_from(None, 16), 4);
        assert_eq!(jobs_from(None, 6), 3);
        assert_eq!(jobs_from(None, 1), 1);
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
        assert_eq!(report.runs[0].status, "interrupted");
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
        let base = fingerprint("python", "print(1)", &[], &[], &[], false, 0);
        assert_ne!(
            base,
            fingerprint("python", "print(2)", &[], &[], &[], false, 0)
        );
        assert_ne!(
            base,
            fingerprint("python", "print(1)", &["rich".into()], &[], &[], false, 0)
        );
        assert_ne!(
            base,
            fingerprint("node", "print(1)", &[], &[], &[], false, 0)
        );
        // The full digest: this value is the approval identity, and a truncated one is
        // a collision a hostile author could manufacture.
        assert_eq!(base.len(), 64);
    }

    #[test]
    fn fingerprints_change_with_declared_reads() {
        let base = fingerprint("python", "print(1)", &[], &[], &[], false, 0);
        let read = fingerprint(
            "python",
            "print(1)",
            &[],
            &[("site.css".into(), "body{}".into())],
            &[],
            false,
            0,
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
                &[],
                false,
                0,
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
                &[],
                false,
                0,
            )
        );
    }

    #[test]
    fn fingerprints_change_with_the_write_grant_and_cannot_be_forged_onto_one() {
        let bare = fingerprint("bash", "make", &[], &[], &[], false, 0);
        let granted = fingerprint("bash", "make", &[], &[], &["target".into()], false, 0);
        assert_ne!(bare, granted, "a grant is part of what review approves");
        assert_ne!(
            granted,
            fingerprint(
                "bash",
                "make",
                &[],
                &[],
                &["target".into(), "gen".into()],
                false,
                0
            ),
            "a wider grant is a different approval"
        );
        // The forgery `reads=` could otherwise mount: a file whose *text* replays the
        // grant section. Prepending the grant keeps the materials distinct, because no
        // runner name begins with `writes=`.
        let forged = fingerprint(
            "bash",
            "make",
            &[],
            &[("writes".into(), "target".into())],
            &[],
            false,
            0,
        );
        assert_ne!(
            granted, forged,
            "an ungranted block can never share a granted print"
        );
    }

    #[test]
    fn fingerprints_change_with_timeout() {
        let base = fingerprint("python", "print(1)", &[], &[], &[], false, 0);
        let with_timeout = fingerprint("python", "print(1)", &[], &[], &[], false, 300);
        assert_ne!(
            base, with_timeout,
            "a timeout is part of what review approves"
        );
        assert_ne!(
            with_timeout,
            fingerprint("python", "print(1)", &[], &[], &[], false, 600),
            "a different timeout is a different approval"
        );
    }

    #[test]
    fn a_write_grant_stays_inside_the_document_folder_and_never_names_the_store() {
        let block = |writes: &str| Block {
            kind: "code".into(),
            id: "b".into(),
            language: "bash".into(),
            run: true,
            writes: writes.into(),
            text: "true".into(),
            ..Block::default()
        };
        assert_eq!(
            declared_writes(&block("target, generated/site")).expect("lawful grant"),
            vec!["target".to_string(), "generated/site".to_string()]
        );
        assert!(declared_writes(&block("../escape")).is_err());
        assert!(declared_writes(&block("/tmp")).is_err());
        assert!(declared_writes(&block(".doc")).is_err());
        assert!(declared_writes(&block(".doc/repo.dxcp")).is_err());
        assert!(declared_writes(&block("a\u{1e}b")).is_err());
    }

    #[test]
    fn a_symlink_cannot_carry_a_write_grant_out_of_the_folder() {
        let scratch = std::env::temp_dir().join("dx-run-tests-writes-symlink");
        let _ = std::fs::remove_dir_all(&scratch);
        let outside = scratch.join("outside");
        let folder = scratch.join("doc");
        std::fs::create_dir_all(&outside).expect("outside");
        std::fs::create_dir_all(&folder).expect("folder");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, folder.join("out")).expect("symlink");
        #[cfg(unix)]
        {
            let refused = granted_writes(&["out".into()], &folder)
                .expect_err("a symlink out of the folder is refused");
            assert!(
                refused.contains("outside the document's folder"),
                "{refused}"
            );
            // And a path *through* the symlink cannot make dx create outside either.
            let through = granted_writes(&["out/deeper".into()], &folder)
                .expect_err("a path through the symlink is refused");
            assert!(
                through.contains("outside the document's folder"),
                "{through}"
            );
            assert!(
                !outside.join("deeper").exists(),
                "nothing was created outside"
            );
        }
        let granted =
            granted_writes(&["build/nested".into()], &folder).expect("a missing folder is created");
        assert!(folder.join("build/nested").is_dir());
        assert_eq!(granted.len(), 1);
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
        let before = declared_reads(&block, &provided, &[], None).expect("resolves");
        let recorded = fingerprint("python", &block.text, &[], &before, &[], false, 0);

        let mut edited = resolve::Provided::new();
        edited.add_file("site.css", "body{color:red}");
        let after = declared_reads(&block, &edited, &[], None).expect("resolves");
        assert_ne!(
            recorded,
            fingerprint("python", &block.text, &[], &after, &[], false, 0)
        );
    }

    /// A resolver whose folder walk answers, standing in for the CLI's.
    struct Walked(Vec<(String, String)>);

    impl Resolver for Walked {
        fn file(&self, _path: &str) -> Option<String> {
            None
        }
        fn document(&self, _path: &str) -> Option<String> {
            None
        }
        fn files_under(&self, path: &str) -> Option<Vec<(String, String)>> {
            (path == "data").then(|| self.0.clone())
        }
    }

    #[test]
    fn a_reads_folder_expands_to_its_files_and_stales_with_them() {
        let block = Block {
            id: "check".into(),
            language: "bash".into(),
            run: true,
            reads: "data".into(),
            text: "cat data/a.txt".into(),
            ..Block::default()
        };
        let before = Walked(vec![("data/a.txt".into(), "one".into())]);
        let reads = declared_reads(&block, &before, &[], None).expect("resolves");
        assert_eq!(reads, vec![("data/a.txt".to_string(), "one".to_string())]);
        let recorded = fingerprint("bash", &block.text, &[], &reads, &[], false, 0);

        // A file appearing under the declared folder is a change the record must see.
        let grown = Walked(vec![
            ("data/a.txt".into(), "one".into()),
            ("data/b.txt".into(), "two".into()),
        ]);
        let after = declared_reads(&block, &grown, &[], None).expect("resolves");
        assert_ne!(
            recorded,
            fingerprint("bash", &block.text, &[], &after, &[], false, 0)
        );

        // But not a change to the block's powers: approval names the declared path.
        let paths = declared_read_paths(&block.reads).expect("lawful");
        assert_eq!(
            approval_fingerprint("bash", &block.text, &[], &paths, &[], false),
            approval_fingerprint("bash", &block.text, &[], &paths, &[], false)
        );
        assert_eq!(paths, vec!["data".to_string()]);
    }

    #[test]
    fn a_reads_folder_leaves_out_what_the_block_writes() {
        let block = Block {
            id: "check".into(),
            language: "bash".into(),
            run: true,
            reads: "data".into(),
            text: "true".into(),
            ..Block::default()
        };
        let walked = Walked(vec![
            ("data/a.txt".into(), "input".into()),
            ("data/out/result.txt".into(), "changes every run".into()),
        ]);
        let reads =
            declared_reads(&block, &walked, &["data/out".to_string()], None).expect("resolves");
        assert_eq!(reads, vec![("data/a.txt".to_string(), "input".to_string())]);
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
    fn approval_names_the_code_and_its_powers_never_the_data() {
        let base = approval_fingerprint("python", "print(1)", &[], &["a.css".into()], &[], false);
        // The same program over the same declared paths is one approval — a `reads=`
        // file's text is not an input here at all, which is the whole point.
        assert_eq!(
            base,
            approval_fingerprint("python", "print(1)", &[], &["a.css".into()], &[], false)
        );
        assert_ne!(
            base,
            approval_fingerprint("python", "print(2)", &[], &["a.css".into()], &[], false)
        );
        assert_ne!(
            base,
            approval_fingerprint("python", "print(1)", &[], &["b.css".into()], &[], false)
        );
        assert_ne!(
            base,
            approval_fingerprint(
                "python",
                "print(1)",
                &[],
                &["a.css".into()],
                &["target".into()],
                false
            )
        );
        assert_ne!(
            base,
            approval_fingerprint(
                "python",
                "print(1)",
                &["rich".into()],
                &["a.css".into()],
                &[],
                false
            )
        );
        assert_eq!(base.len(), 64, "the full digest is the approval identity");
    }

    #[test]
    fn an_edited_input_re_runs_reviewed_code_without_re_opening_review() {
        let options = gate_options("input-edit");
        let source = "::code id=check lang=bash run reads=site.css\necho verified\n::end\n";
        let mut provided = resolve::Provided::new();
        provided.add_file("site.css", "body{}");
        let first = run_document(
            source,
            &RunOptions {
                approve: true,
                ..options.clone()
            },
            &provided,
        )
        .expect("acyclic run");
        assert_eq!(first.runs[0].status, "ok");

        // The input changes; the code does not. The recorded output is stale (its hash
        // covered the old text), and the reviewed program re-runs over the new data —
        // it is not sent back through review, because nobody edited it.
        let mut edited = resolve::Provided::new();
        edited.add_file("site.css", "body{color:red}");
        let second = run_document(&first.source, &options, &edited).expect("acyclic run");
        assert_eq!(
            second.runs[0].status, "ok",
            "reviewed code runs over new data: {}",
            second.runs[0].output
        );
        assert_eq!(second.executed(), 1, "stale output re-ran, not skipped");
    }

    #[test]
    fn a_local_edit_is_the_review() {
        let options = gate_options("edit-review");
        let block = Block {
            kind: "code".into(),
            id: "typed".into(),
            language: "bash".into(),
            run: true,
            text: "echo typed here".into(),
            ..Block::default()
        };
        approve_edited_block(&block, &options.cache_root)
            .expect("ledger writable")
            .expect("a runnable block records an approval");

        // The block arrives in a document exactly as the edit left it: a plain run
        // executes without asking again, because the hand that typed it reviewed it.
        let report = run_document(
            "::code id=typed lang=bash run\necho typed here\n::end\n",
            &options,
            &resolve::Nowhere,
        )
        .expect("acyclic run");
        assert_eq!(report.runs[0].status, "ok", "{}", report.runs[0].output);

        // Nothing to approve: prose, and code whose text lives in a `src=` file the
        // edit did not touch.
        let prose = Block {
            kind: "paragraph".into(),
            id: "p".into(),
            text: "words".into(),
            ..Block::default()
        };
        assert!(approve_edited_block(&prose, &options.cache_root)
            .expect("ledger")
            .is_none());
        let sourced = Block {
            src: "script.sh".into(),
            ..block
        };
        assert!(approve_edited_block(&sourced, &options.cache_root)
            .expect("ledger")
            .is_none());
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
        let hash = fingerprint("bash", &body, &[], &[], &[], false, 0);
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

    /// A folder resolver that also walks folders, standing in for the CLI's.
    struct Tree(PathBuf);

    impl Resolver for Tree {
        fn file(&self, path: &str) -> Option<String> {
            std::fs::read_to_string(self.0.join(path)).ok()
        }
        fn document(&self, path: &str) -> Option<String> {
            self.file(path)
        }
        fn files_under(&self, path: &str) -> Option<Vec<(String, String)>> {
            let dir = self.0.join(path);
            let mut files: Vec<(String, String)> = std::fs::read_dir(&dir)
                .ok()?
                .flatten()
                .filter_map(|entry| {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    let text = std::fs::read_to_string(entry.path()).ok()?;
                    Some((format!("{path}/{name}"), text))
                })
                .collect();
            files.sort();
            Some(files)
        }
    }

    fn nested_repo(label: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("dx-reads-root-{label}"));
        let _ = std::fs::remove_dir_all(&root);
        let doc_dir = root.join("sub/dir");
        std::fs::create_dir_all(&doc_dir).expect("doc folder");
        std::fs::create_dir_all(root.join(".git")).expect("repo marker");
        std::fs::create_dir_all(root.join("crates")).expect("crates");
        std::fs::write(root.join("crates/a.rs"), "fn a() {}").expect("fixture");
        (root, doc_dir)
    }

    #[test]
    fn a_reads_path_the_folder_lacks_resolves_against_the_workspace_root() {
        let (root, doc_dir) = nested_repo("resolves");
        let block = Block {
            id: "check".into(),
            language: "bash".into(),
            run: true,
            reads: "crates".into(),
            text: "echo hi".into(),
            ..Block::default()
        };
        let resolver = Tree(doc_dir.clone());
        let reads = declared_reads(&block, &resolver, &[], Some(&doc_dir)).expect("resolves");
        // The fingerprint input records the base it came from.
        assert_eq!(
            reads,
            vec![("root:crates/a.rs".to_string(), "fn a() {}".to_string())]
        );
        // Without the root fallback the same declaration is refused.
        assert!(declared_reads(&block, &resolver, &[], None).is_err());
        // The sandbox grant follows the same base.
        let granted = granted_reads(&["crates".to_string()], &doc_dir).expect("grant");
        assert_eq!(
            granted,
            vec![root.join("crates").canonicalize().expect("canon")]
        );
        // A folder that holds the path itself wins over the root.
        std::fs::create_dir_all(doc_dir.join("crates")).expect("local crates");
        std::fs::write(doc_dir.join("crates/b.rs"), "fn b() {}").expect("local");
        let local = declared_reads(&block, &resolver, &[], Some(&doc_dir)).expect("local");
        assert_eq!(local[0].0, "crates/b.rs");
    }

    /// A folder resolver that reads text, reads bytes, and — like the CLI's store-aware one
    /// meeting a pointer whose version the store lacks — cannot produce any `.dx` file.
    struct Disk(PathBuf);

    impl Resolver for Disk {
        fn file(&self, path: &str) -> Option<String> {
            if path.ends_with(".dx") {
                return None;
            }
            std::fs::read_to_string(self.0.join(path)).ok()
        }
        fn document(&self, path: &str) -> Option<String> {
            self.file(path)
        }
        fn binary(&self, path: &str) -> Option<Vec<u8>> {
            std::fs::read(self.0.join(path)).ok()
        }
    }

    #[test]
    fn a_declared_binary_file_reads_as_its_digest_and_a_rebuild_stales_it() {
        let (root, _doc_dir) = nested_repo("binary");
        std::fs::create_dir_all(root.join("target/release")).expect("build folder");
        let built = root.join("target/release/dx");
        std::fs::write(&built, [0xff_u8, 0xfe, 0x00, 0x01]).expect("binary");
        let block = Block {
            id: "check".into(),
            language: "bash".into(),
            run: true,
            reads: "target/release/dx".into(),
            text: "target/release/dx --version".into(),
            ..Block::default()
        };
        let resolver = Disk(root.clone());
        let reads = declared_reads(&block, &resolver, &[], Some(&root)).expect("a binary resolves");
        assert_eq!(
            reads,
            vec![(
                "target/release/dx".to_string(),
                sha256_hex(&[0xff, 0xfe, 0x00, 0x01])
            )]
        );
        let recorded = fingerprint("bash", &block.text, &[], &reads, &[], false, 0);
        std::fs::write(&built, [0xff_u8, 0xfe, 0x00, 0x02]).expect("rebuilt");
        let after = declared_reads(&block, &resolver, &[], Some(&root)).expect("resolves");
        assert_ne!(
            recorded,
            fingerprint("bash", &block.text, &[], &after, &[], false, 0)
        );
    }

    #[test]
    fn a_declared_file_on_disk_that_cannot_be_produced_says_so_not_check_the_path() {
        let (root, doc_dir) = nested_repo("pointer");
        std::fs::write(root.join("index.dx"), "~ dx1 0000\n").expect("pointer");
        let block = Block {
            id: "check".into(),
            language: "bash".into(),
            run: true,
            reads: "index.dx".into(),
            text: "true".into(),
            ..Block::default()
        };
        // From a nested document too: the root holds it, and its text is never taken
        // as the content it points at.
        for dir in [&root, &doc_dir] {
            let refused = declared_reads(&block, &Disk(dir.clone()), &[], Some(dir))
                .expect_err("an unresolvable pointer is refused");
            assert!(refused.contains("index.dx is on disk"), "{refused}");
            assert!(refused.contains("dx sync"), "{refused}");
        }
        // A path that names nothing still gets the path sentence.
        let missing = Block {
            reads: "nowhere.txt".into(),
            ..block
        };
        let refused =
            declared_reads(&missing, &Disk(root.clone()), &[], Some(&root)).expect_err("missing");
        assert!(refused.contains("Check the path"), "{refused}");
    }

    #[test]
    fn review_shows_the_code_even_when_a_read_cannot_be_satisfied() {
        let (_root, doc_dir) = nested_repo("review");
        let mut options = gate_options("reads-root-review");
        options.document_dir = doc_dir.clone();
        options.review_only = true;
        let source = "::code id=needy lang=bash run reads=nowhere\necho secret-code\n::end\n";
        let report = run_document(source, &options, &Tree(doc_dir.clone())).expect("acyclic run");
        assert_eq!(report.runs[0].status, "review");
        assert!(report.runs[0].output.contains("nowhere"));
        assert!(report.runs[0].output.contains("echo secret-code"));
        assert!(report.runs[0].output.contains("fingerprint "));
        assert!(!report.changed);
        // And a root-held path reviews cleanly, with no refusal sentence.
        let ok = "::code id=fine lang=bash run reads=crates\necho hi\n::end\n";
        let report = run_document(ok, &options, &Tree(doc_dir)).expect("acyclic run");
        assert!(!report.runs[0].output.contains("could not be read"));
        assert!(report.runs[0].output.contains("echo hi"));
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

    fn count_runs(report: &RunReport, dir: &std::path::Path) -> usize {
        let _ = report;
        std::fs::read_to_string(dir.join("out/count"))
            .map(|text| text.lines().count())
            .unwrap_or(0)
    }

    fn run_counting(body: &str, label: &str, runs: usize) -> (usize, Vec<String>) {
        let root = std::env::temp_dir().join(format!("dx-run-tests-{label}"));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::create_dir_all(&root);
        let mut source = format!("::code id=b lang=bash run writes=out\n{body}\n::end\n");
        let mut statuses = Vec::new();
        let mut last = None;
        for _ in 0..runs {
            let report = run_document(
                &source,
                &RunOptions {
                    document_dir: root.clone(),
                    cache_root: root.join("cache"),
                    default_timeout: Duration::from_secs(60),
                    approve: true,
                    ..RunOptions::default()
                },
                &resolve::Nowhere,
            )
            .expect("no cycle");
            statuses.push(report.runs[0].status.clone());
            source = report.source.clone();
            last = Some(report);
        }
        (count_runs(&last.expect("ran"), &root), statuses)
    }

    #[test]
    fn a_signal_killed_block_is_interrupted_and_reruns() {
        let (count, statuses) = run_counting("echo x >> out/count\nkill -TERM $$", "sig", 2);
        assert_eq!(count, 2, "{statuses:?}");
        assert_eq!(statuses, ["interrupted", "interrupted"]);
    }

    #[test]
    fn a_failing_block_reruns_and_a_passing_block_is_cached() {
        let (count, statuses) = run_counting("echo x >> out/count\nexit 3", "fail", 2);
        assert_eq!(count, 2, "{statuses:?}");
        assert_eq!(statuses, ["error", "error"]);
        let (count, statuses) = run_counting("echo x >> out/count", "pass", 2);
        assert_eq!(count, 1, "{statuses:?}");
        assert_eq!(statuses, ["ok", "skipped"]);
    }

    #[test]
    fn an_older_recorded_143_with_a_kill_marker_counts_as_killed() {
        assert!(looks_killed(143, "Terminated: 15"));
        assert!(!looks_killed(3, "Terminated"));
        assert!(!looks_killed(143, "plain"));
    }

    #[test]
    fn timeout_attribute_causes_timeout_at_specified_seconds() {
        // A block that sleeps 2 seconds with timeout=1 should timeout
        let source_timeout_too_short =
            "::code id=sleep2 lang=bash run timeout=1\nsleep 2 && echo done\n::end\n";
        let report = run_isolated(source_timeout_too_short, "timeout-fail");
        assert_eq!(report.runs.len(), 1);
        assert_eq!(
            report.runs[0].status, "interrupted",
            "block should timeout and be interrupted"
        );
        assert!(
            report.runs[0].output.contains("timed out"),
            "output should mention timeout"
        );

        // Same block with timeout=5 should complete successfully
        let source_timeout_long_enough =
            "::code id=sleep2b lang=bash run timeout=5\nsleep 2 && echo done\n::end\n";
        let report2 = run_isolated(source_timeout_long_enough, "timeout-pass");
        assert_eq!(report2.runs.len(), 1);
        assert_eq!(
            report2.runs[0].status, "ok",
            "block should complete within timeout"
        );
        assert!(
            report2.runs[0].output.contains("done"),
            "output should contain the expected result"
        );
    }

    /// One block run with a stated default and override, timed. dx report-3d99fcfc.
    fn timed_run(
        source: &str,
        label: &str,
        default: Duration,
        timeout_override: Option<Duration>,
    ) -> (String, Duration) {
        let root = std::env::temp_dir().join(format!("dx-run-tests-{label}"));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::create_dir_all(&root);
        let started = std::time::Instant::now();
        let report = run_document(
            source,
            &RunOptions {
                document_dir: root.clone(),
                cache_root: root.join("cache"),
                default_timeout: default,
                timeout_override,
                approve: true,
                ..RunOptions::default()
            },
            &resolve::Nowhere,
        )
        .expect("one block never cycles");
        (report.runs[0].status.clone(), started.elapsed())
    }

    #[test]
    fn the_blocks_own_timeout_header_governs_its_run_not_the_default() {
        // dx report-3d99fcfc: a host gate's `timeout=900` must be what bounds it. Both
        // shapes, sandboxed and `confine=host` (the reported block's), and both
        // directions, each against a default that would give the opposite verdict.
        for confine in ["", " confine=host"] {
            // timeout=3, sleep 1: passes although the default (0.5 s) is shorter.
            let (status, _) = timed_run(
                &format!("::code id=t lang=bash run{confine} timeout=3\nsleep 1\n::end\n"),
                "header-pass",
                Duration::from_millis(500),
                None,
            );
            assert_eq!(
                status, "ok",
                "timeout=3 did not carry sleep 1 ({confine:?})"
            );

            // timeout=1, sleep 3: interrupted at about 1 s although the default is 60 s.
            let (status, elapsed) = timed_run(
                &format!("::code id=t lang=bash run{confine} timeout=1\nsleep 3\n::end\n"),
                "header-cut",
                Duration::from_secs(60),
                None,
            );
            assert_eq!(
                status, "interrupted",
                "timeout=1 did not cut sleep 3 ({confine:?})"
            );
            assert!(
                elapsed >= Duration::from_secs(1) && elapsed < Duration::from_millis(2500),
                "timeout=1 ended after {elapsed:?} ({confine:?})"
            );
        }
    }

    #[test]
    fn an_explicit_timeout_override_replaces_the_blocks_own() {
        // The caller's override (`dx run --timeout S`) wins only when given: it cuts a
        // block whose header allows 60 s at 1 s, and lets a timeout=1 block run 2 s.
        let (status, elapsed) = timed_run(
            "::code id=t lang=bash run timeout=60\nsleep 3\n::end\n",
            "override-cut",
            Duration::from_secs(60),
            Some(Duration::from_secs(1)),
        );
        assert_eq!(status, "interrupted");
        assert!(
            elapsed < Duration::from_millis(2500),
            "override 1 s ended after {elapsed:?}"
        );

        let (status, _) = timed_run(
            "::code id=t lang=bash run timeout=1\nsleep 2\n::end\n",
            "override-pass",
            Duration::from_secs(60),
            Some(Duration::from_secs(5)),
        );
        assert_eq!(status, "ok");
    }

    /// The output a block run under a stated override leaves, with the default 60 s.
    fn env_run(source: &str, label: &str, timeout_override: Option<Duration>) -> BlockRun {
        let root = std::env::temp_dir().join(format!("dx-run-tests-{label}"));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::create_dir_all(&root);
        run_document(
            source,
            &RunOptions {
                document_dir: root.clone(),
                cache_root: root.join("cache"),
                default_timeout: Duration::from_secs(60),
                timeout_override,
                approve: true,
                ..RunOptions::default()
            },
            &resolve::Nowhere,
        )
        .expect("one block never cycles")
        .runs
        .remove(0)
    }

    #[test]
    fn a_block_sees_its_effective_timeout_as_dx_block_timeout() {
        // pcrun reads $DX_BLOCK_TIMEOUT so its own limit never silently undercuts dx's
        // (a `--timeout 3600` host gate was cut at pcrun's 900 s). Sandboxed and host.
        for confine in ["", " confine=host"] {
            let header = format!("::code id=t lang=bash run{confine} timeout=900\necho \"T=$DX_BLOCK_TIMEOUT\"\n::end\n");
            let run = env_run(&header, "env-header", None);
            assert!(
                run.output.contains("T=900"),
                "header ({confine:?}): {}",
                run.output
            );
            let run = env_run(&header, "env-override", Some(Duration::from_secs(3600)));
            assert!(
                run.output.contains("T=3600"),
                "override ({confine:?}): {}",
                run.output
            );
            let bare = format!(
                "::code id=t lang=bash run{confine}\necho \"T=$DX_BLOCK_TIMEOUT\"\n::end\n"
            );
            let run = env_run(&bare, "env-default", None);
            assert!(
                run.output.contains("T=60"),
                "default ({confine:?}): {}",
                run.output
            );
        }
    }

    #[test]
    fn a_crlf_bash_block_runs_with_lf_line_endings() {
        let source =
            "::code id=crlf lang=bash run\nfor i in 1 2; do\r\n echo $i\r\ndone\r\n::end\n";
        assert!(
            parse(source).blocks[0].text.contains("\r\n"),
            "the stored code must carry CRLF for this test to mean anything"
        );
        let report = run_isolated(source, "crlf");
        assert_eq!(report.runs.len(), 1);
        assert_eq!(report.runs[0].status, "ok", "{}", report.runs[0].output);
        assert_eq!(report.runs[0].output.trim(), "1\n2");
    }

    #[test]
    fn crlf_code_is_written_with_lf_and_the_fingerprint_is_untouched() {
        let code = "for i in 1 2; do\r\n echo $i\r\ndone\r\n";
        let dirs = plan::Dirs {
            block: std::env::temp_dir().join("dx-plan-crlf"),
            toolchains: std::env::temp_dir().join("dx-plan-crlf-tc"),
        };
        let prepared = plan::build("bash", code, &[], &dirs).expect("bash exists");
        assert_eq!(prepared.files[0].1, "for i in 1 2; do\n echo $i\ndone\n");
        assert_ne!(
            fingerprint("bash", code, &[], &[], &[], false, 0),
            fingerprint("bash", &code.replace("\r\n", "\n"), &[], &[], &[], false, 0),
            "the fingerprint stays computed from the block as stored"
        );
    }

    #[test]
    fn confine_host_joins_both_fingerprints() {
        assert_ne!(
            fingerprint("bash", "make", &[], &[], &[], false, 0),
            fingerprint("bash", "make", &[], &[], &[], true, 0)
        );
        assert_ne!(
            approval_fingerprint("bash", "make", &[], &[], &[], false),
            approval_fingerprint("bash", "make", &[], &[], &[], true)
        );
        // The declaration is prepended like a grant, so it composes with one.
        assert_ne!(
            approval_fingerprint("bash", "make", &[], &[], &["out".into()], false),
            approval_fingerprint("bash", "make", &[], &[], &["out".into()], true)
        );
    }

    #[test]
    fn confine_accepts_host_and_names_it_when_refusing_anything_else() {
        let block = |value: &str| Block {
            kind: "code".into(),
            confine: value.into(),
            ..Block::default()
        };
        assert_eq!(declared_host(&block("")), Ok(false));
        assert_eq!(declared_host(&block("host")), Ok(true));
        let refused = declared_host(&block("none")).expect_err("only host is accepted");
        assert!(refused.contains("confine=host"), "{refused}");

        let source = "::code id=c lang=bash run confine=sandbox\necho hi\n::end\n";
        let report =
            run_document(source, &gate_options("confine-bad"), &resolve::Nowhere).expect("acyclic");
        assert_eq!(report.runs[0].status, "blocked");
        assert!(report.runs[0].output.contains("`confine=host`"));
    }

    #[test]
    fn an_unapproved_confine_host_block_does_not_run() {
        let options = gate_options("confine-host-unapproved");
        let marker = options.document_dir.join("ran.txt");
        let source = format!(
            "::code id=h lang=bash run confine=host\necho ran > {}\n::end\n",
            marker.display()
        );
        let report = run_document(&source, &options, &resolve::Nowhere).expect("acyclic");
        assert_eq!(report.runs[0].status, "blocked");
        assert!(report.runs[0].output.contains("blocked pending review"));
        assert!(!marker.exists(), "an unapproved host block ran");
    }

    /// "It had your own permissions" includes your own environment: an approved host block
    /// sees dx's `HOME` and `TMPDIR`, not the sandbox's redirection, and still gets the
    /// `DX_*` variables every block gets.
    #[test]
    fn an_approved_confine_host_block_runs_with_dx_s_own_environment() {
        // Other tests point HOME elsewhere for a moment; read it while none of them can.
        let _env = crate::env_lock();
        let home = std::env::var("HOME").expect("HOME is set for the test process");
        let source = "::code id=h lang=bash run confine=host\n\
                      echo \"$HOME\"\necho \"${TMPDIR:-unset}\"\necho \"$DX_BLOCK_ID\"\n\
                      test -d \"$DX_SANDBOX\" && echo sandbox-dir\n::end\n";
        let report = run_isolated(source, "confine-host-environment");
        let run = &report.runs[0];
        assert_eq!(run.status, "ok", "{}", run.output);
        let tmpdir = std::env::var("TMPDIR").unwrap_or_else(|_| "unset".into());
        assert_eq!(
            run.output,
            format!("{HOST_NOTICE}\n{home}\n{tmpdir}\nh\nsandbox-dir")
        );
    }

    #[test]
    fn review_lists_confine_host() {
        let options = RunOptions {
            review_only: true,
            ..gate_options("confine-host-review")
        };
        let source = "::code id=h lang=bash run confine=host\necho hi\n::end\n";
        let report = run_document(source, &options, &resolve::Nowhere).expect("acyclic");
        assert_eq!(report.runs[0].status, "review");
        assert!(
            report.runs[0].output.contains("confine=host"),
            "{}",
            report.runs[0].output
        );
        let plain = "::code id=h lang=bash run\necho hi\n::end\n";
        let report = run_document(plain, &options, &resolve::Nowhere).expect("acyclic");
        assert!(!report.runs[0].output.contains("confine=host"));
    }
}
