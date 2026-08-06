//! The approval ledger: which code fingerprints this machine has agreed to run.
//!
//! `dx run` refuses to execute a block whose fingerprint it has never seen approved —
//! new or edited code is inspected first (`--review`), then approved (`--approve`).
//! The ledger records those decisions as one marker file per fingerprint, under the
//! same local cache root that holds the per-block run state ([`crate::workdir`]), so
//! it lives beside the machine's other run bookkeeping and never inside a repository.
//! Nothing here is committed, synced, or shared: an approval is this machine's own.
//!
//! An approval names a fingerprint, not a block — the fingerprint covers the runner,
//! the exact code, the declared dependencies, and the current text of every declared
//! `reads=` file, so editing any of those produces a new fingerprint and the approval
//! expires with the edit. There is nothing to revoke by hand and nothing to keep
//! current: stale approvals simply stop matching.

use std::fs;
use std::path::{Path, PathBuf};

/// Directory under the cache root holding one marker file per approved fingerprint.
const LEDGER_DIR: &str = "approvals";

/// The local record of approved run fingerprints.
///
/// Cheap to construct — nothing is read or created until a fingerprint is looked up
/// or recorded, so building a ledger for a document with nothing to run costs nothing.
#[derive(Debug, Clone)]
pub struct Ledger {
    dir: PathBuf,
}

impl Ledger {
    /// The ledger stored under `cache_root`, beside the per-block run directories.
    #[must_use]
    pub fn at(cache_root: &Path) -> Self {
        Self {
            dir: cache_root.join(LEDGER_DIR),
        }
    }

    /// Whether `fingerprint` has been approved on this machine.
    ///
    /// A fingerprint the ledger could not possibly have written (anything but the
    /// hex a run fingerprint is made of) is never approved.
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
                "`{fingerprint}` is not a run fingerprint, so it cannot be approved."
            ));
        }
        fs::create_dir_all(&self.dir)
            .map_err(|error| format!("could not create {}: {error}", self.dir.display()))?;
        let marker = self.dir.join(fingerprint);
        fs::write(&marker, b"approved\n")
            .map_err(|error| format!("could not record approval {}: {error}", marker.display()))
    }
}

/// Whether `fingerprint` is shaped like one [`crate`] produces: non-empty lowercase hex.
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
        let dir = std::env::temp_dir().join(format!("dx-approvals-tests-{label}"));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_recorded_approval_is_found_and_an_unrecorded_one_is_not() {
        let ledger = Ledger::at(&scratch("record"));
        assert!(!ledger.is_approved("abcdef0123456789"));
        ledger.approve("abcdef0123456789").expect("approve");
        assert!(ledger.is_approved("abcdef0123456789"));
        assert!(!ledger.is_approved("ffffffffffffffff"));
    }

    #[test]
    fn approvals_survive_a_new_ledger_over_the_same_cache() {
        let root = scratch("survive");
        Ledger::at(&root)
            .approve("00aa11bb22cc33dd")
            .expect("approve");
        assert!(Ledger::at(&root).is_approved("00aa11bb22cc33dd"));
    }

    #[test]
    fn a_value_that_is_not_a_fingerprint_is_refused_not_written() {
        let ledger = Ledger::at(&scratch("malformed"));
        for bad in ["", "../escape", "ABCDEF", "abc/def", "run this"] {
            assert!(ledger.approve(bad).is_err(), "accepted `{bad}`");
            assert!(!ledger.is_approved(bad));
        }
    }
}
