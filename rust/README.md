# DOC platform — Rust workspace

This workspace is the canonical engine for the DOC platform: a single, runtime-agnostic
Rust core compiled to **two hosts** — a native binary (the MCP server / CLI) and a
WebAssembly module (the in-editor block editor). The same Rust code runs in both places,
so the historical dual-parser drift between the VS Code webview and the backend is gone.

Every output the core produces is **byte-identical to the legacy TypeScript reference**
(`src/` in the parent repo): digests, `dxz1`-compressed frames, DOCB1-packed documents,
DXBUN5 bundles, canonical DOCSRC text, and search rankings all round-trip across the Rust
native build, the Rust wasm build, and the old TS code. Interop fixtures (the
`examples/*.dx` → `tests/fixtures/*.expected.dx` pairs) prove this and must never regress.

## Layer map

The workspace is **ports and adapters**: a pure, I/O-free core, wrapped by host shells
that own the outside world.

```
                 ┌─────────────────────────────────────┐
   pure core     │            doc-core                  │   no I/O, no OS deps,
   (this is the  │  model · digest · compress (dxz1) ·  │   #![forbid(unsafe_code)],
   source of     │  docbin (DOCB1) · format (DOCSRC) ·  │   wasm32-safe, deterministic
   truth)        │  bundle (DXBUN5) · search (dxlite)   │
                 └───────────────┬─────────────────────-┘
                                 │ depends on
              ┌──────────────────┴──────────────────┐
   host       │                                     │
   shells   ┌─┴────────────────┐        ┌───────────┴───────────┐
            │   doc-native     │        │      doc-wasm         │
            │  MCP stdio server│        │  wasm-bindgen JS API  │
            │  std::fs + git   │        │  (browser / webview)  │
            │  FsDocStore      │        │  parse/stringify/...  │
            └──────────────────┘        └───────────────────────┘
```

- **`doc-core`** — zero-dependency engine. Modules: `model` (in-memory `Document`/`Block`),
  `digest` (SHA-256 / SHA-1 hex), `compress` (the `dxz1` LZSS byte codec), `docbin` (the
  DOCB1 binary document codec), `format` (DOCSRC `.dx` parse + canonical stringify),
  `bundle` (the DXBUN5 multi-document archive), and `search` (the dxlite-equivalent token
  index). No `unsafe`, no panics in library code, every public item documented.
- **`doc-native`** — the native MCP server shell. A newline-delimited JSON-RPC 2.0 stdio
  loop (`protocol` → `dispatch` → `server`) that exposes document operations to AI agents,
  delegating storage to a `DocStore`. `FsDocStore` wires `doc-core` over `std::fs` + git;
  `MemoryStore` is an in-memory stub. No Node, npm, or TypeScript anywhere in the path.
- **`doc-wasm`** — `doc-core` compiled to `wasm32` via `wasm-bindgen`. Documents cross the
  JS boundary as `camelCase` JSON; binary payloads cross as `Uint8Array`; fallible calls
  return `Result<_, JsValue>`. The webview parses through this instead of its old JS parser.

## Build & run

Toolchain: rustup (`~/.cargo/bin`). The wasm build also needs the
`wasm32-unknown-unknown` target and `wasm-pack`.

```bash
# Native MCP server (release binary, no Node)
cargo build --release -p doc-native
./target/release/doc-native              # speaks MCP/JSON-RPC over stdin/stdout

# WebAssembly module for the editor
wasm-pack build doc-wasm --target web    # emits the JS glue + .wasm into doc-wasm/pkg

# Re-measure the micro-benchmark table in ANALYSIS.md
cargo run --release -p doc-core --example bench
```

## Test strategy

`cargo test --workspace` runs the suite (67 `#[test]` functions across the three crates;
`doc-core` alone holds 50 — 48 unit + 2 integration). Coverage is layered:

- **Byte-exact TS interop vectors** — `doc-core`'s `format` tests parse/stringify the real
  `examples/*.dx` documents and assert byte-for-byte equality against
  `tests/fixtures/*.expected.dx`; `digest` tests assert the published SHA test vectors.
  These pin the Rust output to the legacy TypeScript reference.
- **Round-trip / pipeline** — `tests/pipeline.rs` exercises the full storage path
  (parse → DOCB1 pack → dxz1 compress → decompress → unpack) and asserts the document
  survives unchanged, plus that a corrupted frame is *rejected*, not panicked.
- **On-disk binary smoke** — `doc-native`'s `tests/binary_smoke.rs` launches the *actual
  compiled `doc-native` executable*, feeds it newline-delimited JSON-RPC on stdin, and
  asserts the stdout responses, proving the shipped binary speaks MCP with no Node present.
- Edge-size and malformed-input cases throughout (empty input, truncated frames, clamped
  heading levels, unknown block kinds) act as the **fuzz/robustness** layer: the contract
  is that the core never panics on bad bytes — failures surface as `Result` errors.

Quality gates that must stay green: `cargo test`, `cargo clippy --all-targets -- -D warnings`,
and `cargo fmt --check`.

## Design

The patterns below are the ones actually used in the code, not aspirational:

- **`DocStore` trait as a Strategy / port.** `doc-native`'s `dispatch` layer talks only to
  the `DocStore` trait, never to a concrete store. Two adapters implement it: `MemoryStore`
  (an in-memory stub used by the protocol tests) and `FsDocStore` (the real filesystem +
  git + `doc-core` backend). Swapping storage strategies requires zero changes to the
  protocol/dispatch code.
- **Ports and adapters (hexagonal) separation.** `doc-core` is the pure domain: no
  filesystem, no clock, no network, no `unsafe`, `wasm32`-safe. All outside-world concerns
  (file I/O, git state, stdio framing, the JS boundary) live in the host shells
  `doc-native` and `doc-wasm`. This is exactly what lets one core serve both a native
  binary and a browser module.
- **`Result`-based error enums.** Fallible operations return typed error enums rather than
  panicking or stringly-typed errors: `compress::DecompressError`, `docbin::UnpackError`,
  `bundle::BundleError` in the core, and `store::StoreError`
  (`NotFound` / `InvalidArgument` / `Backend`) in the host, which the dispatcher maps to
  the correct JSON-RPC error codes. Library code never `unwrap`s on untrusted input.
- **Deterministic, dependency-light core.** `doc-core` has no third-party crates; metadata
  values are kept as raw JSON text so the core needs no JSON dependency, and search results
  break ties by ascending path so rankings are fully reproducible.
