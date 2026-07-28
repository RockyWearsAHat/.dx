# dx — Rust workspace

The engine for `dx`. See the repository [`README.md`](../README.md) for what the platform
does and [`CLAUDE.md`](../CLAUDE.md) for how to work on it; this file covers the workspace
itself.

## The shape of it

Ports and adapters: a pure, I/O-free core wrapped by host shells that own the outside world.

```text
                       ┌──────────────────┐
                       │     doc-core     │   no I/O, no OS deps, no unsafe,
                       │  format · render │   wasm32-safe, deterministic
                       │  digest · search │
                       └───┬──────────┬───┘
              compiled for │          │ compiled for
              the host OS  │          │ wasm32
        ┌──────────────────┴──┐    ┌──┴────────────────┐
        │  doc-run · doc-shot │    │     doc-wasm      │
        │       doc-cli       │    │  (VS Code editor) │
        └─────────────────────┘    └───────────────────┘
```

`doc-core` knows nothing about processes, files, or browsers, which is what lets it compile
to `wasm32` untouched — and what makes the editor and the CLI render identically.

| Crate | Responsibility |
|-------|----------------|
| `doc-core` | The format and its views: `model`, `format` (DOCSRC parse + canonical stringify), `render` (HTML, Markdown, outline, sections), `digest` (SHA-256/SHA-1), `compress` (the `dxz1` LZSS codec), `docbin` (DOCB1), `bundle` (DXBUN5), `search` (dxlite) |
| `doc-run` | Executing `::code … run` blocks: per-language plans, dependency install, sandboxes, timeouts, output capture |
| `doc-shot` | Rendering a document to PNG through an installed Chromium browser (measure pass, then capture pass) |
| `doc-cli` | The `dx` binary — CLI commands, the MCP stdio server, the installer |
| `doc-wasm` | The JavaScript boundary: `doc-core` behind `wasm-bindgen` |

## Byte-exact against the reference

Every canonical output is byte-identical to the original TypeScript implementation (`src/`
in the parent repo): digests, `dxz1` frames, DOCB1 documents, DXBUN5 bundles, DOCSRC text,
and search rankings. The fixtures (`examples/*.dx` → `doc-core/tests/fixtures/*.expected.dx`)
prove it and must never regress — a change that breaks them reformats every document that
already exists.

Format changes must be **additive**: a new attribute has to serialize to nothing when unset.

## Working on it

```bash
cargo test                                  # 238 tests
cargo clippy --all-targets -- -D warnings
cargo fmt --check

cargo build --release -p doc-cli            # → target/release/dx
```

Rules that hold everywhere: no `unsafe`, every public item documented, no panics in library
code. Fallible operations return `Result`, and a failure a user will see carries a sentence
saying what to do about it.

For the wasm build, put the rustup toolchain ahead of any Homebrew `rustc`:

```bash
PATH="$HOME/.cargo/bin:$PATH" wasm-pack build --release --target nodejs \
  --out-dir ../../editor/vscode/wasm
```

[`ANALYSIS.md`](ANALYSIS.md) records the complexity and measured timings of each `doc-core`
operation.
