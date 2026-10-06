//! `dx live` — read or refresh the live snapshot of every branch against base.

use std::path::PathBuf;

use crate::args::Args;
use crate::live;

/// `dx live [--repo DIR] [--base B] [--once] [--json] [--no-run] [--add DIR] [--prune]`.
pub fn run(args: &Args) -> Result<String, String> {
    let start = args
        .value("repo")
        .map_or_else(|| std::env::current_dir().unwrap_or_default(), PathBuf::from);
    let repo = live::repo_of(&start);

    if let Some(dir) = args.value("add") {
        let added = live::watch::add_repo(&PathBuf::from(dir))?;
        return Ok(format!("watching {}\n", added.display()));
    }
    if args.present("prune") {
        live::checkout::prune(&repo, &[])?;
        return Ok(format!("pruned dx live checkouts of {}\n", repo.display()));
    }
    let base = args
        .value("base")
        .map_or_else(|| live::default_base(&repo), str::to_string);
    if args.present("once") {
        let snapshot = live::refresh(&repo, &base, !args.present("no-run"))?;
        live::write(&repo, &snapshot)?;
        return if args.present("json") {
            serde_json::to_string_pretty(&snapshot).map_err(|e| e.to_string())
        } else {
            Ok(live::render_text(&snapshot))
        };
    }
    if args.present("json") {
        live::read_json(&repo)
    } else {
        live::read_text(&repo)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_a_snapshot_the_read_says_so() {
        let dir = std::env::temp_dir().join(format!("dxlive-cmd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let raw = vec!["--repo".to_string(), dir.display().to_string()];
        let err = run(&Args::parse(&raw)).unwrap_err();
        assert!(err.contains("no snapshot"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
