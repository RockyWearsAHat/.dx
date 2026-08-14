/**
 * The geometry engine's own door — `board_edge_layout` and `board_edge_preview` — against
 * the engine the VS Code extension actually loads.
 *
 *   node --test "editor/vscode/test/*.test.mjs"
 *
 * `doc-core` already tests the geometry itself, thoroughly (`render::board`'s own suite),
 * and `doc-wasm` tests the JSON boundary in Rust (`cargo test -p doc-wasm`). What this file
 * tests is the part neither of those reaches: that a board laid out through this door from
 * the exact wasm build a host loads matches what the same board rendered statically, and —
 * the test that actually keeps report-b1031ce7 closed — that `editor/surface/edit.js` no
 * longer answers a geometry question itself.
 */

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const engine = createRequire(import.meta.url)(join(here, '..', 'wasm', 'doc_wasm.js'));
const SURFACE = readFileSync(join(here, '..', '..', 'surface', 'edit.js'), 'utf8');

test('the geometry the surface needs is exported', () => {
  assert.equal(typeof engine.board_edge_layout, 'function');
  assert.equal(typeof engine.board_edge_preview, 'function');
});

test('the surface no longer answers a geometry question itself', () => {
  // Every one of these was a hand-kept copy of `render::board`'s own math, deleted when the
  // surface started asking the engine through `board_edge_layout`/`board_edge_preview`
  // instead. A name reappearing here means the duplication came back.
  for (const name of [
    'cubicAt',
    'curveBetween',
    'controlsFor',
    'leadOf',
    'autoSides',
    'clearestSides',
    'crossingSine',
    'hiddenSamples',
    'midpointOn',
    'boxHolds',
    'segmentHits',
    'reachOf',
    'spansAcross',
    'anchorOn',
    'labelSpot',
    'centreOf',
  ]) {
    assert.doesNotMatch(
      SURFACE,
      new RegExp(`function ${name}\\(`),
      `edit.js still defines ${name} — the geometry belongs to render::board now`
    );
  }
  // What the surface still legitimately decides itself: a DOM measurement, and a UI-only
  // drop-point snap the engine never sees a pointer to make.
  assert.match(SURFACE, /function boxOf\(/);
  assert.match(SURFACE, /function sideNearest\(/);
});

const BOARD =
  '::board id=plan height=520\n- a x=0 y=0 w=200 h=100 to=b\n- b x=600 y=0 w=200 h=100\n::end\n\n' +
  '::paragraph id=a hidden\nFrom.\n::end\n\n' +
  '::paragraph id=b hidden\nTo.\n::end\n';

test('a board laid out from its stated boxes is the board the renderer drew', () => {
  const page = engine.render_html(BOARD, 'light', true);
  const staticPath = page.match(/<path data-from="a" data-to="b"[^>]*\sd="([^"]+)"/);
  assert.ok(staticPath, `no static edge found in:\n${page}`);

  const layout = JSON.parse(
    engine.board_edge_layout(
      JSON.stringify({
        scale: 1,
        nodes: [
          { id: 'a', x: 0, y: 0, w: 200, h: 100 },
          { id: 'b', x: 600, y: 0, w: 200, h: 100 },
        ],
        edges: [{ from: 'a', to: 'b' }],
      })
    )
  );
  assert.equal(layout.length, 1);
  assert.equal(
    layout[0].path,
    staticPath[1],
    'a measured re-layout must draw the exact curve the static render did'
  );
});

test('an unpinned end is chosen and a pinned end is kept', () => {
  const layout = JSON.parse(
    engine.board_edge_layout(
      JSON.stringify({
        nodes: [
          { id: 'a', x: 0, y: 0, w: 200, h: 100 },
          { id: 'b', x: 0, y: 400, w: 200, h: 100 },
        ],
        edges: [{ from: 'a', to: 'b', fromSide: 'r' }],
      })
    )
  );
  assert.equal(layout[0].fromSide, 'r', 'the pin must be honoured');
  assert.equal(layout[0].toSide, 't', "the router's own choice for the unpinned end");
});

test('a drag preview is a path from the side it left', () => {
  const preview = JSON.parse(
    engine.board_edge_preview(
      JSON.stringify({
        from: { box: { x: 0, y: 0, w: 200, h: 100 }, side: 'r' },
        to: { x: 500, y: 40 },
      })
    )
  );
  assert.match(preview.path, /^M /);
});

test('a malformed spec is an error, not an empty path', () => {
  assert.throws(() => engine.board_edge_layout('not json'), /layout request/);
  assert.throws(() => engine.board_edge_preview('{}'), /preview request/);
});

test('both hosts hand the surface its engine', () => {
  const preview = readFileSync(join(here, '..', 'src', 'preview.ts'), 'utf8');
  const swift = readFileSync(
    join(here, '..', '..', '..', 'packaging', 'app', 'Editor.swift'),
    'utf8'
  );
  assert.match(preview, /op:\s*'engine'/, 'preview.ts must offer the engine op');
  assert.match(preview, /dxEditor\.engine\(/, 'preview.ts must call it after attaching');
  assert.match(swift, /op:\s*'engine'/, 'Editor.swift must offer the engine op');
  assert.match(swift, /dxEditor\.engine\(/, 'Editor.swift must call it after attaching');
});
