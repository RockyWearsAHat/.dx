import { saveDocumentSourceByRelativePath } from '../../build/runtime/src/doc-service.js';
import { rm, rmdir } from 'node:fs/promises';
import path from 'node:path';

const root = process.cwd();
const runtime = null;

// ---------------------------------------------------------------------------
// Document source texts in canonical .dx block format
// ---------------------------------------------------------------------------

const welcome = [
  '::heading level=1 id=welcome-heading',
  'Welcome to .dx',
  '::end',
  '',
  '::quote id=welcome-tagline',
  'Structured documents that agents can query, humans can edit, and both can trust.',
  '::end',
  '',
  '::heading level=2 id=what-is-dx',
  'What is .dx',
  '::end',
  '',
  '::paragraph id=dx-description',
  '.dx is a writing environment where every document is stored as typed blocks — headings, paragraphs, lists, quotes, and code — rather than raw markup. The source file on disk is a lightweight pointer. The content lives in a local SQLite database and a compact binary bundle that travels with your repository.',
  '::end',
  '',
  '::heading level=2 id=why-not-markdown',
  'Why not Markdown',
  '::end',
  '',
  '::paragraph id=markdown-problems',
  'Markdown is ambiguous by design. Multiple syntaxes produce the same output. Inline HTML forces parsers to handle two grammars at once. Edge cases grow with every extension. Writers end up fighting syntax instead of writing content. .dx uses one canonical block grammar with no ambiguity and no escape hatches.',
  '::end',
  '',
  '::heading level=2 id=how-it-works',
  'How it works',
  '::end',
  '',
  '::numbered-list id=how-it-works-steps',
  '- Every document is a sequence of typed blocks. There is no markdown, no inline HTML, and no implicit formatting.',
  '- Blocks are stored in SQLite and packed into a compact binary bundle at .doc/.repo-docs.bin that commits with your code.',
  '- The .dx file on disk is a stub pointer — a few lines that reference the archive. The real content never lives in the stub.',
  '- An MCP server exposes every document to any connected agent for search, retrieval, and structured editing.',
  '::end',
  '',
  '::heading level=2 id=explore-further',
  'Explore further',
  '::end',
  '',
  '::paragraph id=tutorial-link',
  'Ready to start writing? Open [the hands-on tutorial](examples/tutorial.dx) to create your first document and learn every block type.',
  '::end',
  '',
  '::paragraph id=reference-link',
  'Want to see every block type in one place? Browse [the block reference](examples/block-reference.dx) for a complete catalogue with working examples.',
  '::end',
].join('\n');

const tutorial = [
  '::heading level=1 id=tutorial-heading',
  'Tutorial',
  '::end',
  '',
  '::quote id=tutorial-intro',
  'This tutorial walks you through everything you need to write, edit, and style documents in .dx. It takes about ten minutes.',
  '::end',
  '',
  '::heading level=2 id=opening-a-doc',
  'Opening a document',
  '::end',
  '',
  '::paragraph id=opening-desc',
  'Any file with a .dx extension opens automatically in the structured editor. Click any block in the view to open its source. Edit the text and press Escape or click outside the block to commit the change.',
  '::end',
  '',
  '::heading level=2 id=block-types-overview',
  'Block types',
  '::end',
  '',
  '::paragraph id=block-types-intro',
  'Every document is a sequence of typed blocks. The full catalogue is in [the block reference](examples/block-reference.dx). The most common ones are:',
  '::end',
  '',
  '::bulleted-list id=block-type-summary',
  '- heading — section titles at levels 1 through 4',
  '- paragraph — body text, supports [links to other .dx docs](examples/block-reference.dx)',
  '- quote — pull quotes and callout text',
  '- bulleted-list and numbered-list — unordered and ordered items',
  '- code — source blocks with an optional language tag for display',
  '- image — embedded images with alt text',
  '- rule — horizontal dividers between sections',
  '::end',
  '',
  '::heading level=2 id=writing-your-first-doc',
  'Writing your first document',
  '::end',
  '',
  '::numbered-list id=first-doc-steps',
  '- The editor opens any .dx file automatically. Create a new file with a .dx extension in your workspace to get started.',
  '- The new file opens with a single empty paragraph block. Click it to start editing.',
  '- Type your content and press Escape or click outside the block to confirm.',
  '- Add a heading or any other block type using the block menu between blocks.',
  '- Save with Cmd+S (macOS) or Ctrl+S (Windows/Linux). The stub on disk updates and the archive is rewritten.',
  '::end',
  '',
  '::heading level=2 id=styling-blocks',
  'Styling blocks',
  '::end',
  '',
  '::paragraph id=styling-desc',
  'Every block has an id and an optional class. To style a block, hover over it and click the CSS icon. A scoped editor opens pre-filled with the selector for that block. Write your CSS declarations inside the braces and close the editor. Styles are document content — they are stored and versioned like any other block.',
  '::end',
  '',
  '::code id=styling-example lang=css',
  '/* Target a specific block by its id */',
  '#tutorial-heading {',
  '  font-size: clamp(2rem, 4vw, 3rem);',
  '  letter-spacing: -0.03em;',
  '}',
  '',
  '/* Target any block with a shared class */',
  '.callout {',
  '  background: var(--surface-2);',
  '  border-radius: 6px;',
  '  padding: 1rem 1.25rem;',
  '}',
  '::end',
  '',
  '::heading level=2 id=querying-with-agents',
  'Querying with an agent',
  '::end',
  '',
  '::paragraph id=agent-desc',
  'The .dx MCP server exposes all documents in your workspace to any connected agent. Once the server is running, an agent can list documents, retrieve a document by path or id, search across all block content, and write back edits via structured calls. No markdown parsing, no fragile regex — just typed blocks.',
  '::end',
  '',
  '::heading level=2 id=whats-next',
  'What to explore next',
  '::end',
  '',
  '::paragraph id=next-steps',
  'You now know how .dx works end-to-end. See [the block reference](examples/block-reference.dx) for the complete block catalogue, or go back to [the welcome doc](examples/welcome.dx) for a summary of the design principles.',
  '::end',
].join('\n');

const blockReference = [
  '::heading level=1 id=block-reference-heading',
  'Block Reference',
  '::end',
  '',
  '::quote id=block-reference-intro',
  'Every block type that .dx supports, shown with a live working example. Use this as your catalogue when writing documents.',
  '::end',
  '',
  '::heading level=2 id=heading-block',
  'Heading',
  '::end',
  '',
  '::paragraph id=heading-desc',
  'Section titles at levels 1 through 4. Level 1 is the document title. Levels 2 through 4 are subsections.',
  '::end',
  '',
  '::heading level=3 id=heading-example-label',
  'This is a level-3 heading',
  '::end',
  '',
  '::heading level=2 id=paragraph-block',
  'Paragraph',
  '::end',
  '',
  '::paragraph id=paragraph-desc',
  'Body text. The default block type. Supports [inline links to other .dx documents](examples/welcome.dx) using the same bracket syntax as Markdown links. The linked path must be a relative .dx path.',
  '::end',
  '',
  '::heading level=2 id=quote-block',
  'Quote',
  '::end',
  '',
  '::quote id=quote-example',
  'Pull quotes stand out from body text. Use them for key insights, design principles, or anything you want a reader to notice immediately.',
  '::end',
  '',
  '::heading level=2 id=bulleted-list-block',
  'Bulleted list',
  '::end',
  '',
  '::paragraph id=bulleted-list-desc',
  'Unordered items. Each line in the source is a separate list item.',
  '::end',
  '',
  '::bulleted-list id=bulleted-list-example',
  '- First item in the list',
  '- Second item in the list',
  '- Third item, [with a link](examples/tutorial.dx) to another document',
  '::end',
  '',
  '::heading level=2 id=numbered-list-block',
  'Numbered list',
  '::end',
  '',
  '::paragraph id=numbered-list-desc',
  'Ordered items. The renderer applies sequential numbers automatically.',
  '::end',
  '',
  '::numbered-list id=numbered-list-example',
  '- Open the document in the editor',
  '- Click a block to edit it',
  '- Press Escape to confirm the edit',
  '- Save with Cmd+S or Ctrl+S',
  '::end',
  '',
  '::heading level=2 id=code-block',
  'Code',
  '::end',
  '',
  '::paragraph id=code-desc',
  'Source blocks with an optional language tag. The editor displays the raw text in a monospace font. When the language is css, the block is eligible for scoped style activation via the CSS editor.',
  '::end',
  '',
  '::code id=code-example lang=js',
  'function greet(name) {',
  '  return `Hello, ${name}.`;',
  '}',
  '::end',
  '',
  '::heading level=2 id=image-block',
  'Image',
  '::end',
  '',
  '::paragraph id=image-desc',
  'Embedded images with alt text for accessibility. The src attribute holds a relative path or a data URI for inline images uploaded through the editor.',
  '::end',
  '',
  '::heading level=2 id=rule-block',
  'Rule',
  '::end',
  '',
  '::paragraph id=rule-desc',
  'A horizontal divider between sections. It has no content — just the block declaration.',
  '::end',
  '',
  '::rule id=rule-example',
  '::end',
  '',
  '::paragraph id=rule-after',
  'The rule above separates this block from the previous section.',
  '::end',
  '',
  '::heading level=2 id=back-link',
  'Return',
  '::end',
  '',
  '::paragraph id=back-link-text',
  'Back to [the welcome doc](examples/welcome.dx) or continue with [the tutorial](examples/tutorial.dx).',
  '::end',
].join('\n');

// ---------------------------------------------------------------------------
// Write production docs
// ---------------------------------------------------------------------------

console.log('Writing examples/welcome.dx...');
await saveDocumentSourceByRelativePath(root, runtime, 'examples/welcome.dx', welcome);

console.log('Writing examples/tutorial.dx...');
await saveDocumentSourceByRelativePath(root, runtime, 'examples/tutorial.dx', tutorial);

console.log('Writing examples/block-reference.dx...');
await saveDocumentSourceByRelativePath(root, runtime, 'examples/block-reference.dx', blockReference);

// ---------------------------------------------------------------------------
// Remove junk test files (stubs from disk)
// ---------------------------------------------------------------------------

const junkFiles = [
  'examples/chat-tool-visibility-test.dx',
  'examples/chat-tool-visibility-test-2.dx',
  'examples/id-test-1778863390562.dx',
  'examples/interaction-test-1778863326155.dx',
  'examples/interaction-test-1778863373971.dx',
  'examples/viewer-interaction-test.dx',
  'research/grill-with-docs.dx',
  'research/video-notes.dx',
];

console.log('\nRemoving junk files...');
for (const rel of junkFiles) {
  const abs = path.resolve(root, rel);
  try {
    await rm(abs);
    console.log('  deleted:', rel);
  } catch (e) {
    console.log('  skip:', rel, '(' + e.code + ')');
  }
}

// Remove research dir if empty
try {
  await rmdir(path.resolve(root, 'research'));
  console.log('  removed research/ directory');
} catch {
  // May not be empty (assets dir) — that's fine
}

// Verify the new docs read back correctly
console.log('\nVerifying round-trips...');
const { readDocArchive } = await import('../../build/runtime/src/doc-archive.js');
for (const rel of ['examples/welcome.dx', 'examples/tutorial.dx', 'examples/block-reference.dx']) {
  const doc = await readDocArchive(root, path.resolve(root, rel));
  const title = doc.blocks.find(b => b.type === 'heading' && b.level === 1)?.text || '?';
  console.log(' ', rel, '->', title, '(' + doc.blocks.length + ' blocks)');
}

console.log('\nDone.');
