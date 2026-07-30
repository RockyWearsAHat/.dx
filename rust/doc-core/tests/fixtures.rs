//! The fixture corpus is hermetic, and this keeps it honest.
//!
//! `doc-core`'s round-trip assertions read `tests/fixtures/*.input.dx` rather than the
//! documents in `examples/` and `documents/`. That matters because those working-tree
//! documents are exactly what the store is designed to convert into one-line pointers: a
//! `dx sync` at the repository root would replace them, and a test suite that compiled them
//! with `include_str!` would stop testing documents and start testing pointers.
//!
//! The copies therefore have to stay copies. This test compares each fixture input against
//! the document it mirrors and fails when they drift, so refreshing an example is a
//! deliberate two-file change rather than a silent divergence.

use std::fs;
use std::path::{Path, PathBuf};

/// Each fixture input and the working-tree document it mirrors.
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

/// The repository root, two directories above this crate.
fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate sits two levels below the repository root")
        .to_path_buf()
}

#[test]
fn every_fixture_input_still_matches_the_document_it_mirrors() {
    let root = repository_root();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");

    for (fixture, source) in MIRRORS {
        let original = root.join(source);
        let Ok(expected) = fs::read_to_string(&original) else {
            // The working-tree document is gone or is now a pointer. Either way the fixture
            // is the surviving copy and there is nothing to compare against.
            continue;
        };
        // A pointer is not a document; skip rather than fail if the workspace was converted.
        if expected.starts_with("~ dx1 ") {
            continue;
        }

        let copy = fs::read_to_string(fixtures.join(fixture))
            .unwrap_or_else(|error| panic!("fixture {fixture} is missing: {error}"));
        assert_eq!(
            copy, expected,
            "tests/fixtures/{fixture} has drifted from {source}; copy the document over it \
             and re-check the .expected.dx output"
        );
    }
}

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
