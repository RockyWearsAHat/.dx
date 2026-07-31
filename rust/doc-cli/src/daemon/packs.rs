//! The packs the daemon is holding in memory.
//!
//! This is the reason the daemon exists. Before it, a reader opening a pull request that
//! touched six documents made the browser download `.doc/repo.dxcp` twelve times — once per
//! file per side — decompress it twelve times, and hand the whole thing across a process
//! boundary as base64 on every single call. Nothing was ever kept, because the only place to
//! keep it was a service worker the browser shuts down after seconds of idleness.
//!
//! Here a pack is fetched once, decoded once, and then answered from memory for as long as
//! the daemon is running. A pack is identified by an opaque key the client chooses; the
//! browser uses the URL it fetched the bytes from.
//!
//! # That key can go stale, and the client is what notices
//! The URL names a *ref*, which is a commit id on a permalink but often a branch name
//! (`…/raw/main/.doc/repo.dxcp`) — and a branch moves. This cache has no expiry and cannot
//! have one: it has no way to ask github.com whether `main` still points where it did.
//!
//! So staleness is caught where the truth is known. Every document the extension displays is
//! verified against the digest in its pointer, and a mismatch makes it refetch the pack and
//! store it here again under the same key, which replaces it. Storing is therefore the
//! refresh mechanism as well as the arrival mechanism, and it is why [`Packs::store`] must
//! keep overwriting rather than skipping a key it already holds.

use std::collections::HashMap;

use doc_core::chunk::Pack;

/// How many decoded packs to hold before the least recently used one is dropped.
///
/// Reading spans a handful of repositories at a time, and a dropped pack costs one re-upload,
/// never a wrong answer.
const CAPACITY: usize = 12;

/// Decoded packs, keyed by whatever the client calls them, with the least recently used
/// dropped once [`CAPACITY`] is exceeded.
///
/// Invariant: `order` holds exactly the keys of `packs`, least recently used first.
#[derive(Debug, Default)]
pub struct Packs {
    /// The decoded packs, by key.
    packs: HashMap<String, Pack>,
    /// The keys of `packs`, least recently used first.
    order: Vec<String>,
}

impl Packs {
    /// An empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Decode `bytes` and hold the result under `key`, replacing any pack already there.
    ///
    /// Returns the paths the pack carries, which is what a client wants to know about a pack
    /// it has just handed over.
    ///
    /// # Errors
    /// When `bytes` are not a `DXCP1` pack. Nothing is stored in that case, so a corrupt
    /// upload cannot displace a good pack already held under the same key.
    pub fn store(&mut self, key: &str, bytes: &[u8]) -> Result<Vec<String>, String> {
        let pack = doc_core::chunk::decode_pack(bytes)
            .map_err(|error| format!("that is not a dx pack: {error:?}"))?;
        let paths = pack
            .entries
            .iter()
            .map(|entry| entry.path.clone())
            .collect();

        self.order.retain(|held| held != key);
        self.packs.insert(key.to_string(), pack);
        self.order.push(key.to_string());
        while self.order.len() > CAPACITY {
            let evicted = self.order.remove(0);
            self.packs.remove(&evicted);
        }
        Ok(paths)
    }

    /// The pack held under `key`, marking it as the most recently used.
    pub fn get(&mut self, key: &str) -> Option<&Pack> {
        if !self.packs.contains_key(key) {
            return None;
        }
        self.order.retain(|held| held != key);
        self.order.push(key.to_string());
        self.packs.get(key)
    }

    /// How many packs are held, which is what `GET /health` reports.
    #[must_use]
    pub fn len(&self) -> usize {
        self.packs.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use doc_core::chunk::Pack as CorePack;

    /// A real encoded pack holding one document at `path`.
    fn pack_bytes(path: &str, source: &str) -> Vec<u8> {
        let document = doc_core::format::parse(source);
        let pack = CorePack::build([(path, &document)]);
        doc_core::chunk::encode_pack(&pack)
    }

    #[test]
    fn a_stored_pack_reports_its_paths_and_answers_for_them() {
        let mut packs = Packs::new();
        let paths = packs
            .store(
                "repo@main",
                &pack_bytes("notes.dx", "::heading level=1\nHello\n"),
            )
            .expect("a real pack decodes");
        assert_eq!(paths, vec!["notes.dx".to_string()]);
        let source = packs
            .get("repo@main")
            .and_then(|pack| pack.source("notes.dx"))
            .expect("the stored pack holds the document");
        assert!(source.contains("Hello"), "{source}");
    }

    #[test]
    fn an_unknown_key_is_a_miss_rather_than_a_wrong_document() {
        let mut packs = Packs::new();
        packs
            .store("repo@main", &pack_bytes("notes.dx", "::text\nOne\n"))
            .expect("a real pack decodes");
        assert!(packs.get("repo@other").is_none());
    }

    #[test]
    fn corrupt_bytes_are_refused_and_leave_the_held_pack_alone() {
        let mut packs = Packs::new();
        packs
            .store("repo@main", &pack_bytes("notes.dx", "::text\nOne\n"))
            .expect("a real pack decodes");
        let error = packs
            .store("repo@main", b"not a pack at all")
            .expect_err("corrupt bytes are refused");
        assert!(error.contains("not a dx pack"), "{error}");
        assert!(
            packs.get("repo@main").is_some(),
            "a refused upload must not evict the good pack"
        );
    }

    #[test]
    fn the_least_recently_used_pack_is_the_one_dropped() {
        let mut packs = Packs::new();
        for index in 0..CAPACITY {
            packs
                .store(
                    &format!("repo@{index}"),
                    &pack_bytes("notes.dx", "::text\nOne\n"),
                )
                .expect("a real pack decodes");
        }
        // Touch the oldest so it is no longer the least recently used.
        assert!(packs.get("repo@0").is_some());
        packs
            .store("repo@new", &pack_bytes("notes.dx", "::text\nOne\n"))
            .expect("a real pack decodes");

        assert_eq!(packs.len(), CAPACITY);
        assert!(packs.get("repo@0").is_some(), "the touched pack survives");
        assert!(
            packs.get("repo@1").is_none(),
            "the untouched oldest is dropped"
        );
    }

    #[test]
    fn storing_the_same_key_twice_holds_one_pack_not_two() {
        let mut packs = Packs::new();
        let bytes = pack_bytes("notes.dx", "::text\nOne\n");
        packs
            .store("repo@main", &bytes)
            .expect("a real pack decodes");
        packs
            .store("repo@main", &bytes)
            .expect("a real pack decodes");
        assert_eq!(packs.len(), 1);
    }
}
