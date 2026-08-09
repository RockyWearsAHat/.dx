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
| `compress::compress` (DXZ2, DEFLATE) | **O(n)** | A bounded hash-chained match search per position, so the per-byte scan is a constant factor; "store the bytes as they are" stays a candidate, so output never exceeds input plus the 8-byte header. |
| `compress::decompress` | **O(n)** | Each token is a literal or a bounded back-reference copy; total bytes emitted equals the original length, copied once. `DXZ1` (LZSS) is decoded forever by the same linear walk. |
| `format::parse` (DOCSRC) | **O(n)** | A line-oriented scan of the source; each line is classified and appended to the current block once. Per-block normalization (id slugging, level clamping) is O(1) amortized. |
| `format::stringify` | **O(n)** | Emits one opening line per block plus body lines verbatim; output size is proportional to the document, written in a single pass. |
| `chunk::split` | **O(n)** | One `stringify` per block plus a SHA-256 over each block's canonical text — linear in the document's byte size. |
| `chunk::encode_pack` (DXCP1) | **O(Sigma bytes)** | Deduplicate chunks by digest, then one `compress` pass per unique chunk; total work is linear in the corpus's canonical bytes. |
| `chunk::decode_pack` | **O(Sigma bytes)** | One linear walk of the entry table, decompressing each unique chunk once; reassembly is concatenation. |
| `search::build_index` | **O(Sigma tokens)** time and space | Each searchable block's text is tokenized once and tallied into a per-document hash map; total cost is the sum of all tokens across all documents. |
| `search::SearchIndex::search` | **O(d * qtokens)** | A linear scan over the `d` documents, each doing `qtokens` O(1) hash-map lookups, followed by an O(h log h) sort of the `h <= d` non-zero hits. |

Notes:
- `compress` encodes each way and keeps the smallest frame, naming the codec in the frame's
  first four bytes — so a heavier codec later is a new magic, never a conversion
  (`docs/dx-format-contract.md` § Compression).
- `search` uses a per-document count map and a linear document scan (not an inverted
  index); for the platform's corpus sizes this is simpler and the query cost is dominated
  by the small `qtokens * d` term, as the measured ~340 ns query below confirms.

## Measured timings

**How measured.** A dependency-free harness, `doc-core/examples/bench.rs`, times each
operation with `std::time::Instant` — no Criterion or other bench crate. Each row is the
**mean of 2 000 iterations** after a warm-up, with `std::hint::black_box` guarding inputs
and results so the optimizer cannot elide the work. Inputs: a 64 KiB compressible text
buffer (digest + codec rows) and the real `examples/welcome.dx` plus a 3-document corpus
for the document, pack, and search rows.

Reproduce with:

```bash
cargo run --release -p doc-core --example bench
```

Build profile: workspace `release`. Numbers below were recorded on the development host
(Apple Silicon, macOS, 2026-08); absolute values are machine-dependent, but the relative
costs and orders of magnitude are stable across runs.

| Operation | Input | ns/op | Throughput |
|-----------|-------|-------|-----------|
| `digest::sha256_hex` | 64 KiB | 205 894 | 318 MB/s |
| `digest::sha1_hex` | 64 KiB | 207 540 | 316 MB/s |
| `compress::compress` | 64 KiB | 71 358 | 918 MB/s |
| `compress::decompress` | 64 KiB | 31 752 | 2 064 MB/s |
| `chunk::split` | welcome.dx (3 619 B) | 25 002 | 145 MB/s |
| `format::parse` | welcome.dx (3 638 B) | 27 634 | 132 MB/s |
| `format::stringify` | welcome.dx (3 638 B) | 9 872 | 369 MB/s |
| `chunk::encode_pack` | 3 docs (4 000 B) | 96 539 | 41 MB/s |
| `chunk::decode_pack` | 3 docs (4 000 B) | 79 760 | 50 MB/s |
| `search::build_index` | 3 docs | 44 865 | — |
| `search::query` (`"block document"`) | 3-doc index | 336 | — |

Reading the table:
- **Codecs and digests** run in the hundreds of MB/s — the expected range for a pure
  scalar Rust implementation with no SIMD intrinsics; decompress beats compress because it
  has no match search.
- **`format::parse`** does the real work — line classification, block normalization, id
  slugging — and is the operation the editor hits on every keystroke; at ~28 µs for a full
  document that is comfortably interactive. `stringify` is ~3x faster since it only emits.
- **`chunk::encode_pack`** carries the compression cost of every unique chunk, which is
  why it is the most expensive step per byte; `decode_pack` decompresses then slices.
- **`search`**: building the index dominates (one tokenization pass per document), while a
  query against the built index is sub-microsecond, matching the O(d * qtokens) analysis.
