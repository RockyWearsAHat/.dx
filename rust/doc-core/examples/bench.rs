//! Throughput micro-benchmark for the `doc-core` engine, using only `std::time`.
//!
//! This is a *measurement* example, not a test: it times each core operation
//! (`digest`, `compress`/`decompress`, `docbin` pack/unpack, `format` parse/stringify,
//! `bundle` encode/decode, and `search` build/query) on representative inputs and prints
//! a fixed-width table of nanoseconds-per-op plus MB/s where a byte rate makes sense.
//!
//! It deliberately avoids any external benchmark crate (no Criterion): a simple warm-up
//! followed by an averaged loop over many iterations gives stable enough numbers to put
//! in `ANALYSIS.md`. Run with:
//!
//! ```text
//! cargo run --release -p doc-core --example bench
//! ```
//!
//! The absolute numbers depend on the host; the example is checked in so the table can be
//! regenerated on any machine.

use std::time::Instant;

use doc_core::bundle::{BundleEntry, GitFlags};
use doc_core::model::Document;
use doc_core::{bundle, compress, digest, docbin, format, search};

/// One real example document, embedded so the bench needs no filesystem access.
const WELCOME_DX: &str = include_str!("../../../examples/welcome.dx");
const TUTORIAL_DX: &str = include_str!("../../../examples/tutorial.dx");
const BLOCK_REF_DX: &str = include_str!("../../../examples/block-reference.dx");

/// Number of timed iterations per operation (after a warm-up pass).
const ITERS: u32 = 2_000;

/// Time `op` over `ITERS` iterations and return the mean nanoseconds per call.
///
/// `op` returns a value so the optimiser cannot elide the work; the returned value is fed
/// into a black-box accumulator by the caller.
fn time_ns<T>(label: &str, byte_size: usize, mut op: impl FnMut() -> T) -> u64 {
    // Warm-up: prime caches / branch predictors, ignore the timing.
    for _ in 0..64 {
        std::hint::black_box(op());
    }
    let start = Instant::now();
    for _ in 0..ITERS {
        std::hint::black_box(op());
    }
    let elapsed = start.elapsed();
    let per_call = elapsed.as_nanos() / u128::from(ITERS);
    let mbps = if byte_size > 0 && per_call > 0 {
        (byte_size as f64) / (per_call as f64) * 1_000.0 // bytes/ns * 1e3 = MB/s (decimal)
    } else {
        0.0
    };
    if byte_size > 0 {
        println!("{label:<28} {per_call:>10} ns   {mbps:>8.1} MB/s   ({byte_size} B)");
    } else {
        println!("{label:<28} {per_call:>10} ns", per_call = per_call);
    }
    per_call as u64
}

fn main() {
    // ---- Representative inputs -------------------------------------------------
    // A 64 KiB pseudo-text buffer (compressible) for the codec and digest rows.
    let buf64k: Vec<u8> = (0..65_536u32)
        .map(|i| b"the quick brown fox jumps over the lazy dog "[(i as usize) % 44])
        .collect();

    // The largest example document, parsed once for the codec/stringify rows.
    let doc: Document = format::parse(WELCOME_DX);
    let packed = docbin::pack(&doc);

    // A small multi-document corpus for the search rows.
    let corpus: Vec<(String, Document)> = vec![
        ("welcome.dx".to_string(), format::parse(WELCOME_DX)),
        ("tutorial.dx".to_string(), format::parse(TUTORIAL_DX)),
        (
            "block-reference.dx".to_string(),
            format::parse(BLOCK_REF_DX),
        ),
    ];
    let index = search::build_index(&corpus);

    // A bundle of the three packed docs for the archive rows.
    let entries: Vec<BundleEntry> = corpus
        .iter()
        .map(|(path, document)| BundleEntry {
            path: path.clone(),
            git: GitFlags::default(),
            packed: docbin::pack(document),
        })
        .collect();
    let bundle_bytes = bundle::encode_bundle(&entries);

    println!("doc-core micro-benchmark (mean of {ITERS} iters, release)\n");
    println!("{:<28} {:>13}   {:>13}", "operation", "ns/op", "throughput");
    println!("{}", "-".repeat(70));

    // ---- digest ---------------------------------------------------------------
    time_ns("digest::sha256_hex 64KB", buf64k.len(), || {
        digest::sha256_hex(&buf64k)
    });
    time_ns("digest::sha1_hex 64KB", buf64k.len(), || {
        digest::sha1_hex(&buf64k)
    });

    // ---- compress / decompress ------------------------------------------------
    let frame = compress::compress(&buf64k);
    time_ns("compress::compress 64KB", buf64k.len(), || {
        compress::compress(&buf64k)
    });
    time_ns("compress::decompress 64KB", buf64k.len(), || {
        compress::decompress(&frame).expect("valid frame")
    });

    // ---- docbin pack / unpack -------------------------------------------------
    time_ns("docbin::pack welcome.dx", packed.len(), || {
        docbin::pack(&doc)
    });
    time_ns("docbin::unpack welcome.dx", packed.len(), || {
        docbin::unpack(&packed).expect("valid blob")
    });

    // ---- format parse / stringify --------------------------------------------
    time_ns("format::parse welcome.dx", WELCOME_DX.len(), || {
        format::parse(WELCOME_DX)
    });
    time_ns("format::stringify welcome.dx", WELCOME_DX.len(), || {
        format::stringify(&doc)
    });

    // ---- bundle encode / decode ----------------------------------------------
    time_ns("bundle::encode 3 docs", bundle_bytes.len(), || {
        bundle::encode_bundle(&entries)
    });
    time_ns("bundle::decode 3 docs", bundle_bytes.len(), || {
        bundle::decode_bundle(&bundle_bytes).expect("valid bundle")
    });

    // ---- search build / query -------------------------------------------------
    time_ns("search::build_index 3 docs", 0, || {
        search::build_index(&corpus)
    });
    time_ns("search::query 'block document'", 0, || {
        index.search("block document")
    });
}
