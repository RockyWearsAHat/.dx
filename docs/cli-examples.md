# dx CLI Guide with Examples

This guide provides practical examples for each dx command category. For the concise reference, run `dx help`.

## Quick Start

```bash
# Create a document
dx new myproject.dx --title "My Project"

# Read it as Markdown (useful for scripting)
dx text myproject.dx

# View it in a browser  
dx render myproject.dx | head -20   # Show first bit
dx open myproject.dx                # Full browser window

# Add content and make it runnable
dx insert myproject.dx --after intro --type code --lang python --run --deps "requests"
dx set myproject.dx myblock --text "import requests; print(requests.get('http://example.com').status_code)"

# Review before running
dx run myproject.dx --review

# Approve and execute
dx run myproject.dx --approve
```

## READ Commands

### dx text — get document as plain Markdown

Read any document as clean Markdown text (no formatting). Useful for processing with other tools:

```bash
# Print the whole document
dx text notes.dx

# Get just one section  
dx text notes.dx --section architecture

# Get block IDs so you can reference them in other commands
dx text notes.dx --ids
```

### dx render — view as HTML page

Render to an interactive HTML page:

```bash
# Print HTML to stdout (pipe to file or browser)
dx render notes.dx > page.html

# Render just one block (single block, exact styling)
dx render notes.dx --block myblock

# Render the whole site
dx render --all . --out ../html-site/

# Choose light/dark theme
dx render notes.dx --theme dark
```

### dx png — capture as image

Take a screenshot of rendered content:

```bash
# Whole document as PNG
dx png notes.dx --out page.png

# One page per image (for multi-page documents)
dx png notes.dx --pages --out outputs/

# One block alone at natural size (boards render at their exact size)
dx png notes.dx --block diagram1

# Multiple blocks, one file each
dx png notes.dx --block intro,diagram1,diagram2

# Compare a block render to a golden image (for regression testing)
dx png notes.dx --block diagram1 --against golden.png
# Output: "identical" or "differs: N px in x,y wxh"
```

### dx outline — list blocks and their preview

Get a machine-readable block list:

```bash
# Print block ids, types, and first line of each
dx outline notes.dx

# Use in scripts (one block per line, tab-separated)
dx outline notes.dx | grep "h2" | cut -f1
```

### dx ls — list all documents

Find all .dx files in a project:

```bash
# List docs in current dir
dx ls

# List in a specific directory
dx ls src/
```

### dx search — find in documents and source code

Search both documents and code simultaneously:

```bash
# Ask a question in plain language
dx search "how do I build this on Windows"

# Limit results
dx search "configuration options" --limit 10

# Search in specific directory
dx search "cache strategy" docs/
```

Results show either:
- A block from a .dx document (with block ID and section)
- A line range from source code (file path and lines)

### dx trace — extract symbols and references

Build a symbol index from source code:

```bash
# List all symbols (functions, types, constants) and where they're defined/used
dx trace

# Compact ranking (show most-used symbols first, suitable for embedding)
dx trace --brief
dx trace src/ --brief
```

Works with: Rust, JavaScript/TypeScript, Python, Go.

### dx coverage — search quality report

See what searches are missing documentation:

```bash
# Report on last 200 searches
dx coverage

# Customize window and set minimum threshold
dx coverage --window 100 --min-rate 80
# Fails if under 80% of searches landed in a document
```

Shows:
- % of searches that hit a .dx document
- Most-repeated queries that fell back to source-only
- Hints for what to document next

## WRITE Commands

### dx new — create a document

```bash
dx new notes.dx
dx new notes.dx --title "Project Notes"
```

### dx index — auto-scan and scaffold docs

Analyzes the project and creates index.dx plus dev.dx:

```bash
# Generate from current directory
dx index

# Re-scan the tree (refresh)
dx index --force
```

Creates:
- `index.dx` — project map (files ranked by importance)
- `dev.dx` — verification gates (test, lint, build commands)

### dx insert — add a block

Insert a new block after another:

```bash
# Add after "intro" block
dx insert notes.dx --after intro --type text

# Code block (specify language, mark runnable, declare deps)
dx insert notes.dx --after intro \
  --type code --id analyze --lang python --run \
  --deps "numpy,matplotlib"

# Give it an id so you can reference it later
dx insert notes.dx --after intro --id section1 --type h2
dx set notes.dx section1 --text "My New Section"
```

Block types: `text`, `h1`-`h4` (headings), `code`, `checklist`, `ul` (list), `ol` (ordered list), `quote`, `board`, etc.

### dx append — add a block at the end

Same options as `dx insert`, but always goes to the end:

```bash
dx append notes.dx --type text --text "Added at end"
dx append notes.dx --type code --lang bash --run
```

### dx set — edit a block

Replace block content:

```bash
# Replace entire body
dx set notes.dx myblock --text "new content here"

# Replace the header line (::kind attrs)
dx set notes.dx myblock --header "::code lang=rust run"

# Quick find-replace within a block
dx set notes.dx myblock --replace "oldname" --with "newname"
dx set notes.dx myblock --replace "foo" --with "bar" --all
```

### dx remove — delete a block

```bash
# Delete one block (history stays in store)
dx remove notes.dx myblock

# Delete entire document (history stays in store)
dx remove notes.dx
```

### dx check — toggle checkbox in checklist

```bash
# Tick the 3rd box (counting from 0)
dx check notes.dx tasks --item 2

# Toggle again to untick
dx check notes.dx tasks --item 2
```

### dx board — arrange diagram nodes

Edit the layout of a ::board:

```bash
# Move a node
dx board notes.dx diagram --place node1 --x 100 --y 50 --w 200

# Add a new node
dx board notes.dx diagram --add --x 300 --y 100

# Remove a node
dx board notes.dx diagram --detach node1

# Draw or erase connections
dx board notes.dx diagram --link nodeA --to nodeB
dx board notes.dx diagram --unlink nodeA --to nodeB
```

### dx source — extract raw block text

Get the exact text of a block (for offline editing):

```bash
# Print block contents
dx source notes.dx myblock

# Include the ::kind opening line
dx source notes.dx myblock --header
```

Copy, edit offline, then paste back with `dx set --text`.

### dx fmt — format blocks canonically

Ensure consistent formatting:

```bash
# Reformat files in-place
dx fmt notes.dx

# Check without modifying
dx fmt notes.dx --check
```

## RUN Commands

### dx run — execute code blocks

Execute blocks marked `run`:

```bash
# Run all blocks
dx run myproject.dx

# Run one block only
dx run myproject.dx --only analyze

# Simulate (show what would run)
dx run myproject.dx --dry

# Change execution order (follow ::board edges instead of doc order)
dx run myproject.dx --follow-edges
```

### Approval Workflow

**New or edited code must be reviewed before execution:**

```bash
# Step 1: Review code and fingerprints (no execution)
dx run myproject.dx --review

# Step 2: Approve reviewed code and run it
dx run myproject.dx --approve

# Step 3: Future runs of approved code (auto-runs)
dx run myproject.dx

# Override approval (use for trusted code)
dx run myproject.dx --force
# Output marks this as FORCED_NOTICE
```

**Key point:** Editing a block clears its approval. The edit is the review.

### Sandbox & Permissions

Code runs confined to the project (no access to your full system):

```bash
::code id=build lang=rust run \
  deps="cargo" \
  reads="src,Cargo.lock" \
  writes="target"
  # This block:
  # - reads: src/ and Cargo.lock (declared inputs)
  # - writes: target/ (build output)
  # - Can use system toolchains
  # - Cannot access network
  # - Cannot read outside the document's folder
  # - Cannot write outside declared folders
::end
```

**Block attributes:**
- `deps="pkg1,pkg2"` — install these during setup (packages)
- `reads="src,data.csv"` — this block reads these files (comma-separated, confined to doc folder)
- `writes="target,out"` — this block writes to these folders (comma-separated, must be in doc folder)

Changes to deps or writes re-open review (as with code edits).

## STORAGE Commands

### dx sync — repair and restore

Reconciles .dx files and their stored content:

```bash
# After git operations (merge, checkout, etc.)
dx sync

# Recursive sync of directory tree
dx sync src/
```

Use when:
- Documents don't parse (malformed pointers)
- After merging branches that both changed documents
- After restoring from backup
- If .doc/index.db got corrupted

### dx stats — storage information

```bash
# Show documents, blocks, sharing, size
dx stats

# Analyze a subdirectory
dx stats src/
```

### dx rm — delete a document

```bash
# Delete (history survives in store)
dx rm notes.dx
```

### dx git-setup — configure git

```bash
# Initial setup (usually automatic)
dx git-setup

# Or to repair an existing repo
dx git-setup .
```

Configures git to:
- Diff .dx files (show documents, not digests)
- Merge .dx files block-by-block
- Track .doc/repo.dxcp (the store)
- Ignore machine-local files (.doc/index.db, .doc/local.dxcp)

## REPORT Commands

Report bugs and suggestions to the dx team:

```bash
# File a bug
dx report bug \
  --title "search misses function names" \
  --detail "Asked 'where is parse_config' and it found nothing"

# File a suggestion
dx report suggestion \
  --title "add --format json to dx outline"

# Name the affected command
dx report bug \
  --title "render crashes on big documents" \
  --route "render" \
  --repro "dx render 1gb-file.dx"
```

Manage reports:

```bash
# See waiting reports
dx report list

# Sync with the intake
dx report sync

# Mark a report fixed (once the bug is fixed upstream)
dx report close <id>
```

First-time setup:

```bash
dx report setup
# Mints a collision-resistant project key
# Reuses stored token (or prompts for one)
```

## PLATFORM Commands

### dx serve — local rendering service

```bash
# Start on default port
dx serve

# Specify port
dx serve --port 8000
```

Service:
- Holds document packs in memory
- Renders through the single engine (consistent across all surfaces)
- Reads no files, writes nothing, runs nothing

### dx mcp — serve documents to AI agents

```bash
dx mcp
```

Starts MCP server on stdio. Used by Claude and other AI agents to read and edit documents.

### dx setup — install dx

One-time machine setup:

```bash
# Full installation
dx setup

# Skip binary installation (use existing one)
dx setup --no-path

# Specify where to install binary
dx setup --bin-dir ~/.local/bin

# Uninstall everything
dx setup --uninstall
```

Installs:
- Binary on PATH
- MCP server (registered with Anthropic Claude, OpenAI, etc.)
- Rendering service (starts at login)
- DX.app (double-click .dx files)
- Browser extension (GitHub, VS Code, etc.)

### dx browser — install GitHub extension

```bash
dx browser                    # Show setup steps
dx browser --open            # Open in browser for installation
dx browser --browser firefox  # Setup for Firefox only
```

View .dx files on github.com inline.

### dx doctor — check installation

```bash
dx doctor
```

Reports:
- Binary location and PATH status
- Image rendering capability
- Code sandbox (seatbelt, bubblewrap, or none)
- Runtime toolchains (Rust, Python, Node, Go, etc.)
- Agent/MCP registrations
- Service status
- Document and git configuration
- Report inbox status

## Common Patterns

### Iterative Development

```bash
# Create and scaffold
dx new project.dx
dx index

# Keep notes while developing
dx append project.dx --type text --text "Found bug in parser"
dx append project.dx --type text --text "Fixed by adding check for nulls"

# Document decisions
dx insert project.dx --after overview --id decision --type h3
dx set project.dx decision --text "Use strategy pattern for renderers"

# Track progress with checklists
dx insert project.dx --after tasks --type checklist --id checklist
dx check project.dx checklist --item 0  # tick first box
```

### Search & Documentation Workflow

```bash
# Check what queries are missing documentation
dx coverage --window 200 --min-rate 80

# See which queries failed
dx coverage

# Add blocks to answer the missed queries
dx append project.dx --type text --id cache_strategy
dx set project.dx cache_strategy --text "Our cache uses LRU eviction..."

# Re-run coverage to verify it helps
dx coverage
```

### Runnable Examples

```bash
# Code that documents itself
dx insert examples.dx --after intro --type code \
  --lang python --run --deps "numpy" \
  --id matrix_ops

dx set examples.dx matrix_ops --text "
import numpy as np
data = np.array([[1,2],[3,4]])
print('Matrix shape:', data.shape)
print('Sum:', data.sum())
"

# Run and store output
dx run examples.dx --approve

# Now anyone reading the document sees the output
dx text examples.dx
```

### Batch Operations

```bash
# Find all code blocks
dx outline myproject.dx | grep "^code" | cut -f1 | while read id; do
  echo "Running $id..."
  dx run myproject.dx --only $id
done

# Format all docs in src/
for file in $(dx ls src/); do
  dx fmt "$file"
done

# Search across all docs
dx search "database configuration" docs/
```

## Integration with Other Tools

### With git

```bash
# After pulling, repair documents
dx sync
git status  # should show no unexpected changes

# View what changed in documents
git diff notes.dx  # shows block-by-block diff, not hash changes
```

### With CI/CD

```bash
# Verify all code blocks are runnable
dx run *.dx --dry

# Run and fail if any block fails
dx run myproject.dx
echo "All code blocks passed"
```

### With Editors

The `dx mcp` server is automatically available to:
- Anthropic Claude (web and desktop)
- OpenAI ChatGPT (with MCP support)
- Any tool that speaks MCP over stdio

Code/agents read and edit documents with:
- `dx_read` for rendering
- `dx_source` for exact text
- `dx_set` for editing

## Tips & Tricks

### Finding Block IDs

```bash
# List all block IDs in a document
dx text file.dx --ids

# Or with outline
dx outline file.dx | cut -f1
```

### Batch Editing

```bash
# Replace across many blocks efficiently
dx set file.dx block1 --replace "old_name" --with "new_name"
dx set file.dx block2 --replace "old_name" --with "new_name"
```

### Performance

```bash
# Fast preview (no images)
dx text file.dx

# Fast update (no re-render)
dx set file.dx myblock --text "new text"

# Track what's slow
dx coverage --window 50  # focused window for recent queries
```

### Troubleshooting

```bash
# Check what's wrong
dx doctor

# Repair documents
dx sync

# See raw document content
dx textconv file.dx
```

