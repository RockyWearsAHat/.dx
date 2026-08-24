# `.doc/` — where your documents actually live

A `.dx` file in this project is a one-line pointer:

```text
~ dx1 c939d5becfb64b14193566ffed7ccf8217c90bf5c90e6ba2a5ce8bf87903c823
```

The content is here, stored once per distinct block.

## Contents

| File | What it is | Commit it? |
|------|------------|-----------|
| `repo.dxcp` | **Your documents.** Every repository document, split into content-addressed chunks and deduplicated across documents and versions. Written *uncompressed*: git deltas plain bytes between revisions and cannot delta a compressed stream, so leaving the bytes alone makes the file bigger here and the repository smaller. | **Yes.** This is the content. |
| `local.dxcp` | Documents git ignores or has never tracked — scratch work that should not reach a teammate. | No |
| `index.db` | SQLite: the local queryable authority. Chunks, manifests, sections, and a token index for search. Rebuildable from `repo.dxcp`. | No |

Every dx write writes the `.gitignore` lines that get this right, so a fresh clone or a new
worktree needs no ceremony. `dx git-setup` repeats it by hand if a checkout has lost them.

## How a document is stored

A document is one **chunk** per block, where a chunk's payload is byte-for-byte the canonical
text `dx` would write for that block. Reassembly is concatenation, so nothing can be lost in
translation — the store cannot drop a block attribute it had not heard of. Two blocks that read
the same are one chunk, so editing one paragraph of a long document costs one new chunk.

A **manifest** records one version: its content digest and its ordered chunk list. Every version
ever saved keeps its manifest, which is why `git log -p` can render an old revision — and why it
costs almost nothing to keep, since an edited document shares every unchanged block with its
predecessor.

## Reading a document

Never open the pointer expecting content. Ask `dx`, and it resolves:

```bash
dx text notes.dx        # the document as Markdown
dx textconv notes.dx    # its exact canonical source
dx render notes.dx      # a self-contained HTML page
dx stats .              # what sharing and compression saved
```

Agents use the MCP server (`dx mcp`), which resolves the same way.

## If something looks wrong

```bash
dx sync .
```

That is the repair command. It adopts any plain-text `.dx` file something else wrote, rebuilds
`index.db` from the packs when it is missing — the fresh-clone case — and rewrites pointers that
drifted. It never discards content: a pointer it cannot resolve is reported, not blanked.

If a pointer is reported unresolved, its content is in neither the index nor the packs. Restore
`repo.dxcp` from version control.

## Do not hand-edit anything in here

These are generated artifacts. Write documents through `dx` or your editor; both go through the
same store.
