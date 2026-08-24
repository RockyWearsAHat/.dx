//! `dx merge-driver` — what git runs when two branches changed the same documents.
//!
//! # Why git needed teaching
//! A workspace commits two kinds of file and git could merge neither. `.doc/repo.dxcp` is a
//! container git classifies as binary, so it refuses and keeps one side whole; a `.dx` file
//! is a single pointer line, so two branches that both edited a document collide on a hex
//! digest, and there is nothing a person can do with that conflict but pick a side and lose
//! the other branch's edits. The everyday consequence — two agents in two worktrees, each
//! writing different documents — was a merge that could not be completed at all.
//!
//! # What runs here
//! Git calls this once per conflicting path, and the path decides the shape:
//!
//! - `.doc/repo.dxcp` — three packs in, one merged pack out ([`doc_store::merge`]).
//! - a `.dx` pointer — three revisions of one document in, one pointer out.
//!
//! Git makes no promise about the order, and the two invocations cannot talk to each other.
//! They do not need to: both call [`doc_core::merge::merge_documents`] on the same three
//! revisions of the same document, and that is a pure function, so the pointer this writes
//! is the digest of the document the pack invocation stores — whichever ran first.
//!
//! # This writes no store and creates no index
//! A merge driver runs mid-operation, with the working tree half-rewritten and the pack
//! about to be replaced by git itself; ingesting from here would race git's own write and
//! would make `git merge` mutate a database that is supposed to be rebuildable from the
//! pack. So the driver is bytes in, bytes out, and `dx sync` — the same command a fresh
//! clone runs — is what puts the index back in agreement afterwards.
//!
//! # A conflict is left in words, not in digests
//! When two branches genuinely rewrote the same block, the `.dx` file is written as **plain
//! document text carrying git's conflict markers**, not as a pointer. That is a file a person
//! can open and resolve, and `dx sync` adopts the result once the markers are gone — it
//! refuses while they remain, so a conflict cannot be stored as if it were a document.

use std::path::{Path, PathBuf};
use std::process::Command;

use doc_core::chunk::PackStorage;
use doc_core::merge::{merge_documents, DEFAULT_MARKER_SIZE};
use doc_store::{merge as pack_merge, pack, stub};

use crate::args::Args;
use crate::workspace;

/// The revisions a git operation leaves lying around, tried before the pack's own history.
///
/// A merge sets `MERGE_HEAD`; a rebase sets `REBASE_HEAD`; `git cherry-pick` and `git revert`
/// set their own. The driver does not need to know which operation is running, because it
/// looks content up **by digest** — any pack that carries the content answers, so the whole
/// list is tried and the first hit wins.
///
/// These are one `git cat-file` each, which is why they go first. They are **not** enough on
/// their own; [`pack_history`] states the case that proves it.
const SIDE_REVISIONS: [&str; 6] = [
    "HEAD",
    "MERGE_HEAD",
    "REBASE_HEAD",
    "CHERRY_PICK_HEAD",
    "REVERT_HEAD",
    "ORIG_HEAD",
];

/// How far back the committed pack's history is followed looking for one version.
///
/// The content being looked for belongs to a commit that is part of the operation in flight, so
/// it is near the front of this list in every real case. The bound is here because an
/// unresolvable digest must cost a bounded search rather than one proportional to the age of
/// the repository.
const HISTORY_DEPTH: usize = 200;

/// `dx merge-driver` — merge one path git could not.
///
/// Git supplies `--ancestor %O --ours %A --theirs %B --path %P --marker-size %L`. The result
/// is written over the `--ours` file, which is where git reads it back from. `--root` names
/// the workspace to resolve content against; it is not configured, because a driver that
/// carried a baked-in root would resolve a linked worktree's merge against a different
/// checkout — [`worktree_root`] is the answer, and it is computed per invocation.
///
/// # Errors
/// A conflict, or a failure to read any side. Both are the same signal to git — a non-zero
/// exit means "left conflicted, ask a person" — so the message says which it was.
pub fn run_merge_driver(args: &Args) -> Result<String, String> {
    let ours = required(args, "ours")?;
    let theirs = required(args, "theirs")?;
    let ancestor = args.value("ancestor").map(PathBuf::from);
    // %P is the path in the working tree; %A is a temporary file with our content in it. Only
    // %P says what kind of file this is, so it decides which merge runs.
    let named = args.value("path").unwrap_or(&ours).to_string();
    let marker = args
        .number("marker-size")
        .map_or(DEFAULT_MARKER_SIZE, |size| size as usize);
    let root = args.value("root").map_or_else(
        || workspace::workspace_root(&worktree_root()),
        PathBuf::from,
    );

    if named.ends_with(".dxcp") {
        merge_pack(
            &named,
            ancestor.as_deref(),
            Path::new(&ours),
            Path::new(&theirs),
            marker,
        )
    } else {
        merge_pointer(
            &root,
            &named,
            ancestor.as_deref(),
            Path::new(&ours),
            Path::new(&theirs),
            marker,
        )
    }
}

/// Merge the committed pack: every document in it, three ways.
fn merge_pack(
    named: &str,
    ancestor: Option<&Path>,
    ours: &Path,
    theirs: &Path,
    marker: usize,
) -> Result<String, String> {
    let base = match ancestor {
        Some(path) => Some(read_bytes(path)?),
        None => None,
    };
    let merged = pack_merge::merge_packs(
        base.as_deref(),
        &read_bytes(ours)?,
        &read_bytes(theirs)?,
        // Git only ever merges a tracked file, and the tracked pack is the one written plain
        // so git can delta it between revisions.
        PackStorage::ForVersionControl,
        marker,
    )
    .map_err(|error| format!("dx merge-driver could not merge {named}: {error}"))?;

    std::fs::write(ours, &merged.bytes)
        .map_err(|error| format!("could not write the merged pack: {error}"))?;

    if merged.is_clean() {
        return Ok(format!(
            "dx merge-driver — merged {} document(s) in {named}\n",
            merged.documents.len()
        ));
    }
    Err(format!(
        "dx merge-driver — {named} merged, but {} document(s) need a person: {}\nOpen each \
         .dx file, resolve the <<<<<<< markers, then run `dx sync`.\n",
        merged.conflicts.len(),
        merged.conflicts.join(", ")
    ))
}

/// Merge one document, and write the pointer that names the result.
fn merge_pointer(
    root: &Path,
    named: &str,
    ancestor: Option<&Path>,
    ours: &Path,
    theirs: &Path,
    marker: usize,
) -> Result<String, String> {
    let mut sides = Sides::new(root);
    // An ancestor that will not resolve costs a three-way merge its base, which makes more
    // blocks conflict and loses nothing. A *side* that will not resolve is the case with
    // nothing to merge at all, and [`unresolved_side`] is what must happen then.
    let base = ancestor.and_then(|path| sides.source_at(named, "common ancestor", path).ok());
    let mine = sides.source_at(named, "side of this branch", ours);
    let yours = sides.source_at(named, "incoming side", theirs);
    let (mine, yours) = match (mine, yours) {
        (Ok(mine), Ok(yours)) => (mine, yours),
        (mine, yours) => return unresolved_side(named, ours, marker, &mine, &yours),
    };

    let merged = merge_documents(base.as_deref(), &mine, &yours, marker);
    if merged.is_clean() {
        // The pointer, not the content: the content is the pack's job, and this invocation
        // may well be running before git has even replaced the pack.
        let pointer = if merged.text.is_empty() {
            String::new()
        } else {
            stub::render(&merged.text)
        };
        std::fs::write(ours, pointer)
            .map_err(|error| format!("could not write the merged pointer: {error}"))?;
        return Ok(format!("dx merge-driver — merged {named}\n"));
    }

    // Conflicted: the file becomes the document itself, markers and all. A `.dx` file is
    // allowed to be plain text — that is how anything but dx writes one — so this is a file
    // every editor, and `git diff`, can already show.
    std::fs::write(ours, &merged.text)
        .map_err(|error| format!("could not write the conflicted document: {error}"))?;
    Err(format!(
        "dx merge-driver — {named} conflicts in {} block(s): {}\nIt is now plain text with \
         conflict markers; resolve them and run `dx sync`.\n",
        merged.conflicts.len(),
        merged.conflicts.join(", ")
    ))
}

/// Write a file for the case where one side of the merge cannot be resolved to a document.
///
/// # Why this writes anything at all
/// It used to write nothing: the driver returned the error and git, seeing a failed driver,
/// marked the path unmerged and left the `--ours` file as it stood. That file is a **clean
/// pointer to our side**, so every reader — `dx text`, an editor, a person opening it — was
/// shown a finished document with the other branch's work absent and nothing to say it had
/// ever existed. `git add -A && git commit`, which is how a person finishes a merge, then
/// committed exactly that.
///
/// So the unresolvable case is written down as what it is: a conflict, in git's own markers,
/// carrying whichever side did resolve and the sentence explaining the one that did not. It is
/// plain text, which a `.dx` file is allowed to be, and every route in dx refuses to read or
/// adopt it while the markers remain — [`workspace::half_merged`] and `Store::sync` both — so
/// this cannot be committed as a document by accident.
fn unresolved_side(
    named: &str,
    ours: &Path,
    marker: usize,
    mine: &Result<String, String>,
    yours: &Result<String, String>,
) -> Result<String, String> {
    let run = |glyph: char| std::iter::repeat_n(glyph, marker).collect::<String>();
    let body = |side: &Result<String, String>| match side {
        Ok(source) => source.trim_end().to_string(),
        Err(sentence) => sentence.clone(),
    };
    let text = format!(
        "{} ours\n{}\n{}\n{}\n{} theirs\n",
        run('<'),
        body(mine),
        run('='),
        body(yours),
        run('>')
    );
    std::fs::write(ours, &text)
        .map_err(|error| format!("could not write the unresolved merge: {error}"))?;
    let why = match (mine, yours) {
        (Err(sentence), _) | (_, Err(sentence)) => sentence.clone(),
        // Unreachable: this is only called when at least one side is an error.
        (Ok(_), Ok(_)) => String::new(),
    };
    Err(format!(
        "{why}\n{named} is now plain text holding the side that did resolve, between conflict \
         markers, so it cannot be committed as a finished document. Recover the missing side \
         and run `dx sync`.\n"
    ))
}

/// Resolves each side of a pointer merge to the document text it stands for.
///
/// Three sources are tried in cost order, and only as far as the answer needs:
///
/// 1. The file is already document text (a side something other than dx wrote) — use it.
/// 2. The workspace's own store and packs — every version this machine has held.
/// 3. The pack blob at each revision git can still produce — [`SIDE_REVISIONS`] first, then
///    the committed pack's own history ([`pack_history`]).
///
/// The third is what makes an incoming branch resolvable at all: content that arrived over
/// the network has never been in this machine's index, and lives only inside the committed
/// pack in git's object database. The lookup is by digest rather than by revision, so it does
/// not have to know whether a merge, a rebase, or a cherry-pick is running.
struct Sides<'a> {
    root: &'a Path,
    /// Documents decoded out of git's pack blobs so far, keyed by digest.
    found: Vec<(String, String)>,
    /// Revisions whose pack has not been decoded yet, in the order they are worth trying.
    queue: std::collections::VecDeque<String>,
    /// Whether [`SIDE_REVISIONS`] has been queued, and whether the history has followed it.
    seeded: bool,
    walked: bool,
}

impl<'a> Sides<'a> {
    fn new(root: &'a Path) -> Self {
        Self {
            root,
            found: Vec::new(),
            queue: std::collections::VecDeque::new(),
            seeded: false,
            walked: false,
        }
    }

    /// The document text one side's file stands for; an absent or empty file is a deletion.
    ///
    /// `named` and `side` are for the failure only: git hands each revision over as a temp
    /// file with a name like `.merge_file_abfKlS`, so a message that named the file it was
    /// given told the reader nothing about which document, or whose half of it, could not be
    /// found.
    fn source_at(&mut self, named: &str, side: &str, path: &Path) -> Result<String, String> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
            Err(error) => return Err(format!("could not read {}: {error}", path.display())),
        };
        if text.trim().is_empty() {
            return Ok(String::new());
        }
        let Some(digest) = stub::digest_in(&text) else {
            return Ok(text);
        };
        if let Ok(source) = workspace::resolve_contents(&text, self.root) {
            return Ok(source);
        }
        if let Some(source) = self.find_in_git(&digest) {
            return Ok(source);
        }
        Err(format!(
            "dx merge-driver cannot merge {named}: its {side} points at version {digest}, which \
             is not in this workspace's store, its packs, or any .doc/repo.dxcp git still has. \
             That side's commit carries a pointer whose content never reached .doc/repo.dxcp — \
             run `dx sync` on the branch it came from and commit the pack, or fetch the branch \
             if it is simply not here yet."
        ))
    }

    /// The document with `digest`, out of whichever pack blob git can still produce it from.
    ///
    /// Revisions are decoded one at a time and only until the wanted version turns up, because
    /// the common case answers on the first: `HEAD`'s pack holds every document this branch has
    /// ever committed. What follows costs nothing when it is not needed.
    fn find_in_git(&mut self, digest: &str) -> Option<String> {
        loop {
            if let Some((_, source)) = self.found.iter().find(|(held, _)| held == digest) {
                return Some(source.clone());
            }
            let revision = self.next_revision()?;
            let Some(bytes) = pack_blob(self.root, &revision) else {
                continue;
            };
            let Ok(documents) = pack::decode(&bytes, &revision) else {
                continue;
            };
            for (_, source) in documents {
                self.found.push((stub::digest_of(&source), source));
            }
        }
    }

    /// The next revision worth decoding: the named refs, and then the pack's own history.
    fn next_revision(&mut self) -> Option<String> {
        if !self.seeded {
            self.seeded = true;
            self.queue
                .extend(SIDE_REVISIONS.iter().map(|name| (*name).to_string()));
        }
        if let Some(next) = self.queue.pop_front() {
            return Some(next);
        }
        if !self.walked {
            self.walked = true;
            self.queue.extend(pack_history(self.root, HISTORY_DEPTH));
        }
        self.queue.pop_front()
    }
}

/// Every revision of the committed pack, newest first, across every ref in the repository.
///
/// # Why the named refs are not enough
/// The case that proves it is the ordinary one. **Git writes `MERGE_HEAD` only once the merge
/// is known to be unfinished, which is after every merge driver has run** — so for the whole
/// time this driver is executing, the commit being merged in has no name at all. `HEAD` and
/// `ORIG_HEAD` are both our own side, and the incoming document's content, which lives only
/// inside that commit's `.doc/repo.dxcp`, could not be reached by any of them.
///
/// What that cost was not a failed merge but a silent one. The pointer driver gave up, wrote
/// nothing, and left the `--ours` file exactly as it found it: a clean pointer to our side.
/// Git marked the path unmerged, and the next `git add -A && git commit` — the thing a person
/// does to finish a merge — committed our side as the merge result with the other branch's
/// edit dropped and no marker anywhere to say so.
///
/// `--all` reaches it, because a fetched branch is a ref even while the merge that is
/// consuming it is not. The lookup this feeds is by digest, so which revision carries the
/// content does not matter, and the incoming tip is a recent one either way.
///
/// Complexity: one `git rev-list` bounded by `depth`; the caller decodes only as far as it
/// must.
pub(crate) fn pack_history(root: &Path, depth: usize) -> Vec<String> {
    let Ok(output) = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "rev-list",
            "--all",
            &format!("--max-count={depth}"),
            "--",
            pack::REPO_PACK,
        ])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_string)
        .collect()
}

/// The committed pack as it stood at `revision`, or `None` when git has no such thing.
pub(crate) fn pack_blob(root: &Path, revision: &str) -> Option<Vec<u8>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "cat-file",
            "blob",
            &format!("{revision}:{}", pack::REPO_PACK),
        ])
        .output()
        .ok()?;
    if output.status.success() && !output.stdout.is_empty() {
        Some(output.stdout)
    } else {
        None
    }
}

/// The worktree this driver was invoked inside.
///
/// Git runs a driver from the top of the working tree, so the current directory is normally
/// right; asking git makes it right even when it is not. **This must never be a baked-in
/// absolute path**: a linked worktree is a second checkout of the same repository, and a
/// driver that resolved against the original would merge one checkout's documents into
/// another's — and touch that checkout's index while doing it.
pub(crate) fn worktree_root() -> PathBuf {
    let here = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let Ok(output) = Command::new("git")
        .arg("-C")
        .arg(&here)
        .args(["rev-parse", "--show-toplevel"])
        .output()
    else {
        return here;
    };
    if !output.status.success() {
        return here;
    }
    let top = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if top.is_empty() {
        here
    } else {
        PathBuf::from(top)
    }
}

/// The value of a flag git always supplies, or a sentence saying the driver was misconfigured.
fn required(args: &Args, name: &str) -> Result<String, String> {
    args.value(name).map(str::to_string).ok_or_else(|| {
        format!(
            "dx merge-driver needs --{name}; git supplies it. Run `dx git-setup` to write the \
             driver configuration this repository is missing."
        )
    })
}

/// Read a whole file, treating an absent one as empty — git hands an empty side for a path
/// a branch does not have.
fn read_bytes(path: &Path) -> Result<Vec<u8>, String> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(format!("could not read {}: {error}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use doc_core::format::parse;

    fn args(tokens: &[&str]) -> Args {
        Args::parse(&tokens.iter().map(|t| (*t).to_string()).collect::<Vec<_>>())
    }

    fn scratch(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dx-merge-driver-tests-{label}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".doc")).expect("scratch");
        std::fs::canonicalize(&root).expect("canonical")
    }

    fn document(heading: &str, body: &str) -> String {
        format!("::heading level=1 id=h\n{heading}\n::end\n\n::paragraph id=p\n{body}\n::end\n")
    }

    /// Write the three sides git would hand the driver, and return their paths.
    fn sides(root: &Path, base: &str, ours: &str, theirs: &str) -> (PathBuf, PathBuf, PathBuf) {
        let (o, a, b) = (root.join("O"), root.join("A"), root.join("B"));
        std::fs::write(&o, base).expect("base");
        std::fs::write(&a, ours).expect("ours");
        std::fs::write(&b, theirs).expect("theirs");
        (o, a, b)
    }

    fn driver(root: &Path, named: &str, o: &Path, a: &Path, b: &Path) -> Result<String, String> {
        run_merge_driver(&args(&[
            "--ancestor",
            &o.to_string_lossy(),
            "--ours",
            &a.to_string_lossy(),
            "--theirs",
            &b.to_string_lossy(),
            "--path",
            named,
            "--marker-size",
            "7",
            "--root",
            &root.to_string_lossy(),
        ]))
    }

    /// Put a version in the store the way saving the document would, so a pointer to it
    /// resolves. Successive saves keep every version — that is what lets a merge reach the
    /// ancestor's text as well as the two branches'.
    fn store(root: &Path, source: &str) {
        workspace::save(&root.join("scratch.dx"), &parse(source)).expect("store the version");
    }

    #[test]
    fn two_branches_writing_different_documents_merge_into_one_pack() {
        // The filed symptom: a worktree added documents while master edited another, and git
        // called the pack binary and refused.
        let root = scratch("pack-clean");
        let pack_of = |documents: &[(&str, String)]| {
            let parsed: Vec<(String, doc_core::model::Document)> = documents
                .iter()
                .map(|(path, source)| ((*path).to_string(), parse(source)))
                .collect();
            doc_core::chunk::encode_pack_for(
                &doc_core::chunk::Pack::build(
                    parsed.iter().map(|(path, doc)| (path.as_str(), doc)),
                ),
                PackStorage::ForVersionControl,
            )
        };
        let base = pack_of(&[("index.dx", document("Index", "one"))]);
        let ours = pack_of(&[
            ("index.dx", document("Index", "one")),
            ("new.dx", document("New", "mine")),
        ]);
        let theirs = pack_of(&[("index.dx", document("Index", "edited"))]);

        let (o, a, b) = (root.join("O"), root.join("A"), root.join("B"));
        std::fs::write(&o, &base).expect("base");
        std::fs::write(&a, &ours).expect("ours");
        std::fs::write(&b, &theirs).expect("theirs");

        let report = driver(&root, pack::REPO_PACK, &o, &a, &b).expect("clean merge");
        assert!(report.contains("2 document(s)"), "{report}");

        let merged =
            pack::decode(&std::fs::read(&a).expect("read back"), "merged").expect("decode");
        assert_eq!(merged.len(), 2);
        let index = &merged
            .iter()
            .find(|(p, _)| p == "index.dx")
            .expect("index")
            .1;
        assert!(index.contains("edited"), "{index}");
    }

    #[test]
    fn the_pointer_a_merge_writes_names_the_document_the_pack_merge_stored() {
        // This is the whole architecture in one test. Git invokes the two drivers separately
        // and in no order a caller may rely on, and they cannot talk to each other — so the
        // pointer must come out naming exactly the document the pack came out holding, with
        // neither invocation having seen the other's work.
        let root = scratch("pointer-clean");
        let base = document("Notes", "one");
        let mine = format!("{base}\n::paragraph id=mine\nmine\n::end\n");
        let yours = document("Notes edited", "one");
        // Every version has to be resolvable, so store all three the way the workspace would.
        for source in [&base, &mine, &yours] {
            store(&root, source);
        }

        // The pack side, exactly as git would call it.
        let pack_of = |source: &str| {
            let parsed = parse(source);
            doc_core::chunk::encode_pack_for(
                &doc_core::chunk::Pack::build([("notes.dx", &parsed)]),
                PackStorage::ForVersionControl,
            )
        };
        let (po, pa, pb) = (root.join("pO"), root.join("pA"), root.join("pB"));
        std::fs::write(&po, pack_of(&base)).expect("base pack");
        std::fs::write(&pa, pack_of(&mine)).expect("our pack");
        std::fs::write(&pb, pack_of(&yours)).expect("their pack");
        driver(&root, pack::REPO_PACK, &po, &pa, &pb).expect("pack merges clean");
        std::fs::write(
            root.join(pack::REPO_PACK),
            std::fs::read(&pa).expect("read"),
        )
        .expect("git puts the merged pack in the tree");

        // The pointer side, which has not seen any of that.
        let (o, a, b) = sides(
            &root,
            &stub::render(&base),
            &stub::render(&mine),
            &stub::render(&yours),
        );
        let report = driver(&root, "notes.dx", &o, &a, &b).expect("clean merge");
        assert!(report.contains("merged notes.dx"), "{report}");

        let written = std::fs::read_to_string(&a).expect("read back");
        assert!(stub::is_stub(&written), "{written}");
        let merged = pack::source(&root, "notes.dx")
            .expect("read the merged pack")
            .expect("the merged pack carries the document");
        assert_eq!(
            stub::digest_in(&written).expect("a digest"),
            stub::digest_of(&merged),
            "the pointer must name what the pack merge stored"
        );
        assert!(merged.contains("Notes edited"), "{merged}");
        assert!(merged.contains("id=mine"), "{merged}");
    }

    #[test]
    fn a_conflict_leaves_words_a_person_can_resolve_rather_than_a_digest() {
        let root = scratch("pointer-conflict");
        let base = document("Notes", "one");
        let mine = document("Notes", "my version");
        let yours = document("Notes", "their version");
        for source in [&base, &mine, &yours] {
            store(&root, source);
        }

        let (o, a, b) = sides(
            &root,
            &stub::render(&base),
            &stub::render(&mine),
            &stub::render(&yours),
        );
        let error = driver(&root, "notes.dx", &o, &a, &b).expect_err("must conflict");
        assert!(error.contains("conflicts in 1 block(s)"), "{error}");

        let written = std::fs::read_to_string(&a).expect("read back");
        assert!(!stub::is_stub(&written), "a conflict is not a pointer");
        assert!(written.contains("<<<<<<<"), "{written}");
        assert!(written.contains("my version"), "{written}");
        assert!(written.contains("their version"), "{written}");
    }

    #[test]
    fn a_side_written_as_plain_text_merges_without_needing_the_store() {
        // Anything but dx writes a `.dx` file as plain text, and a branch may well carry one.
        let root = scratch("pointer-plain");
        let (o, a, b) = sides(
            &root,
            &document("Notes", "one"),
            &document("Notes", "mine"),
            &document("Notes", "one"),
        );
        driver(&root, "notes.dx", &o, &a, &b).expect("clean merge");
        let written = std::fs::read_to_string(&a).expect("read back");
        assert!(stub::is_stub(&written), "{written}");
    }

    #[test]
    fn a_pointer_naming_content_nobody_has_says_what_to_run() {
        let root = scratch("pointer-missing");
        let (o, a, b) = sides(
            &root,
            &document("Notes", "one"),
            &stub::render(&document("Notes", "unknown to everyone")),
            &document("Notes", "theirs"),
        );
        let error = driver(&root, "notes.dx", &o, &a, &b).expect_err("cannot resolve");
        assert!(error.contains("dx sync"), "{error}");
    }

    #[test]
    fn a_side_that_cannot_be_resolved_is_written_down_as_a_conflict() {
        // The silent failure this pins down: the driver used to write nothing here, and the
        // `--ours` file git left in place was a clean pointer to our side. Every reader then
        // saw a finished document with the other branch's work simply absent.
        let root = scratch("pointer-unresolvable-theirs");
        let mine = document("Notes", "the side that is here");
        store(&root, &mine);
        let (o, a, b) = sides(
            &root,
            &document("Notes", "one"),
            &stub::render(&mine),
            &stub::render(&document("Notes", "arrived without its pack")),
        );

        let error = driver(&root, "notes.dx", &o, &a, &b).expect_err("cannot resolve the incoming");
        assert!(error.contains("incoming side"), "{error}");

        let written = std::fs::read_to_string(&a).expect("read back");
        assert!(
            !stub::is_stub(&written),
            "our side must not be left standing as a finished document: {written}"
        );
        assert!(doc_core::merge::has_conflict_markers(&written), "{written}");
        assert!(written.contains("the side that is here"), "{written}");
        assert!(
            written.contains("is not in this workspace's store"),
            "the missing side must say why it is missing: {written}"
        );
    }

    #[test]
    fn an_incoming_branch_resolves_before_git_has_a_name_for_it() {
        // Git writes `MERGE_HEAD` only once the merge is known to be unfinished, which is
        // after every driver has run, so the commit being merged in has no name while this
        // code executes. Here the incoming version exists *only* inside a commit no named
        // revision reaches — which is exactly the shape of a real merge — and the driver has
        // to walk the pack's own history to find it.
        let root = scratch("incoming-unnamed");
        let git = |args: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(args)
                .output()
                .is_ok_and(|out| out.status.success())
        };
        if !git(&["init", "-q", "-b", "main"]) {
            return; // No usable git on this machine; there is nothing to drive.
        }
        assert!(git(&["config", "user.email", "dx@example.invalid"]));
        assert!(git(&["config", "user.name", "dx tests"]));

        let base = document("Notes", "one");
        let theirs = document("Notes", "what the other branch wrote");
        store(&root, &base);
        // Only the pack and the pointer: a committed `index.db` would answer for free and
        // prove nothing.
        assert!(git(&["add", "scratch.dx", pack::REPO_PACK]));
        assert!(git(&["commit", "-qm", "base"]));
        store(&root, &theirs);
        assert!(git(&["add", "scratch.dx", pack::REPO_PACK]));
        assert!(git(&["commit", "-qm", "theirs"]));

        // Put that commit where only `--all` reaches it, and take away every local copy of
        // its content, so the pack blob in git's object database is the sole remaining source.
        assert!(git(&["branch", "side"]));
        assert!(git(&["reset", "-q", "--hard", "HEAD~1"]));
        // The reset names that commit `ORIG_HEAD`; a real merge does not — there `ORIG_HEAD`
        // is our own side. Leaving it would let the test pass for a reason the merge cannot
        // rely on.
        std::fs::remove_file(root.join(".git/ORIG_HEAD")).expect("unname the incoming commit");
        std::fs::remove_file(root.join(".doc/index.db")).expect("drop the local index");
        let _ = std::fs::remove_file(root.join(".doc/local.dxcp"));
        assert!(
            workspace::resolve_contents(&stub::render(&theirs), &root).is_err(),
            "the incoming version must be unreachable except through git"
        );

        let (o, a, b) = sides(
            &root,
            &stub::render(&base),
            &stub::render(&base),
            &stub::render(&theirs),
        );
        let report = driver(&root, "notes.dx", &o, &a, &b).expect("the incoming side resolves");
        assert!(report.contains("merged notes.dx"), "{report}");

        let written = std::fs::read_to_string(&a).expect("read back");
        assert_eq!(
            stub::digest_in(&written).expect("a digest"),
            stub::digest_of(&theirs),
            "an unchanged side must take the incoming version whole"
        );
    }

    #[test]
    fn a_missing_flag_names_the_command_that_writes_the_configuration() {
        let error = run_merge_driver(&args(&["--ours", "/tmp/a"])).expect_err("needs --theirs");
        assert!(error.contains("--theirs"), "{error}");
        assert!(error.contains("dx git-setup"), "{error}");
    }
}
