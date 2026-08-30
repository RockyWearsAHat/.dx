//! Integration test: Verify the Rust workspace scaffold is properly configured.
//! This ensures the workspace remains valid as the project evolves.

use std::process::Command;

#[test]
fn cargo_metadata_recognizes_all_workspace_members() {
    // GREEN: cargo metadata must work and report all workspace members.
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1"])
        .current_dir(workspace_root())
        .output()
        .expect("cargo metadata must run");

    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let metadata = String::from_utf8_lossy(&output.stdout);
    // The workspace must report at least one member.
    // A real workspace will have several; the count is validated in index.dx.
    assert!(
        metadata.contains("\"workspace_members\""),
        "metadata missing workspace_members field"
    );
    assert!(
        !metadata.contains("\"workspace_members\":[]"),
        "workspace has no members"
    );
}

#[test]
fn all_workspace_member_crates_have_source_directories() {
    // GREEN: Every crate listed in Cargo.toml must have a src/ directory.
    let root = workspace_root();
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).expect("Cargo.toml exists");

    // Parse the members list from the manifest.
    for line in manifest.lines() {
        if line.contains("members = [") {
            // Example: members = ["doc-cli", "doc-core", ...]
            let members_str = line
                .trim_start_matches("members = [")
                .trim_end_matches("]")
                .trim_matches('"');

            for member in members_str.split(",") {
                let crate_name = member.trim().trim_matches('"');
                let crate_path = root.join(crate_name);
                let src_dir = crate_path.join("src");

                assert!(
                    src_dir.is_dir(),
                    "Crate '{}' listed in members has no src/ directory at {}",
                    crate_name,
                    src_dir.display()
                );
            }
            break;
        }
    }
}

#[test]
fn workspace_builds_with_no_dependency_gaps() {
    // GREEN: cargo check must pass, verifying no dependency gaps or unresolved references.
    let output = Command::new("cargo")
        .args(["check", "--all"])
        .env("GOWORK", "off")
        .current_dir(workspace_root())
        .output()
        .expect("cargo check must run");

    assert!(
        output.status.success(),
        "cargo check failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn workspace_dependency_graph_is_acyclic() {
    // GREEN: cargo tree must succeed, indicating no circular dependencies.
    let output = Command::new("cargo")
        .args(["tree", "--depth", "10"])
        .env("GOWORK", "off")
        .current_dir(workspace_root())
        .output()
        .expect("cargo tree must run");

    assert!(
        output.status.success(),
        "cargo tree failed (circular dependency?): {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // tree output should list all members.
    let tree = String::from_utf8_lossy(&output.stdout);
    let expected_crates = vec![
        "doc-cli",
        "doc-core",
        "doc-run",
        "doc-shot",
        "doc-store",
        "doc-wasm",
    ];
    for crate_name in expected_crates {
        assert!(
            tree.contains(&format!("{} v", crate_name)),
            "cargo tree missing workspace member: {}",
            crate_name
        );
    }
}

/// Find the Rust workspace root (the directory containing Cargo.toml at workspace level).
fn workspace_root() -> std::path::PathBuf {
    // CARGO_MANIFEST_DIR points to the crate's directory (doc-cli).
    // The workspace root is the parent of that (rust/).
    let manifest_dir =
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set by cargo");
    let crate_dir = std::path::PathBuf::from(manifest_dir);

    // Walk up to find the workspace root.
    let mut path = crate_dir.clone();
    loop {
        if path.join("Cargo.toml").exists() {
            let content = std::fs::read_to_string(path.join("Cargo.toml")).unwrap_or_default();
            if content.contains("[workspace]") {
                return path;
            }
        }
        if !path.pop() {
            panic!(
                "Could not find workspace root (no Cargo.toml with [workspace]) starting from {}",
                crate_dir.display()
            );
        }
    }
}
