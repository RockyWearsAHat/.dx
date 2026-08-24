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

/// The revisions a git operation leaves lying around, and where a merge's other side lives.
///
/// A merge sets `MERGE_HEAD`; a rebase sets `REBASE_HEAD`; `git cherry-pick` and `git revert`
/// set their own. The driver does not need to know which operation is running, because it
/// looks content up **by digest** — any pack that carries the content answers, so the whole
/// list is tried and the first hit wins.
const SIDE_REVISIONS: [&str; 6] = [
    "HEAD",
    "MERGE_HEAD",
    "REBASE_HEAD",
    "CHERRY_PICK_HEAD",
    "REVERT_HEAD",
    "ORIG_HEAD",
];

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
    let base = match ancestor {
        Some(path) => Some(sides.source_at(named, "common ancestor", path)?),
        None => None,
    };
    let mine = sides.source_at(named, "side of this branch", ours)?;
    let yours = sides.source_at(named, "incoming side", theirs)?;

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

/// Resolves each side of a pointer merge to the document text it stands for.
///
/// Three sources are tried in cost order, and only as far as the answer needs:
///
/// 1. The file is already document text (a side something other than dx wrote) — use it.
/// 2. The workspace's own store and packs — every version this machine has held.
/// 3. The pack blob at each revision a git operation leaves behind ([`SIDE_REVISIONS`]).
///
/// The third is what makes an incoming branch resolvable at all: content that arrived over
/// the network has never been in this machine's index, and lives only inside the committed
/// pack in git's object database. The lookup is by digest rather than by revision, so it does
/// not have to know whether a merge, a rebase, or a cherry-pick is running.
struct Sides<'a> {
    root: &'a Path,
    /// Documents found in git's pack blobs, keyed by digest; built once, on first need.
    from_git: Option<Vec<(String, String)>>,
}

impl<'a> Sides<'a> {
    fn new(root: &'a Path) -> Self {
        Self {
            root,
            from_git: None,
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
        for (found, source) in self.git_documents() {
            if *found == digest {
                return Ok(source.clone());
            }
        }
        Err(format!(
            "dx merge-driver cannot merge {named}: its {side} points at version {digest}, which \
             is not in this workspace's store, its packs, or any .doc/repo.dxcp git still has. \
             That side's commit carries a pointer whose content never reached .doc/repo.dxcp — \
             run `dx sync` on the branch it came from and commit the pack, or fetch the branch \
             if it is simply not here yet."
        ))
    }

    /// Every document in every pack blob git can still produce, keyed by digest.
    fn git_documents(&mut self) -> &[(String, String)] {
        self.from_git.get_or_insert_with(|| {
            let mut found: Vec<(String, String)> = Vec::new();
            for revision in SIDE_REVISIONS {
                let Some(bytes) = pack_blob(self.root, revision) else {
                    continue;
                };
                let Ok(documents) = pack::decode(&bytes, revision) else {
                    continue;
                };
                for (_, source) in documents {
                    found.push((stub::digest_of(&source), source));
                }
            }
            found
        })
    }
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
    fn a_missing_flag_names_the_command_that_writes_the_configuration() {
        let error = run_merge_driver(&args(&["--ours", "/tmp/a"])).expect_err("needs --theirs");
        assert!(error.contains("--theirs"), "{error}");
        assert!(error.contains("dx git-setup"), "{error}");
    }
}
