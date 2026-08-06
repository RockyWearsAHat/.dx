//! The fixture corpus is hermetic, and this keeps it honest.
//!
//! `doc-core`'s round-trip assertions read `tests/fixtures/*.input.dx` rather than the
//! documents in `examples/` and `documents/`. Those working-tree documents live in the
//! workspace store — one-line pointers on disk, content in `.doc/repo.dxcp` — which is
//! exactly what this crate's `include_str!` corpus must never become: a suite that compiled
//! pointers would stop testing documents. The store's walker therefore never adopts a
//! `fixtures` directory, and these copies stay plain text.
//!
//! Drift between a fixture and the stored document it mirrors is caught in `doc-cli`
//! (`workspace::tests::every_fixture_input_still_matches_the_document_it_mirrors`), the
//! crate that can resolve a pointer. This test only pins that the corpus is complete.

use std::path::Path;

/// Every fixture input the round-trip suite compiles in, and the document it mirrors.
const MIRRORS: &[(&str, &str)] = &[
    ("welcome.input.dx", "examples/welcome.dx"),
    ("tutorial.input.dx", "examples/tutorial.dx"),
    ("showcase.input.dx", "examples/showcase.dx"),
    ("block-reference.input.dx", "examples/block-reference.dx"),
    (
        "compactness-comparison.input.dx",
        "examples/compactness-comparison.dx",
    ),
    ("footprint-pair.input.dx", "examples/footprint-pair.dx"),
    ("compact-proof.input.dx", "documents/compact-proof.dx"),
];

#[test]
fn the_fixture_corpus_is_complete() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    for (fixture, source) in MIRRORS {
        assert!(
            fixtures.join(fixture).is_file(),
            "{fixture} (mirroring {source}) is not in the fixture directory"
        );
    }
}
