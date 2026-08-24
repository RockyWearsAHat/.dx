//! Archive validity for extension stores.
//!
//! The Chrome Web Store and Mozilla Add-ons require archives with no resource forks,
//! parsable manifests, and version advancement. This test shells out to the verification script
//! to ensure archives pass preflight checks before uploading.

use std::process::Command;

#[test]
fn archives_pass_store_preflight() {
    let current = std::env::current_dir().expect("current directory");
    let parent = current.parent().expect("parent directory");
    let root = parent.parent().expect("project root").to_path_buf();

    let verify_script = root.join("packaging/verify-archives.sh");
    assert!(
        verify_script.exists(),
        "verify-archives.sh not found at {}",
        verify_script.display()
    );

    let output = Command::new("bash")
        .arg(&verify_script)
        .current_dir(&root)
        .output()
        .expect("verify-archives.sh runs");

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        panic!(
            "verify-archives.sh failed:\nstdout: {}\nstderr: {}",
            stdout, stderr
        );
    }
}
