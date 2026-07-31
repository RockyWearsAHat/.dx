# dx — Rust workspace

The engine for `dx`. [`README.md`](../README.md) is what the platform does;
[`CLAUDE.md`](../CLAUDE.md) is the authority on the crates, the non-negotiables, and the
quality gates. This file covers only the shape of the workspace, which is not described
anywhere else.

## Ports and adapters

A pure, I/O-free core wrapped by host shells that own the outside world.

```text
                       ┌──────────────────┐
                       │     doc-core     │   no I/O, no OS deps, no unsafe,
                       │  format · render │   wasm32-safe, deterministic
                       │  chunk  · search │
                       └───┬──────────┬───┘
              compiled for │          │ compiled for
              the host OS  │          │ wasm32
   ┌───────────────────────┴─┐     ┌──┴──────────────────────┐
   │  doc-store · doc-run    │     │        doc-wasm         │
   │  doc-shot  · doc-cli    │     │ (VS Code · github.com)  │
   └─────────────────────────┘     └─────────────────────────┘
```

`doc-core` knows nothing about processes, files, or browsers, which is what lets it compile
to `wasm32` untouched — and what makes the editor, the browser extension, and the CLI render
identically. `doc-store` is where content and the filesystem meet; nothing above it reads a
pack directly.

[`ANALYSIS.md`](ANALYSIS.md) records the complexity and measured timings of each `doc-core`
operation.
