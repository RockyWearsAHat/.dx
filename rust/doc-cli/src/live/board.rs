//! The live board: every watched repo's snapshot as one generated `.dx` document, so every dx
//! surface shows it with no renderer of its own — `dx render`/`dx png`, the DX.app viewer (which
//! re-renders a document whenever its file is replaced) and an agent's `dx_read`.
//!
//! Two boards are written, both from snapshot files alone:
//! - `<git-common-dir>/dx-live/board.dx`, beside `snapshot.json`: that repo, written with it.
//! - `<config dir>/live-board/board.dx` (beside `~/.config/dx/live-repos`): every watched
//!   repo, rewritten by the watcher after each refresh and by `dx live --board`.
//!
//! Building a board is pure reads: snapshot files, a stat per screen image, and a clone-copy of
//! an image the board folder does not hold yet. No git, no gate runs. A screen an `::image
//! for=<gate>` claims is shown under that gate only while the gate passes.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use doc_core::model::{Block, Document, Item};

use super::gates::{GateState, GateVerdict};
use super::merge::MergeState;
use super::{iso, sha7, BranchState, Snapshot};

/// The board's file name inside its folder.
pub const BOARD: &str = "board.dx";

/// Where the board's screen copies live, relative to the board (so the renderer's path law,
/// "a reference stays inside the document's folder", lets it embed them).
const IMG: &str = "img";

/// One repo on a board: where it is, and its snapshot or why there is none.
#[derive(Debug, Clone)]
pub struct Entry {
    pub repo: PathBuf,
    pub snapshot: Result<Snapshot, String>,
}

/// The extension an `::image` may embed, normalized, or `None` for anything else.
pub fn raster_extension(path: &str) -> Option<&'static str> {
    let ext = path.rsplit_once('.')?.1.to_ascii_lowercase();
    match ext.as_str() {
        "png" => Some("png"),
        "jpg" | "jpeg" => Some("jpg"),
        "gif" => Some("gif"),
        "webp" => Some("webp"),
        _ => None,
    }
}

/// The absolute git common dir of the repo at or above `repo`, found by reading `.git` files
/// only (a `gitdir:` file and its `commondir`), never by running git.
pub fn common_dir(repo: &Path) -> Option<PathBuf> {
    let top = repo.ancestors().find(|p| p.join(".git").exists())?;
    let dot = top.join(".git");
    if dot.is_dir() {
        return Some(dot);
    }
    let text = fs::read_to_string(&dot).ok()?;
    let gitdir = text.trim().strip_prefix("gitdir:")?.trim();
    let gitdir = top.join(gitdir);
    Some(match fs::read_to_string(gitdir.join("commondir")) {
        Ok(common) => gitdir.join(common.trim()),
        Err(_) => gitdir,
    })
}

/// The snapshot `dx live` last wrote for `repo`, read from its file.
pub fn read_snapshot(repo: &Path) -> Result<Snapshot, String> {
    let common = common_dir(repo).ok_or_else(|| format!("{} is not a git repo", repo.display()))?;
    let path = common.join("dx-live").join("snapshot.json");
    let text = fs::read_to_string(&path)
        .map_err(|e| format!("no snapshot at {} yet ({e})", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{} does not read: {e}", path.display()))
}

/// The folder of the board of every watched repo: `live-board` beside the watch list.
pub fn combined_dir() -> PathBuf {
    let list = super::watch::config_path();
    list.parent()
        .map_or_else(|| PathBuf::from("live-board"), |d| d.join("live-board"))
}

/// Rewrite the board of every repo in `repos` from their snapshot files; its path.
pub fn write_combined(repos: &[PathBuf]) -> Result<PathBuf, String> {
    let entries: Vec<Entry> = repos
        .iter()
        .map(|repo| Entry {
            repo: repo.clone(),
            snapshot: read_snapshot(repo),
        })
        .collect();
    write(&combined_dir(), &entries)
}

/// Stable name for a screen copy: FNV-1a of its source path, size and mtime, so an unchanged
/// screen costs one stat and a changed one gets a new name (and the viewer a fresh picture).
fn copy_name(path: &Path, meta: &fs::Metadata, ext: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    let key = format!("{}\0{}\0{mtime}", path.display(), meta.len());
    for b in key.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}.{ext}")
}

/// Write `entries` as `dir/board.dx`, copying the screens it shows into `dir/img` and removing
/// copies no longer shown. The board file is replaced (tmp + rename) only when its text
/// changed, so a viewer watching it reloads exactly when there is something new.
pub fn write(dir: &Path, entries: &[Entry]) -> Result<PathBuf, String> {
    let img = dir.join(IMG);
    fs::create_dir_all(&img).map_err(|e| format!("cannot create {}: {e}", img.display()))?;
    let mut used = BTreeSet::new();
    let mut place = |source: &str| -> Option<String> {
        let path = Path::new(source);
        let ext = raster_extension(source)?;
        let meta = fs::metadata(path).ok().filter(|m| m.is_file())?;
        if meta.len() as usize > doc_core::resolve::MAX_IMAGE_BYTES {
            return None;
        }
        let name = copy_name(path, &meta, ext);
        let target = img.join(&name);
        if !target.exists() {
            let tmp = img.join(format!(".{name}.tmp{}", std::process::id()));
            fs::copy(path, &tmp).ok()?;
            fs::rename(&tmp, &target).ok()?;
        }
        used.insert(name.clone());
        Some(format!("{IMG}/{name}"))
    };
    let text = doc_core::format::stringify(&document(entries, &mut place));
    let board = dir.join(BOARD);
    if fs::read_to_string(&board).ok().as_deref() != Some(text.as_str()) {
        super::atomic_write(&board, &text)?;
    }
    if let Ok(rd) = fs::read_dir(&img) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !used.contains(&name) && !name.starts_with('.') {
                let _ = fs::remove_file(e.path());
            }
        }
    }
    Ok(board)
}

fn state_word(s: GateState) -> &'static str {
    match s {
        GateState::Pass => "pass",
        GateState::Fail => "fail",
        GateState::Stale => "stale",
        GateState::Unrun => "unrun",
        GateState::Unapproved => "unapproved",
        GateState::Interrupted => "interrupted",
    }
}

fn merge_word(s: MergeState) -> &'static str {
    match s {
        MergeState::Merged => "merged",
        MergeState::Clean => "clean",
        MergeState::Conflict => "conflict",
        MergeState::Error => "error",
    }
}

/// A code span that survives any text (backticks inside become quotes).
fn code(text: &str) -> String {
    format!("`{}`", text.replace('`', "'"))
}

fn gate_name(g: &GateVerdict) -> String {
    format!("{}#{}", g.doc, g.block)
}

fn heading(level: u8, id: String, text: String) -> Block {
    Block {
        kind: "heading".into(),
        id,
        level,
        text,
        ..Block::default()
    }
}

fn paragraph(id: String, text: String) -> Block {
    Block {
        kind: "paragraph".into(),
        id,
        text,
        ..Block::default()
    }
}

fn list(id: String, items: Vec<Item>) -> Block {
    Block {
        kind: "bulleted-list".into(),
        id,
        items,
        ..Block::default()
    }
}

fn item(text: String, nested: Vec<Item>) -> Item {
    Item {
        checked: false,
        text,
        nested,
    }
}

fn summary(gates: &[GateVerdict]) -> String {
    let pass = gates.iter().filter(|g| g.state == GateState::Pass).count();
    format!("gates {pass}/{} pass", gates.len())
}

/// The board as a document. `place` turns a screen's absolute path into the `src` the board
/// uses (or `None` when the file is not there), so building it touches nothing else.
pub fn document(entries: &[Entry], place: &mut dyn FnMut(&str) -> Option<String>) -> Document {
    let mut b = Vec::new();
    let newest = entries
        .iter()
        .filter_map(|e| e.snapshot.as_ref().ok().map(|s| s.updated_ms))
        .max();
    b.push(heading(1, "board".into(), "dx live board".into()));
    b.push(paragraph(
        "about".into(),
        format!(
            "{} watched repo{} · newest snapshot {}\n\
             Every branch is merged virtually with its base. Gates read **pass**, **fail**, \
             **interrupted**, **stale**, **unrun** or **unapproved**; a screen shows under its gate only while \
             that gate passes.",
            entries.len(),
            if entries.len() == 1 { "" } else { "s" },
            newest.map_or_else(|| "none yet".to_string(), iso),
        ),
    ));
    for (i, e) in entries.iter().enumerate() {
        let r = format!("r{}", i + 1);
        b.push(heading(2, r.clone(), code(&e.repo.display().to_string())));
        let s = match &e.snapshot {
            Ok(s) => s,
            Err(why) => {
                b.push(paragraph(
                    format!("{r}-none"),
                    format!("No snapshot: {why}"),
                ));
                continue;
            }
        };
        b.push(paragraph(
            format!("{r}-base"),
            format!(
                "**base** {}@{} · {} · {} branch{} · updated {}",
                code(&s.base),
                sha7(&s.base_sha),
                summary(&s.base_gates),
                s.branches.len(),
                if s.branches.len() == 1 { "" } else { "es" },
                iso(s.updated_ms)
            ),
        ));
        gates_section(&mut b, &format!("{r}-base"), &s.base_gates, place);
        for (j, br) in s.branches.iter().enumerate() {
            branch_section(&mut b, &format!("{r}-b{}", j + 1), s, br, place);
        }
    }
    Document {
        blocks: b,
        ..Document::default()
    }
}

fn branch_section(
    b: &mut Vec<Block>,
    id: &str,
    s: &Snapshot,
    br: &BranchState,
    place: &mut dyn FnMut(&str) -> Option<String>,
) {
    let word = merge_word(br.merge.state);
    b.push(heading(
        3,
        id.to_string(),
        format!("{} {word}", code(&br.branch)),
    ));
    let mut line = format!(
        "**{word}** {}@{} into {}",
        code(&br.branch),
        sha7(&br.sha),
        code(&s.base)
    );
    match br.merge.state {
        MergeState::Clean => {
            if br.checkout.is_none() {
                line.push_str(" · gates not run (no checkout)");
            } else {
                line.push_str(&format!(" · {}", summary(&br.gates)));
            }
        }
        MergeState::Conflict => {
            let n = br.merge.conflicts.len();
            line.push_str(&format!(
                " · conflict in {n} file{}",
                if n == 1 { "" } else { "s" }
            ));
        }
        MergeState::Merged => line.push_str(" · already in base"),
        MergeState::Error => {
            line.push_str(&format!(
                " · {}",
                br.merge.error.as_deref().unwrap_or("unknown error")
            ));
        }
    }
    if let Some(wt) = &br.worktree {
        line.push_str(&format!("\nworktree {}", code(wt)));
    }
    b.push(paragraph(format!("{id}-merge"), line));
    if br.merge.state == MergeState::Conflict {
        let items = br
            .merge
            .conflicts
            .iter()
            .map(|c| item(format!("conflict {}", code(&c.path)), vec![]))
            .collect();
        b.push(list(format!("{id}-conflicts"), items));
    }
    if !br.gates.is_empty() {
        gates_section(b, id, &br.gates, place);
    }
}

/// One list item per verdict state present (pass gates named inline, the rest one per line
/// with the tail of a failure), then each passing gate's screens under its own heading.
fn gates_section(
    b: &mut Vec<Block>,
    id: &str,
    gates: &[GateVerdict],
    place: &mut dyn FnMut(&str) -> Option<String>,
) {
    if gates.is_empty() {
        b.push(paragraph(format!("{id}-nogates"), "No gates.".into()));
        return;
    }
    let mut items = Vec::new();
    for state in [
        GateState::Fail,
        GateState::Interrupted,
        GateState::Stale,
        GateState::Unrun,
        GateState::Unapproved,
        GateState::Pass,
    ] {
        let of: Vec<&GateVerdict> = gates.iter().filter(|g| g.state == state).collect();
        if of.is_empty() {
            continue;
        }
        let head = format!("**{}** {}", state_word(state), of.len());
        if state == GateState::Pass {
            let names: Vec<String> = of.iter().map(|g| code(&gate_name(g))).collect();
            items.push(item(format!("{head}: {}", names.join(" ")), vec![]));
            continue;
        }
        let nested = of
            .iter()
            .map(|g| {
                let mut t = format!("{} {}", state_word(state), code(&gate_name(g)));
                if state == GateState::Fail && !g.tail.is_empty() {
                    t.push_str(&format!(": {}", code(&g.tail)));
                }
                if !g.images.is_empty() {
                    let n = g.images.len();
                    t.push_str(&format!(
                        " · {n} screen{} hidden until it passes",
                        if n == 1 { "" } else { "s" }
                    ));
                }
                item(t, vec![])
            })
            .collect();
        items.push(item(head, nested));
    }
    b.push(list(format!("{id}-gates"), items));
    for (k, g) in gates.iter().enumerate() {
        if g.state != GateState::Pass || g.images.is_empty() {
            continue;
        }
        let gid = format!("{id}-g{}", k + 1);
        b.push(heading(
            4,
            gid.clone(),
            format!("**pass** {} proves", code(&gate_name(g))),
        ));
        for (n, path) in g.images.iter().enumerate() {
            let shown = Path::new(path)
                .file_name()
                .map_or_else(|| path.clone(), |f| f.to_string_lossy().to_string());
            match place(path) {
                Some(src) => b.push(Block {
                    kind: "image".into(),
                    id: format!("{gid}-s{}", n + 1),
                    src,
                    alt: format!("{} · {shown}", gate_name(g)),
                    ..Block::default()
                }),
                None => b.push(paragraph(
                    format!("{gid}-s{}", n + 1),
                    format!("screen not found: {}", code(path)),
                )),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::merge::{Conflict, MergeStatus};

    fn gate(block: &str, state: GateState, images: &[&str]) -> GateVerdict {
        GateVerdict {
            doc: "t.dx".into(),
            block: block.into(),
            state,
            ms: None,
            tail: if state == GateState::Fail {
                "boom".into()
            } else {
                String::new()
            },
            images: images.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn branch(name: &str, state: MergeState, gates: Vec<GateVerdict>) -> BranchState {
        BranchState {
            branch: name.into(),
            sha: "bbbbbbbbbb".into(),
            worktree: None,
            merge: MergeStatus {
                state,
                base_sha: "a".into(),
                branch_sha: "b".into(),
                tree: None,
                conflicts: if state == MergeState::Conflict {
                    vec![Conflict {
                        path: "x.txt".into(),
                        hunks: "<<<<<<<".into(),
                    }]
                } else {
                    vec![]
                },
                error: None,
            },
            checkout: Some("/c".into()),
            gates,
        }
    }

    #[test]
    fn screens_show_under_passing_gates_only_and_every_branch_is_named() {
        let s = Snapshot {
            v: 1,
            repo: "/r".into(),
            base: "main".into(),
            base_sha: "aaaaaaaaaa".into(),
            updated_ms: 0,
            base_gates: vec![gate("ok", GateState::Pass, &["/r/ok.png"])],
            branches: vec![
                branch(
                    "b-pass",
                    MergeState::Clean,
                    vec![gate("ok", GateState::Pass, &["/c/ok.png"])],
                ),
                branch(
                    "b-fail",
                    MergeState::Clean,
                    vec![gate("ok", GateState::Fail, &["/c/ok.png"])],
                ),
                branch("b-conflict", MergeState::Conflict, vec![]),
                branch("b-merged", MergeState::Merged, vec![]),
            ],
        };
        let mut asked = Vec::new();
        let mut place = |p: &str| {
            asked.push(p.to_string());
            Some(format!("img/{}", asked.len()))
        };
        let doc = document(
            &[Entry {
                repo: "/r".into(),
                snapshot: Ok(s),
            }],
            &mut place,
        );
        assert_eq!(
            asked,
            vec!["/r/ok.png", "/c/ok.png"],
            "only passing gates' screens are placed"
        );
        let text = doc_core::format::stringify(&doc);
        for needle in [
            "`b-pass` clean",
            "`b-fail` clean",
            "`b-conflict` conflict",
            "`b-merged` merged",
            "conflict `x.txt`",
            "fail `t.dx#ok`: `boom` · 1 screen hidden until it passes",
        ] {
            assert!(text.contains(needle), "{needle} missing in\n{text}");
        }
        // Parses back to the same blocks: the board is an ordinary document.
        assert_eq!(
            doc_core::format::parse(&text).blocks.len(),
            doc.blocks.len()
        );
    }

    #[test]
    fn write_copies_screens_once_and_prunes_the_rest() {
        let root = std::env::temp_dir().join(format!("dxboard-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let shot = root.join("shot.png");
        fs::write(&shot, b"\x89PNG fake").unwrap();
        let s = Snapshot {
            v: 1,
            repo: root.display().to_string(),
            base: "main".into(),
            base_sha: "a".into(),
            updated_ms: 0,
            base_gates: vec![gate("ok", GateState::Pass, &[&shot.display().to_string()])],
            branches: vec![],
        };
        let out = root.join("board");
        fs::create_dir_all(out.join(IMG)).unwrap();
        fs::write(out.join(IMG).join("old.png"), b"x").unwrap();
        let entries = [Entry {
            repo: root.clone(),
            snapshot: Ok(s),
        }];
        let board = write(&out, &entries).unwrap();
        let imgs: Vec<_> = fs::read_dir(out.join(IMG)).unwrap().flatten().collect();
        assert_eq!(imgs.len(), 1, "old copy pruned, one screen copied");
        let text = fs::read_to_string(&board).unwrap();
        assert!(text.contains("src=img/"), "{text}");
        let before = fs::metadata(&board).unwrap().modified().unwrap();
        write(&out, &entries).unwrap();
        assert_eq!(
            fs::metadata(&board).unwrap().modified().unwrap(),
            before,
            "unchanged board not rewritten"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn common_dir_reads_git_files_for_a_linked_worktree() {
        let root = std::env::temp_dir().join(format!("dxboard-cd-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("main/.git/worktrees/w")).unwrap();
        fs::create_dir_all(root.join("w")).unwrap();
        fs::write(
            root.join("w/.git"),
            format!("gitdir: {}\n", root.join("main/.git/worktrees/w").display()),
        )
        .unwrap();
        fs::write(root.join("main/.git/worktrees/w/commondir"), "../..\n").unwrap();
        let c = common_dir(&root.join("w")).unwrap();
        assert_eq!(
            fs::canonicalize(c).unwrap(),
            fs::canonicalize(root.join("main/.git")).unwrap()
        );
        assert_eq!(
            common_dir(&root.join("main")).unwrap(),
            root.join("main/.git")
        );
        let _ = fs::remove_dir_all(&root);
    }
}
