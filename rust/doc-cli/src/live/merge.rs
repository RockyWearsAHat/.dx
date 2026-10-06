//! Virtual merge of a branch into base: `git merge-tree --write-tree`, no checkout touched.
//! Git failures come back as data (`MergeState::Error`), never panics or prompts.

use std::path::Path;
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MergeState {
    Merged,
    Clean,
    Conflict,
    Error,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Conflict {
    pub path: String,
    pub hunks: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MergeStatus {
    pub state: MergeState,
    pub base_sha: String,
    pub branch_sha: String,
    pub tree: Option<String>,
    pub conflicts: Vec<Conflict>,
    pub error: Option<String>,
}

const MAX_CONFLICTS: usize = 20;
const MAX_HUNK_CHARS: usize = 4000;
const CONTEXT: usize = 3;

struct Out {
    code: Option<i32>,
    stdout: Vec<u8>,
    stderr: String,
}

fn git(repo: &Path, args: &[&str]) -> Result<Out, String> {
    let o = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("could not run git: {e}"))?;
    Ok(Out {
        code: o.status.code(),
        stdout: o.stdout,
        stderr: String::from_utf8_lossy(&o.stderr).trim().to_string(),
    })
}

fn rev_parse(repo: &Path, name: &str) -> Result<String, String> {
    let spec = format!("{name}^{{commit}}");
    let o = git(repo, &["rev-parse", "--verify", "--quiet", &spec])?;
    if o.code == Some(0) {
        Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
    } else if o.stderr.is_empty() {
        Err(format!("git rev-parse: no commit named '{name}'"))
    } else {
        Err(o.stderr)
    }
}

fn err(base_sha: String, branch_sha: String, msg: String) -> MergeStatus {
    MergeStatus {
        state: MergeState::Error,
        base_sha,
        branch_sha,
        tree: None,
        conflicts: vec![],
        error: Some(msg),
    }
}

/// What merging `branch` into `base` would do, without touching any checkout.
pub fn status(repo: &Path, base: &str, branch: &str) -> MergeStatus {
    let base_sha = match rev_parse(repo, base) {
        Ok(s) => s,
        Err(e) => return err(String::new(), String::new(), e),
    };
    let branch_sha = match rev_parse(repo, branch) {
        Ok(s) => s,
        Err(e) => return err(base_sha, String::new(), e),
    };
    let mk = |state, tree: Option<String>, conflicts| MergeStatus {
        state,
        base_sha: base_sha.clone(),
        branch_sha: branch_sha.clone(),
        tree,
        conflicts,
        error: None,
    };

    match git(repo, &["merge-base", "--is-ancestor", &branch_sha, &base_sha]) {
        Ok(o) if o.code == Some(0) => return mk(MergeState::Merged, None, vec![]),
        Ok(o) if o.code == Some(1) => {}
        Ok(o) => return err(base_sha.clone(), branch_sha.clone(), format!("git merge-base: {}", o.stderr)),
        Err(e) => return err(base_sha.clone(), branch_sha.clone(), e),
    }

    let o = match git(repo, &["merge-tree", "--write-tree", "--messages", &base_sha, &branch_sha]) {
        Ok(o) => o,
        Err(e) => return err(base_sha.clone(), branch_sha.clone(), e),
    };
    let text = String::from_utf8_lossy(&o.stdout).to_string();
    let tree = text.lines().next().unwrap_or("").trim().to_string();
    match o.code {
        Some(0) if !tree.is_empty() => mk(MergeState::Clean, Some(tree), vec![]),
        Some(1) if !tree.is_empty() => {
            let mut paths: Vec<String> = Vec::new();
            for line in text.lines().skip(1) {
                if line.is_empty() {
                    break;
                }
                if let Some((_, path)) = line.split_once('\t') {
                    if !paths.iter().any(|p| p == path) {
                        paths.push(path.to_string());
                    }
                }
            }
            let conflicts = paths
                .into_iter()
                .take(MAX_CONFLICTS)
                .map(|path| {
                    let hunks = match git(repo, &["cat-file", "-p", &format!("{tree}:{path}")]) {
                        Ok(b) if b.code == Some(0) => hunks_of(&String::from_utf8_lossy(&b.stdout)),
                        _ => String::new(),
                    };
                    Conflict { path, hunks }
                })
                .collect();
            mk(MergeState::Conflict, Some(tree), conflicts)
        }
        _ => {
            let msg = if o.stderr.is_empty() {
                format!("git merge-tree exited with {:?}", o.code)
            } else {
                format!("git merge-tree: {}", o.stderr)
            };
            err(base_sha.clone(), branch_sha.clone(), msg)
        }
    }
}

/// Regions from `<<<<<<<` to `>>>>>>>` with 3 lines of context each side, merged when they
/// overlap, capped at 4000 chars.
fn hunks_of(content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    let mut start: Option<usize> = None;
    for (i, l) in lines.iter().enumerate() {
        if l.starts_with("<<<<<<<") {
            start = Some(i);
        } else if l.starts_with(">>>>>>>") {
            if let Some(s) = start.take() {
                let a = s.saturating_sub(CONTEXT);
                let b = (i + CONTEXT).min(lines.len() - 1);
                match ranges.last_mut() {
                    Some(last) if a <= last.1 + 1 => last.1 = last.1.max(b),
                    _ => ranges.push((a, b)),
                }
            }
        }
    }
    let out = ranges
        .iter()
        .map(|&(a, b)| lines[a..=b].join("\n"))
        .collect::<Vec<_>>()
        .join("\n...\n");
    if out.chars().count() > MAX_HUNK_CHARS {
        let mut cut: String = out.chars().take(MAX_HUNK_CHARS).collect();
        cut.push('…');
        cut
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn g(dir: &Path, args: &[&str]) {
        let o = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
            .args(args)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    }

    fn repo(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("dx-live-merge-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        g(&d, &["init", "-q", "-b", "main"]);
        std::fs::write(d.join("a.txt"), "1\n2\n3\n4\n5\n6\n7\n").unwrap();
        g(&d, &["add", "."]);
        g(&d, &["commit", "-q", "-m", "init"]);
        d
    }

    fn commit_file(d: &Path, name: &str, body: &str) {
        std::fs::write(d.join(name), body).unwrap();
        g(d, &["add", "."]);
        g(d, &["commit", "-q", "-m", name]);
    }

    #[test]
    fn clean() {
        let d = repo("clean");
        g(&d, &["checkout", "-q", "-b", "feat"]);
        commit_file(&d, "b.txt", "b\n");
        g(&d, &["checkout", "-q", "main"]);
        commit_file(&d, "c.txt", "c\n");
        let s = status(&d, "main", "feat");
        assert_eq!(s.state, MergeState::Clean, "{s:?}");
        assert!(s.tree.is_some() && s.conflicts.is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn conflict() {
        let d = repo("conflict");
        g(&d, &["checkout", "-q", "-b", "feat"]);
        commit_file(&d, "a.txt", "1\n2\n3\nfeat\n5\n6\n7\n");
        g(&d, &["checkout", "-q", "main"]);
        commit_file(&d, "a.txt", "1\n2\n3\nmain\n5\n6\n7\n");
        let s = status(&d, "main", "feat");
        assert_eq!(s.state, MergeState::Conflict, "{s:?}");
        assert_eq!(s.conflicts.len(), 1);
        assert_eq!(s.conflicts[0].path, "a.txt");
        assert!(s.conflicts[0].hunks.contains("<<<<<<<"), "{}", s.conflicts[0].hunks);
        assert!(s.conflicts[0].hunks.contains(">>>>>>>"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn merged() {
        let d = repo("merged");
        g(&d, &["checkout", "-q", "-b", "feat"]);
        commit_file(&d, "b.txt", "b\n");
        g(&d, &["checkout", "-q", "main"]);
        g(&d, &["merge", "-q", "--no-ff", "-m", "merge", "feat"]);
        let s = status(&d, "main", "feat");
        assert_eq!(s.state, MergeState::Merged, "{s:?}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn bad_ref() {
        let d = repo("bad");
        let s = status(&d, "main", "nope");
        assert_eq!(s.state, MergeState::Error);
        assert!(s.error.is_some());
        let s = status(&d, "nope", "main");
        assert_eq!(s.state, MergeState::Error);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn hunk_cap() {
        let big = format!("<<<<<<< a\n{}\n=======\nx\n>>>>>>> b\n", "y".repeat(9000));
        let h = hunks_of(&big);
        assert!(h.ends_with('…') && h.chars().count() == MAX_HUNK_CHARS + 1);
    }
}
