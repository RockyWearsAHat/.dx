# dx — a notepad for code

A `.dx` file is a document: block-structured plain text that renders to a page, and that can
run the code inside it and keep what that code produced.

GitHub renders this README because it is Markdown. It will not render a `.dx` file — its file
viewer picks from a fixed, first-party list that a repository cannot add to. So the document
below is a **picture** of one, taken with `dx png`. It is
[`examples/showcase.dx`](examples/showcase.dx), and the numbers and the chart in it were
computed by its own code blocks and stored back into the file.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/images/document-dark.png">
  <img alt="The rendered page of examples/showcase.dx: the title 'A Notepad for Code', a lead paragraph, a four-entry table of contents, a Python block that computes latency percentiles with its captured output beneath it, a second Python block whose output is drawn as a line chart, a numbered list of steps, and a closing quote." src="docs/images/document-light.png">
</picture>

## What a document is

A document is a sequence of typed blocks — prose, headings, lists, tables, drawings, boards,
and code. Each block opens with `::type`, carries attributes, and closes with `::end`:

```text
::code id=stats lang=python run deps=numpy
import numpy as np
latency_ms = np.array([12, 18, 9, 44, 15, 21, 13, 120, 17, 11])
print(f"p95  {np.percentile(latency_ms, 95):.1f} ms")
::end
```

`dx run` executes that block in a kernel sandbox — no network, no writes outside its own
scratch directory, and never before the code has been reviewed and approved on this machine —
and writes its output into the document, fingerprinted so re-running executes only what
changed. Reading, rendering, and screenshotting never execute anything.

On disk, a document you have adopted into a workspace is a one-line pointer; the content lives
in `.doc/repo.dxcp` as content-addressed, deduplicated chunks — committed to git,
diffed as prose after `dx git-setup`, and always resolved back to the true document by every
reader: the CLI, the editor, an agent, and git itself.

## Install

One command, once, per device:

```bash
cd rust && cargo build --release -p doc-cli
./target/release/dx setup
```

`dx setup` is the whole install. It puts the binary on your `PATH`, registers the MCP server
with every AI assistant it finds (Claude, Codex, Cursor, VS Code, Windsurf, Gemini), registers
the rendering service to start with your session, installs `DX.app` on a Mac so a
double-clicked `.dx` opens as a page, and configures each browser it can. It needs no
administrator and asks for no password; `dx setup --uninstall` reverses everything.

```bash
dx doctor    # what is installed, what is missing, and what is out of date
```

## First ten minutes

```bash
mkdir /tmp/pad && cd /tmp/pad && git init -q .
dx new notes.dx --title "First notes"
dx sync .            # make this directory a workspace
dx git-setup .       # make git diff, git show, and git log -p render documents
cat notes.dx         # a pointer — the content is in .doc/
dx text notes.dx     # the document
dx open notes.dx     # the rendered page, in your browser
```

Then read the guide, which is itself a dx document in this repository:

```bash
dx text examples/GETTING_STARTED.dx    # or: dx open examples/GETTING_STARTED.dx
```

`dx help` lists every command. The ones you will use daily: `dx text`, `dx outline`,
`dx render`, `dx png`, `dx run`, `dx set`, `dx ls`, `dx search`, `dx sync`.

## The documentation is dx documents

**This repository is a dx workspace, and its own documentation lives in it as documents** —
which is the point: they are searchable by `dx search`, editable one block at a time, and the
claims in them are run blocks whose recorded verdicts prove them. `.dx` files here are
one-line pointers whose content is in the committed pack at [`.doc/repo.dxcp`](.doc/README.md).
Read them with `dx text` / `dx open`, or on github.com with the browser extension installed,
which resolves pointers in place and renders pull requests as document diffs.

| Where to look | What it covers |
|---------------|----------------|
| [`index.dx`](index.dx) | The project map and the worklist, held true by its own verify block |
| [`docs/method.dx`](docs/method.dx) | How the project is worked: orientation, read routes, the harness |
| [`dev.dx`](dev.dx) | The development harness: `dx run dev.dx` proves the repository |
| [`validation.dx`](validation.dx) | What the method costs and saves, measured by run blocks |
| [`reports.dx`](reports.dx) | Field reports about dx — filed with `dx_report` from anywhere, carried here by the intake this checkout subscribes to |
| [`docs/intake.dx`](docs/intake.dx) | The report box's wire contract: the three routes, the computed id, what the box owes a reporter |
| [`docs/dx-format-contract.dx`](docs/dx-format-contract.dx) | The format: every block type, every attribute, canonical form |
| [`docs/github.dx`](docs/github.dx) | The browser extension and how github.com pages resolve |
| [`docs/grill-me.dx`](docs/grill-me.dx) | The review checklist for parser, renderer, and save-path changes |
| [`packaging/packaging.dx`](packaging/packaging.dx) | Building and publishing `DX.app` and the browser extensions |
| [`rust/engine.dx`](rust/engine.dx) | The workspace: what each crate is responsible for |
| [`rust/analysis.dx`](rust/analysis.dx) | Complexity and measured timings of the `doc-core` operations |
| [`examples/`](examples/) | Real documents: a showcase, a tutorial, board templates, a whole site designed and shipped on one board |

## For AI agents

The dx methodology is the standing default: an agent connected to dx works through it on every
task — index every codebase, keep the documents as the project's living, factual memory, and
treat planning, creation, and verification as one motion: edit the block, let it run, read the
result. Every request leaves the project's document truer than it found it, which is what makes
the tenth request in a project cost a fraction of the first.

`dx setup` registers the MCP server (`dx mcp`), whose handshake carries that rule to any agent
in any host. [`docs/method.dx`](docs/method.dx) is the whole of it: the read routes and what
each costs, why writing into the document beats holding it in context, and the verify-block
loop that makes "done" a thing the document proves rather than a thing anyone claims.

The agents using dx are also how it improves. When a tool misleads one, a message does not say
what to do next, or something has to be worked around, `dx_report` files it — bug, suggestion,
or observation — from whatever project the agent is in. It lands in this machine's inbox
outside every repository *and* is pushed to the intake, a public endpoint holding the report
database, so [`reports.dx`](reports.dx) in this checkout carries what agents on other machines
filed: `dx report subscribe` sets that up once, `dx report sync` folds the open reports in, and
an MCP session keeps them current on its own timer. Reports are keyed by kind, title and route,
so a defect reported by three sessions is one block with three sightings rather than three
duplicates, and `dx report close <id>` both removes the block and closes the record upstream.
`dx report drain` is the offline half, for a machine that could not reach the intake.

The intake this build files to is `https://rockywearsahat.com/report?dx` — dx's own, so a
report reaches the people who fix dx without anybody configuring anything. The service is the
query and nothing else: another service is `https://rockywearsahat.com/report?<serviceName>`,
created on the box by a registered account or its operator — filing to one that exists needs
nothing. It receives the report's text, the tool
version, the platform, and the *name* of the folder you were working in, never its path and
never its contents. `DX_REPORT_ENDPOINT=<url>` points it somewhere else — with a service on it
(`…/report?billing`) that names the database too — and `DX_REPORT_ENDPOINT=off` turns the push
off entirely, leaving the local inbox.

`dx report setup` wires any repository in one command — it mints a collision-resistant project
key of its own (never the folder name, never a typed `--project`, so nobody reaches this repo's
service by guessing), reuses the machine's stored token, subscribes the checkout, and syncs. On a
machine running the box (with local `selfhost` operator access), setup claims a per-service
reader token and registers a project-scoped MCP server at `.mcp.json` — with its key, token, and
endpoint all baked into that one file — so agents working in that repo get a report tool already
bound to that project, with no token to remember or leak, and no endpoint to reconfigure by hand.
`dx report token <t>` stores the owner's token once per machine (mint it with `selfhost reports
token` on the box), after which setup anywhere on the machine needs nothing.

## What executes, and what does not

Rendering never executes and never writes. Code runs in exactly three places — `dx run` (or
`dx_run`); the agent read tools' refresh, which re-runs only code this machine *already
approved* when its recorded output goes stale; and `dx_edit` running the runnable block it just
rewrote. `DX_NO_EXEC=1` turns execution off.

Code that runs is confined by the kernel — Seatbelt on macOS, bubblewrap on Linux — with no
network, and only after review on this machine. **A local edit is that review**: a block you
just typed is approved as saved, because the hand writing the code is the reviewer the gate
exists to consult; a document that merely *arrived* still waits for `--review`/`--approve`.
Approval names the code and its powers — runner, deps, code, the declared `reads=` paths, the
`writes=` grant — never its data, so new content re-runs by itself while new powers re-open
review.

The full rules, the grant language, and the payloads that test them are in
[`docs/dx-format-contract.dx`](docs/dx-format-contract.dx),
[`index.dx`](index.dx)'s engine-contracts section, and
[`rust/doc-run/tests/attacks.rs`](rust/doc-run/tests/attacks.rs) — a file of real attacks, each
asserted to fail.

## How it fits together

One Rust engine, compiled for every surface — which is what guarantees a document cannot look
like one thing in your editor and another to an agent. [`rust/engine.dx`](rust/engine.dx) is
the crate-by-crate map; [`index.dx`](index.dx) draws it as a board.

## Development

```bash
dx run dev.dx            # engine suites, lints, JS suites, corpus, page contract
dx run dev.dx --force    # re-prove everything regardless of staleness
```

Two suites must watch from outside the sandbox — the attack payloads that prove the kernel
boundary, and the Chromium captures — so the full proof stays a host-shell command:

```bash
cd rust
cargo test                                  # the whole suite, attacks and Chromium included
cargo clippy --all-targets -- -D warnings
cargo fmt --check
node --test "../editor/github/test/"*.test.mjs   # after ../editor/github/test/fixture.sh
node --test "../editor/vscode/test/"*.test.mjs
```

`.github/workflows/ci.yml` runs the same things on every push and pull request, on both
kernels that can confine a running block — Seatbelt on macOS, bubblewrap on Linux — because a
platform that cannot impose the boundary refuses to run a block at all, and testing one of
them would leave half the contract unproven. `rust/rust-toolchain.toml` pins the compiler so
CI, a fresh clone, and this machine all write one `target/`.

[`validation.dx`](validation.dx) is where the claims about what this saves are held to
measured numbers — including the two that came out against the method.

Every public item is documented, there is no `unsafe`, and library code does not panic.
[`dev.dx`](dev.dx) is the authority on the gates and what each one reads.
