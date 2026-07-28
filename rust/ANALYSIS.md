# DOC engine — complexity & timing analysis

This document records the asymptotic complexity of each `doc-core` operation and a table
of **measured** timings on representative inputs. All operations are pure, single-threaded,
and deterministic; complexity is in terms of input size with no hidden allocation surprises
(every codec pre-sizes its output buffer).

Notation: `n` = input byte length; `b` = number of blocks in a document; `Sigma tokens` =
total tokens across all indexed documents; `qtokens` = distinct tokens in a query; `d` =
number of documents in a search corpus.

## Asymptotic complexity

| Operation | Complexity | Why |
|-----------|------------|-----|
| `digest::sha256_hex` / `sha1_hex` | **O(n)** | One pass over the padded message, fixed 64/80-round block compression per 64-byte block; constant work per byte. |
| `compress::compress` (dxz1) | **O(n * MAX_CHAIN)** -> O(n) | LZSS with a hash-chained match search bounded by `MAX_CHAIN = 128` candidates per position, so the per-byte match scan is a constant factor; worst case is linear in `n`. |
| `compress::decompress` (dxz1) | **O(n)** | Each token is either a literal byte or a back-reference copy of bounded length (<= 258); total bytes emitted equals the original length, copied once. |
| `docbin::pack` | **O(n)** | A single ordered walk of the `b` blocks and their items, emitting varint-prefixed fields; total work is proportional to the document's byte size. |
| `docbin::unpack` | **O(n)** | One linear scan of the packed bytes, decoding each varint/field exactly once into the model. |
| `format::parse` (DOCSRC) | **O(n)** | A line-oriented scan of the source; each line is classified and appended to the current block once. Per-block normalization (id slugging, level clamping) is O(1) amortized. |
| `format::stringify` | **O(n)** | Emits one opening line per block plus body lines verbatim; output size is proportional to the document, written in a single pass. |
| `bundle::encode_bundle` (DXBUN5) | **O(Sigma bytes)** | Concatenate every entry's header + packed payload into one buffer, then one `dxz1` compress pass over that buffer -- both linear in the total packed bytes. |
| `bundle::decode_bundle` | **O(Sigma bytes)** | One `dxz1` decompress, then a linear walk of the entry table slicing out each payload. |
| `search::build_index` | **O(Sigma tokens)** time and space | Each searchable block's text is tokenized once and tallied into a per-document hash map; total cost is the sum of all tokens across all documents. |
| `search::SearchIndex::search` | **O(d * qtokens)** | A linear scan over the `d` documents, each doing `qtokens` O(1) hash-map lookups, followed by an O(h log h) sort of the `h <= d` non-zero hits. |

Notes:
- The `dxz1` match search is bounded twice over: by `MAX_CHAIN = 128` chain hops and by
  `MAX_DISTANCE = 65535` window distance, so compression is linear in practice rather than
  quadratic even on adversarial input.
- `search` uses a per-document count map and a linear document scan (not an inverted
  index); for the platform's corpus sizes this is simpler and the query cost is dominated
  by the small `qtokens * d` term, as the measured ~280 ns query below confirms.

## Measured timings

**How measured.** A dependency-free harness, `doc-core/examples/bench.rs`, times each
operation with `std::time::Instant` -- no Criterion or other bench crate. Each row is the
**mean of 2 000 iterations** after a 64-iteration warm-up, with `std::hint::black_box`
guarding inputs and results so the optimizer cannot elide the work. Inputs: a 64 KiB
compressible text buffer (digest + codec rows) and the real `examples/welcome.dx`
(3 302 B source -> 2 997 B packed) plus a 3-document corpus (welcome / tutorial /
block-reference) for the document, bundle, and search rows.

Reproduce with:

```bash
cargo run --release -p doc-core --example bench
```

Build profile: workspace `release` (`opt-level = "z"`, LTO, `codegen-units = 1`).
Numbers below were recorded on the development host (Apple Silicon, macOS); absolute values
are machine-dependent, but the relative costs and orders of magnitude are stable across
runs (two consecutive runs agreed within a few percent).

| Operation | Input | ns/op | Throughput |
|-----------|-------|-------|-----------|
| `digest::sha256_hex` | 64 KiB | 222 757 | 294 MB/s |
| `digest::sha1_hex` | 64 KiB | 192 212 | 341 MB/s |
| `compress::compress` (dxz1) | 64 KiB | 198 252 | 331 MB/s |
| `compress::decompress` (dxz1) | 64 KiB | 128 084 | 512 MB/s |
| `docbin::pack` | welcome.dx (2 997 B) | 830 | 3 611 MB/s |
| `docbin::unpack` | welcome.dx (2 997 B) | 2 661 | 1 126 MB/s |
| `format::parse` | welcome.dx (3 302 B) | 38 179 | 87 MB/s |
| `format::stringify` | welcome.dx (3 302 B) | 10 073 | 328 MB/s |
| `bundle::encode_bundle` | 3 docs (3 896 B) | 78 981 | 49 MB/s |
| `bundle::decode_bundle` | 3 docs (3 896 B) | 18 525 | 210 MB/s |
| `search::build_index` | 3 docs | 16 326 | -- |
| `search::search` (`"block document"`) | 3-doc index | 281 | -- |

Reading the table:
- **Codecs and digests** run at a few ns/byte (~300-500 MB/s) -- the expected range for a
  pure scalar Rust implementation with no SIMD intrinsics; decompress beats compress
  because it has no match search.
- **`docbin` pack/unpack** are the cheapest per byte (GB/s range): a flat varint walk with
  no string analysis.
- **`format::parse`** is the slowest per byte (~87 MB/s) because it does the real work --
  line classification, block normalization, id slugging -- and is the operation the editor
  hits on every keystroke; at ~38 us for a full document that is still comfortably
  interactive. `stringify` is ~4x faster since it only emits.
- **`bundle::encode`** carries the `dxz1` compression cost of the whole payload, which is
  why it is the most expensive archive step; `decode` only decompresses then slices.
- **`search`**: building the index dominates (one tokenization pass per document), while a
  query against the built index is sub-microsecond, matching the O(d * qtokens) analysis.
