//! The rename approval ledger: which renames this machine has approved.
//!
//! `dx rename` refuses to write changes for a rename whose fingerprint it has never
//! seen approved — the fingerprint is computed from the old name, new name, sorted list
//! of sites, and the reference graph digest, so a rename over a stale graph will not
//! silently approve a different set of changes.
//!
//! The ledger records those decisions as one marker file per fingerprint, under a
//! local cache directory, so it lives beside the machine's other bookkeeping and never
//! inside a repository. Nothing here is committed, synced, or shared: an approval is
//! this machine's own.
//!
//! An approval is distinct from the code-run approvals ledger: it covers a structural
//! write operation (a rename at a specific set of sites over a specific graph state),
//! not a runnable block.

use std::fs;
use std::path::{Path, PathBuf};

/// Directory under the cache root holding one marker file per approved rename fingerprint.
const RENAME_LEDGER_DIR: &str = "rename_approvals";

/// The local record of approved rename fingerprints.
///
/// Cheap to construct — nothing is read or created until a fingerprint is looked up
/// or recorded.
#[derive(Debug, Clone)]
pub struct RenameLedger {
    dir: PathBuf,
}

impl RenameLedger {
    /// The ledger stored under `cache_root`, beside the per-block run directories.
    #[must_use]
    pub fn at(cache_root: &Path) -> Self {
        Self {
            dir: cache_root.join(RENAME_LEDGER_DIR),
        }
    }

    /// Whether `fingerprint` has been approved on this machine.
    ///
    /// A fingerprint the ledger could not possibly have written (anything but the
    /// hex a rename fingerprint is made of) is never approved.
    #[must_use]
    pub fn is_approved(&self, fingerprint: &str) -> bool {
        well_formed(fingerprint) && self.dir.join(fingerprint).exists()
    }

    /// Record `fingerprint` as approved.
    ///
    /// # Errors
    /// Returns a sentence naming what could not be written — a decision the reader
    /// made must never be dropped silently — or refusing a malformed fingerprint,
    /// which would otherwise become a stray file path.
    pub fn approve(&self, fingerprint: &str) -> Result<(), String> {
        if !well_formed(fingerprint) {
            return Err(format!(
                "`{fingerprint}` is not a rename fingerprint, so it cannot be approved."
            ));
        }
        fs::create_dir_all(&self.dir)
            .map_err(|error| format!("could not create {}: {error}", self.dir.display()))?;
        let marker = self.dir.join(fingerprint);
        fs::write(&marker, b"approved\n")
            .map_err(|error| format!("could not record approval {}: {error}", marker.display()))
    }
}

/// Whether `fingerprint` is shaped like one a rename produces: non-empty lowercase hex.
///
/// The fingerprint becomes a file name, so this is also what keeps a crafted value
/// from naming a path outside the ledger.
fn well_formed(fingerprint: &str) -> bool {
    !fingerprint.is_empty()
        && fingerprint
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dx-rename-approvals-tests-{label}"));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_recorded_approval_is_found_and_an_unapproved_one_is_not() {
        let ledger = RenameLedger::at(&scratch("record"));
        assert!(!ledger.is_approved("abcdef0123456789"));
        ledger.approve("abcdef0123456789").expect("approve");
        assert!(ledger.is_approved("abcdef0123456789"));
        assert!(!ledger.is_approved("ffffffffffffffff"));
    }

    #[test]
    fn approval_rejects_malformed_fingerprints() {
        let ledger = RenameLedger::at(&scratch("malformed"));
        let err = ledger.approve("NOT-HEX").expect_err("should reject uppercase");
        assert!(err.contains("not a rename fingerprint"));
    }

    #[test]
    fn approval_rejects_empty_fingerprint() {
        let ledger = RenameLedger::at(&scratch("empty"));
        let err = ledger.approve("").expect_err("should reject empty");
        assert!(err.contains("not a rename fingerprint"));
    }
}
