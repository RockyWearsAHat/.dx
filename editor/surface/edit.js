/**
 * Editing a rendered page in place — the one implementation, for every surface that has a
 * reader in front of it.
 *
 * DX.app and the VS Code webview both load this file. Neither one has its own version of
 * "what clicking a paragraph does", because two versions of that is two products: a document
 * would behave one way on a Mac and another in an editor, and the difference would be
 * discovered by a person who had already lost a sentence to it.
 *
 * # The page never stops being the page
 * Two rules, and everything here is one of them.
 *
 * **The field is the block — and clicking a block expands its controls.** Clicking a block
 * replaces its rendered content with the block's own editing form, in the block's place:
 * the writer's `::kind attrs` tag line in a quiet mono above (hidden for a plain paragraph,
 * which needs no machinery over it), and a field holding the exact body beneath, in the
 * block's own type — a heading is written at heading size, a listing in its own column, and
 * the page does not move when reading becomes writing. A prose field stays *dressed as what
 * it says* while it is written: the engine's field renderer decorates the source in place,
 * so `**bold**` is set in bold with its `**` still on the line — the marks are visible, the
 * meaning is visible, and the characters are the block's own. ⌘/Ctrl+B, I, E, and K write
 * the marks over the selection. The tag line is a control: rewrite `::code lang=python` as
 * `::quote` and saving retypes the block. Typing `::` at the start of a plain paragraph
 * promotes it. Ghost text and a small menu at the caret complete block kinds, their
 * attributes, and remembered values; Tab or Return accepts, Escape dismisses. Escape with
 * no menu open puts the rendered content back untouched. Blank paper is writable: a click
 * on the sheet starts the next paragraph after the block above the click, and a paragraph
 * left empty is removed on close rather than saved. A code block marked `run` carries a
 * quiet `run` control in its own label line, which asks the host to execute that one block.
 *
 * **Saving replaces content, never the page.** A commit hands back the re-rendered document
 * and one element is swapped for it. Nothing navigates, so nothing flashes, the scroll does
 * not move, and there is no position to restore afterwards.
 *
 * # What it does not do
 * It does not parse `.dx`, render it, or decide what a block is — not even the decoration
 * inside the field, which is `doc-core`'s `field_html` reached through the host. This file
 * knows how to run a caret and a keyboard. Everything else is asked of the host, which
 * answers through `doc-core` — the same operations an agent calls (`dx source`, `dx set`,
 * `dx set --header`, `dx insert`, `dx remove`, `dx render --field`, `dx run --only`), so a
 * person typing on the page and an agent editing the file are doing the identical thing.
 *
 * # The host contract
 * `attach(host)` where host provides, each returning a promise:
 *
 * - `source(id)` — the exact characters of one block's body
 * - `parts(id)` — `{ header, body }`: the writer's own `::kind attrs` line, and the body
 * - `commit(id, text, then)` — save one block's body and hand back the document. `then` is
 *   `'insert'` when the reader pressed Return, which asks for the next paragraph in the
 *   *same* call, because a second round trip would race the first one's re-render.
 * - `replace(id, header, body, then)` — save the whole block, tag line and all: a changed
 *   header retypes the block, an empty one means the text is plain prose.
 * - `remove(id)` — take a block out, and hand back the document
 * - `decorate(text)` — optional: the field's text as the engine's decorated HTML. Without
 *   it, prose fields are written plain — a host that cannot decorate still edits.
 * - `run(id)` — optional: execute one `run` block and hand back the document. Without it,
 *   no run control appears — a host that cannot execute still edits. A runnable block that
 *   was *edited* runs by itself the moment its field closes: the reader changed the code,
 *   so the page shows what the changed code does, without a second gesture.
 * - `check(id, item)` — optional: tick or untick one box of a checklist, by its position
 *   counting from zero, and hand back the document. Without it a checklist is read the way
 *   it is written and ticked by editing the line, which is what every host did before.
 * - `board(id, action, spec)` — optional: one operation on a `::board` block — `place`,
 *   `add`, `arrange`, `detach`, `link`, or `unlink`. `spec` carries what that action needs:
 *   `node`/`x`/`y`/`w`/`h` for a placement, `arrangement` (`"node,x,y,w,h …"`) for a group
 *   of them, `from`/`to` plus `fromSide`/`toSide` for the edge a reader dragged between two
 *   nodes' sides. The host performs it through the engine (`dx board`), because the board's
 *   line grammar is the engine's: this file measures pixels and never writes a line itself.
 *   Without it, a board still pans and zooms — a reader can look, and cannot rearrange.
 *
 * `commit`, `replace`, `remove`, `run`, `check`, and `board` resolve with `{ document, focus }`:
 * `document` is the re-rendered `.dx-doc` container, and `focus` is the id to open
 * afterwards, if any. A host may offer more than this; nothing else is asked of it.
 */

(function () {
  'use strict';

  /** Blocks a reader may edit: the ones a person wrote. */
  const EDITABLE = [
    'paragraph',
    'heading',
    'quote',
    'code',
    'bulleted-list',
    'numbered-list',
    'checklist',
    'html',
    'svg',
    'mermaid',
    'view',
  ];

  /**
   * Blocks that are never edited, whatever they look like.
   *
   * `output` is what the code produced — editing it would be writing down a result that was
   * never computed. `nav` is derived from the document's own headings. Both are windows onto
   * something else, and the way to change them is to change the something else.
   */
  const READ_ONLY = ['output', 'nav'];

  /**
   * Blocks where Return types a newline instead of finishing.
   *
   * A line break is content in these, so it takes a modifier to say "done".
   */
  const MULTILINE = ['code', 'bulleted-list', 'numbered-list', 'checklist', 'html', 'svg', 'mermaid', 'view'];

  /**
   * Blocks whose body is source — a listing, a drawing, author markup. Their field is the
   * mono column and is never decorated: source means exactly what it spells.
   */
  const SOURCE = ['code', 'html', 'svg', 'mermaid', 'view'];

  /**
   * What the tag line can say. One entry per kind a person may write, each completing to a
   * line that is already a valid header — the attribute a kind cannot live without arrives
   * with it, caret ready for its value.
   */
  const KINDS = [
    '::paragraph',
    '::heading level=2',
    '::quote',
    '::code lang=',
    '::bulleted-list',
    '::numbered-list',
    '::checklist',
    '::rule',
    '::image src=',
    '::html',
    '::svg',
    '::mermaid',
    '::board',
    '::view src=',
  ];

  /** The attributes each kind's header may carry, beyond the universal `id`/`class`/`hidden`. */
  const ATTRS = {
    heading: ['level'],
    code: ['lang', 'src', 'run', 'deps', 'reads', 'timeout', 'format'],
    image: ['src'],
    stylesheet: ['href', 'media'],
    script: ['type', 'src', 'module'],
    nav: ['label'],
    board: ['height'],
    view: ['src', 'width', 'height'],
  };

  /** Attributes that are flags: present or absent, never `key=value`. */
  const BARE_ATTRS = ['run', 'hidden', 'module'];

  /** Seed values for attribute completion; remembered values join them. */
  const VALUES = {
    level: ['1', '2', '3', '4'],
    lang: ['python', 'node', 'bash', 'rust', 'go', 'deno'],
  };

  /**
   * The formatting keys: the mark each writes over the selection. The *meaning* of a mark
   * is the engine's; these four characters are the same fixed vocabulary the completion
   * lists above are.
   */
  const FORMAT_KEYS = { b: '**', i: '*', e: '`' };

  /** Where accepted completions are remembered, across documents. */
  const HISTORY_KEY = 'dx.autocomplete-history.v1';

  /** The most entries any history list keeps — newest win. */
  const HISTORY_LIMIT = 300;

  /** The most undo states a field keeps while it is open. */
  const UNDO_LIMIT = 200;

  /** How far a board zooms: out to a tenth, in to two-and-a-half times. */
  const ZOOM_MIN = 0.1;
  const ZOOM_MAX = 2.5;

  /**
   * The most a fit magnifies — `render::board`'s own bound, so the board a reader opens is
   * the size the same board is on github.com and in a PNG.
   */
  const FIT_MAX = 1.5;

  /** Breathing room a board's fit leaves around its nodes, in viewport pixels. */
  const BOARD_PADDING = 24;

  /** How much of the content must stay inside the viewport when panning, in pixels. */
  const PAN_KEEP = 40;

  /** The smallest box a node can be dragged to, in canvas pixels. */
  const NODE_MIN_WIDTH = 120;
  const NODE_MIN_HEIGHT = 60;

  /** The host, once attached. */
  let host = null;

  /**
   * The block currently open, if any:
   * `{ block, id, header, field, headerShown, demoted, restore, original, past, future }`.
   */
  let open = null;

  /** The completion menu, when one is showing: `{ holder, ghost, field, items, selected, start }`. */
  let menu = null;

  /** Whatever the last save is still doing, so a click cannot overtake it. */
  let settling = Promise.resolve();

  /** True while focus is moved between the tag line and the body on purpose. */
  let shuttling = false;

  /** True while an input method is composing — decoration waits for the composed text. */
  let composing = false;

  /** Ticket for the newest decoration request; older answers are dropped on arrival. */
  let decorating = 0;

  /**
   * Whether an editor was open when the pointer went down.
   *
   * A click on blank paper while writing is a way of saying "done" — it must only close
   * the field (the blur already saved it), never also start a new paragraph. Only a click
   * that began with nothing open creates one.
   */
  let openAtPointerDown = false;

  /**
   * Each board's view — `{ s, tx, ty }`, a scale and a translation — by board id.
   *
   * Kept across saves, because every save re-renders the sheet: a reader who dragged a
   * node must not be snapped back to the fitted view for having done so.
   */
  const boardViews = new Map();

  /**
   * What is picked on a board, if anything: `{ board, node }` for a node,
   * `{ board, from, to }` for an edge. Delete removes it; Escape lets go of it.
   */
  let boardPick = null;

  /** The node box editor, when one is showing: `{ holder, board, node }`. */
  let sizeMenu = null;

  /**
   * Start editing on this page.
   *
   * @param {object} bridge - the host contract described above.
   */
  function attach(bridge) {
    host = bridge;
    document.addEventListener('mousedown', () => { openAtPointerDown = !!open; }, true);
    document.addEventListener('click', onClick, true);
    document.addEventListener('keydown', onKeyDown, true);
    document.addEventListener('keydown', onBoardKey, true);
    document.body.classList.add('dx-editable');
    dressRunnables(document);
    dressChecks(document);
    dressBoards(document);
  }

  /** The block element an event landed in, or null if it landed outside every block. */
  function blockAt(node) {
    const element = node instanceof Element ? node : node && node.parentElement;
    const block = element && element.closest('[data-block-id]');
    if (!block) return null;
    const kind = kindOf(block);
    if (READ_ONLY.includes(kind) || !EDITABLE.includes(kind)) return null;
    return block;
  }

  /**
   * A block's kind, read from the classes the renderer put on it.
   *
   * The renderer names a code block `dx-code` and an output `dx-output`; everything else is
   * its own element, so the tag is the kind. This is the one place that mapping lives.
   */
  function kindOf(block) {
    if (block.classList.contains('dx-board')) return 'board';
    if (block.classList.contains('dx-code')) return 'code';
    if (block.classList.contains('dx-output')) return 'output';
    if (block.classList.contains('dx-output-rendered')) return 'output';
    if (block.classList.contains('dx-nav')) return 'nav';
    if (block.classList.contains('dx-html')) return 'html';
    if (block.classList.contains('dx-view')) return 'view';
    if (block.classList.contains('dx-svg')) return 'svg';
    if (block.classList.contains('dx-mermaid')) return 'mermaid';
    if (block.classList.contains('dx-checklist')) return 'checklist';
    const tag = block.tagName.toLowerCase();
    if (tag === 'p') return 'paragraph';
    if (/^h[1-4]$/.test(tag)) return 'heading';
    if (tag === 'blockquote') return 'quote';
    if (tag === 'ul') return 'bulleted-list';
    if (tag === 'ol') return 'numbered-list';
    return tag;
  }

  /**
   * The kind the open block is being written as: what its tag line says, or what it was.
   *
   * A paragraph whose tag now reads `::checklist` takes checklist keys — Return types an
   * item instead of finishing — before any save, for the same reason `dress` changes its
   * type: the block answers as what the reader says it is, not what it used to be.
   */
  function effectiveKind() {
    if (!open) return '';
    const named = open.headerShown && open.header.value.match(/^\s*::([a-z-]+)/i);
    return named ? named[1].toLowerCase() : kindOf(open.block);
  }

  /** Open the block that was clicked. */
  function onClick(event) {
    // A link is a link. Opening an editor under someone trying to follow one is the kind of
    // thing that makes an editable page feel like a trap.
    if (event.target.closest('a')) return;
    // The fold on a code block is a control, not text: let it open the listing. The run
    // control inside it has its own listener, which this early return leaves in charge.
    if (event.target.closest('summary')) return;
    // A checklist's box is a control too, and it is the one thing on a checklist a reader
    // is likelier to mean than the words: ticking it off is what a checklist is *for*. So
    // the click ticks the box and stops — it does not also open the list for writing.
    const mark = host.check && event.target.closest('.dx-mark[data-check]');
    if (mark) {
      event.preventDefault();
      event.stopPropagation();
      toggleCheck(mark);
      return;
    }
    // Clicks inside the open editor belong to its caret and its menu.
    if (open && open.block.contains(event.target)) return;

    const block = blockAt(event.target);
    if (!block) {
      blankClick(event);
      return;
    }
    if (open && open.block === block) return;

    event.preventDefault();
    event.stopPropagation();
    void enter(block.getAttribute('data-block-id'));
  }

  /**
   * A click on the sheet itself starts the next paragraph there.
   *
   * Only the sheet: the margins outside it are not paper, and a click inside any block is
   * not blank. A click that arrived with a field open is the *end* of writing — the blur
   * has already saved it — so it closes and does nothing more; writing resumes on the next
   * click. The paragraph goes where the click was: after the last block whose middle sits
   * above the pointer, which is also how a reader would name the spot.
   */
  function blankClick(event) {
    if (openAtPointerDown || open) return;
    const sheet = document.querySelector('.dx-doc');
    if (!sheet || event.target !== sheet) return;

    let anchor = null;
    for (const candidate of sheet.querySelectorAll('[data-block-id]')) {
      const box = candidate.getBoundingClientRect();
      if (box.top + box.height / 2 < event.clientY) anchor = candidate;
    }
    if (!anchor) return;
    const id = anchor.getAttribute('data-block-id');

    settling = settling
      .then(() => host.source(id))
      .then((text) => host.commit(id, text, 'insert'))
      .then(apply)
      .catch((error) => report(anchor, String(error)));
  }

  /**
   * Open the block called `id`, once whatever was open has finished saving.
   *
   * By id rather than by element, because saving re-renders the document: the element that
   * was clicked may not be on the page by the time the click is answered, while the block it
   * stood for still is.
   */
  async function enter(id) {
    if (!id) return;
    await settling;
    const block = document.querySelector(`[data-block-id="${cssEscape(id)}"]`);
    if (block) await edit(block);
  }

  /**
   * Expand a block into its editing form: the tag line above, the body field beneath.
   *
   * Both are the block's children, so the body inherits the block's own type and the page
   * does not shift as the reader starts writing. The tag line is hidden over a plain
   * paragraph — prose needs no machinery — and appears the moment the reader types `::`.
   *
   * A prose body is a `contenteditable` element rather than a textarea, because a textarea
   * can only show characters one way. The element holds the exact source; the engine's
   * decoration dresses it in place, and every read and write goes through `fieldText` /
   * `spliceField`, which speak in source offsets on both element kinds.
   */
  async function edit(block) {
    const id = block.getAttribute('data-block-id');
    if (!id) return;
    // Normally nothing is open here — a save already closed it. A save the host *refused*
    // is the exception: it stays open so the reader can retry, and moving on to another
    // block abandons the attempt rather than leaving two fields on the page.
    if (open) close();

    let parts = null;
    try {
      if (host.parts) {
        parts = await host.parts(id);
      } else {
        parts = { header: '', body: await host.source(id) };
      }
    } catch (error) {
      report(block, String(error));
      return;
    }

    const header = document.createElement('textarea');
    header.className = 'dx-header';
    header.value = parts.header;
    header.rows = 1;
    header.spellcheck = false;

    const rich = !SOURCE.includes(kindOf(block));
    let field;
    if (rich) {
      field = document.createElement('div');
      field.className = 'dx-field dx-field-rich';
      field.contentEditable = 'true';
      field.textContent = parts.body;
    } else {
      field = document.createElement('textarea');
      field.className = 'dx-field';
      field.value = parts.body;
      field.rows = 1;
    }
    // What is being written is source, and a proofreader underlining `**strong**` is a
    // second surface on the sheet.
    field.spellcheck = false;

    const restore = block.innerHTML;
    block.innerHTML = '';
    // A folded listing is a `details`, and a `details` without a `summary` invents a default
    // one. Editing means the fold is open and its label is out of the way — a hidden stub
    // keeps the browser from drawing a marker where the label was.
    if (block.tagName === 'DETAILS') {
      block.open = true;
      const stub = document.createElement('summary');
      stub.className = 'dx-fold-stub';
      block.appendChild(stub);
    }
    block.appendChild(header);
    block.appendChild(field);
    block.classList.add('dx-editing');

    open = {
      block,
      id,
      header,
      field,
      // A paragraph is plain prose: its `::paragraph id=…` line is machinery, not content,
      // and showing it would make every sentence wear its plumbing. A code block hides its
      // tag line too — a person editing code wants the code, and the listing's own label
      // already says what it is; ArrowUp at the very start reveals the line when the block
      // needs retyping. Every other kind's tag line is the control that says what it is.
      headerShown:
        kindOf(block) !== 'paragraph' && kindOf(block) !== 'code' && parts.header !== '',
      demoted: false,
      restore,
      original: { header: parts.header, body: parts.body },
      // The undo a browser gives a textarea for free is rebuilt for the rich field, because
      // redrawing the decoration replaces the nodes the browser's own undo remembered.
      past: [],
      future: [],
    };
    if (!open.headerShown) header.classList.add('dx-header-hidden');

    grow(header);
    grow(field);
    field.focus();
    setSelection(field, parts.body.length, parts.body.length);

    header.addEventListener('input', () => { grow(header); dress(); suggest(header, false); });
    field.addEventListener('input', () => {
      if (composing) return;
      grow(field);
      decorate(field);
      promote();
      suggest(field, false);
    });
    if (rich) {
      field.addEventListener('beforeinput', () => { if (!composing) snapshot(); });
      field.addEventListener('compositionstart', () => { composing = true; });
      field.addEventListener('compositionend', () => {
        composing = false;
        decorate(field);
        suggest(field, false);
      });
      // Paste lands as characters, whatever it was copied from: the field holds source,
      // and a pasted heading dragging its own markup in would not be the reader's text.
      field.addEventListener('paste', (event) => {
        event.preventDefault();
        const text = (event.clipboardData || window.clipboardData).getData('text/plain');
        if (text === '') return;
        snapshot();
        const at = selection(field);
        spliceField(field, at.start, at.end, text);
        const landing = at.start + text.length;
        setSelection(field, landing, landing);
        suggest(field, false);
      });
    }
    dress();
    decorate(field);
    header.addEventListener('click', () => suggest(header, false));
    field.addEventListener('click', () => suggest(field, false));
    header.addEventListener('blur', (event) => void leave(event));
    field.addEventListener('blur', (event) => void leave(event));
  }

  // -------------------------------------------------------------------------
  // The field, addressed in source offsets — one vocabulary for both elements.
  // -------------------------------------------------------------------------

  /** Whether this field is the decorated contenteditable rather than a textarea. */
  function isRich(field) {
    return field.tagName !== 'TEXTAREA';
  }

  /** The exact source characters a field holds. */
  function fieldText(field) {
    if (!isRich(field)) return field.value;
    let out = '';
    const walk = (node) => {
      for (const child of node.childNodes) {
        if (child.nodeType === Node.TEXT_NODE) out += child.data;
        else if (child.nodeName === 'BR') out += '\n';
        else walk(child);
      }
    };
    walk(field);
    return out;
  }

  /** Put `text` in a field, replacing what it held, and redraw its decoration. */
  function setFieldText(field, text) {
    if (isRich(field)) {
      field.textContent = text;
      decorate(field);
    } else {
      field.value = text;
      grow(field);
    }
  }

  /** Replace the characters from `from` to `to` with `insert`, in source offsets. */
  function spliceField(field, from, to, insert) {
    const text = fieldText(field);
    setFieldText(field, text.slice(0, from) + insert + text.slice(to));
  }

  /** The selection in source offsets: `{ start, end }`, collapsed to the end when unknown. */
  function selection(field) {
    if (!isRich(field)) return { start: field.selectionStart, end: field.selectionEnd };
    const length = fieldText(field).length;
    const picked = window.getSelection();
    if (!picked || picked.rangeCount === 0) return { start: length, end: length };
    const range = picked.getRangeAt(0);
    if (!field.contains(range.startContainer)) return { start: length, end: length };
    return {
      start: offsetAt(field, range.startContainer, range.startOffset),
      end: offsetAt(field, range.endContainer, range.endOffset),
    };
  }

  /** Put the selection at source offsets `start`–`end`. */
  function setSelection(field, start, end) {
    if (!isRich(field)) {
      field.setSelectionRange(start, end);
      return;
    }
    const from = positionAt(field, start);
    const to = positionAt(field, end);
    const range = document.createRange();
    range.setStart(from.node, from.offset);
    range.setEnd(to.node, to.offset);
    const picked = window.getSelection();
    picked.removeAllRanges();
    picked.addRange(range);
  }

  /** The source offset of a DOM position inside `field`. */
  function offsetAt(field, container, offset) {
    let total = 0;
    let done = false;
    const walk = (node) => {
      if (done) return;
      if (node === container) {
        if (node.nodeType === Node.TEXT_NODE) total += offset;
        else for (let index = 0; index < offset; index += 1) measure(node.childNodes[index]);
        done = true;
        return;
      }
      if (node.nodeType === Node.TEXT_NODE) total += node.data.length;
      else if (node.nodeName === 'BR') total += 1;
      else for (const child of node.childNodes) { walk(child); if (done) return; }
    };
    const measure = (node) => {
      if (!node) return;
      if (node.nodeType === Node.TEXT_NODE) total += node.data.length;
      else if (node.nodeName === 'BR') total += 1;
      else for (const child of node.childNodes) measure(child);
    };
    walk(field);
    return total;
  }

  /** The DOM position of a source offset inside `field`. */
  function positionAt(field, offset) {
    let remaining = offset;
    let last = { node: field, offset: 0 };
    const walk = (node) => {
      for (const child of node.childNodes) {
        if (child.nodeType === Node.TEXT_NODE) {
          if (remaining <= child.data.length) return { node: child, offset: remaining };
          remaining -= child.data.length;
          last = { node: child, offset: child.data.length };
        } else if (child.nodeName === 'BR') {
          const parent = child.parentNode;
          const at = Array.prototype.indexOf.call(parent.childNodes, child);
          if (remaining === 0) return { node: parent, offset: at };
          remaining -= 1;
          last = { node: parent, offset: at + 1 };
        } else {
          const found = walk(child);
          if (found) return found;
        }
      }
      return null;
    };
    return walk(field) || last;
  }

  /**
   * Redraw a rich field's decoration from the engine, keeping the caret on its character.
   *
   * The call is asynchronous — the engine may be across a message channel or behind a
   * process — so each request takes a ticket, and an answer that arrives after a newer
   * keystroke, or after the text moved on, is dropped rather than painted over the reader.
   * A host with no `decorate`, or a failing one, leaves the field plain: an undecorated
   * field is still the exact source, which is the courtesy `remember` extends to a blocked
   * history store.
   */
  function decorate(field) {
    if (!open || field !== open.field || !isRich(field)) return;

    // Source is source: a paragraph being retyped as `::code` drops its dress entirely.
    if (open.block.classList.contains('dx-writes-source') || !host.decorate) {
      if (field.childNodes.length === 1 && field.firstChild.nodeType === Node.TEXT_NODE) return;
      const text = fieldText(field);
      const kept = document.activeElement === field ? selection(field) : null;
      field.textContent = text;
      if (kept) setSelection(field, kept.start, kept.end);
      return;
    }

    const text = fieldText(field);
    const ticket = ++decorating;
    Promise.resolve(host.decorate(text))
      .then((markup) => {
        if (ticket !== decorating || !open || open.field !== field) return;
        if (fieldText(field) !== text) return;
        const kept = document.activeElement === field ? selection(field) : null;
        field.innerHTML = markup;
        if (kept) setSelection(field, kept.start, kept.end);
      })
      .catch(() => {
        // The plain characters are already on the page; losing the dress loses nothing.
      });
  }

  // -------------------------------------------------------------------------
  // Formatting and undo — the keyboard the rich field owes the reader.
  // -------------------------------------------------------------------------

  /**
   * ⌘/Ctrl+B, I, E wrap the selection in its mark — or unwrap it, when it is already
   * worn — and ⌘/Ctrl+K writes the selection into a link with the caret in the target.
   * Only the characters change; what a mark *means* stays the engine's business.
   */
  function formatKey(event, field) {
    if (!(event.metaKey || event.ctrlKey) || event.altKey || event.shiftKey) return false;
    const key = event.key.toLowerCase();
    if (key === 'k') {
      const at = selection(field);
      const text = fieldText(field);
      const label = text.slice(at.start, at.end);
      snapshot();
      spliceField(field, at.start, at.end, `[${label}]()`);
      const target = at.start + label.length + 3;
      setSelection(field, target, target);
      return true;
    }
    const marker = FORMAT_KEYS[key];
    if (!marker) return false;

    const at = selection(field);
    const text = fieldText(field);
    const picked = text.slice(at.start, at.end);
    const length = marker.length;
    snapshot();
    if (
      picked.length >= 2 * length &&
      picked.startsWith(marker) &&
      picked.endsWith(marker) &&
      picked !== marker + marker
    ) {
      // The marks came along in the selection: take them off.
      const inner = picked.slice(length, picked.length - length);
      spliceField(field, at.start, at.end, inner);
      setSelection(field, at.start, at.start + inner.length);
    } else if (
      text.slice(at.start - length, at.start) === marker &&
      text.slice(at.end, at.end + length) === marker
    ) {
      // The marks sit just outside it: same intent, same answer.
      spliceField(field, at.start - length, at.end + length, picked);
      setSelection(field, at.start - length, at.start - length + picked.length);
    } else {
      spliceField(field, at.start, at.end, marker + picked + marker);
      setSelection(field, at.start + length, at.start + length + picked.length);
    }
    return true;
  }

  /** Remember the rich field as it stands, so the next change can be undone. */
  function snapshot() {
    if (!open || !isRich(open.field)) return;
    open.past.push({ text: fieldText(open.field), at: selection(open.field) });
    if (open.past.length > UNDO_LIMIT) open.past.shift();
    open.future = [];
  }

  /** Step the rich field back (or forward) through its remembered states. */
  function timeTravel(backward) {
    if (!open) return false;
    const from = backward ? open.past : open.future;
    const onto = backward ? open.future : open.past;
    const state = from.pop();
    if (!state) return false;
    onto.push({ text: fieldText(open.field), at: selection(open.field) });
    setFieldText(open.field, state.text);
    setSelection(open.field, state.at.start, state.at.end);
    dismiss();
    return true;
  }

  /**
   * The body wears the kind the tag line names, before any save.
   *
   * A paragraph whose tag now says `::code` is being written as a listing, and writing a
   * listing in the prose serif until the save lands is a page that dresses the block only
   * after the reader stops looking at it. The block element itself still renders as what it
   * was — only the field's voice follows the tag.
   */
  function dress() {
    if (!open) return;
    const wasSource = open.block.classList.contains('dx-writes-source');
    const source = SOURCE.includes(effectiveKind()) && open.headerShown;
    open.block.classList.toggle('dx-writes-source', !!source);
    if (wasSource !== !!source) decorate(open.field);
  }

  /**
   * `::` typed at the start of a plain paragraph lifts the text into the tag line.
   *
   * This is how a paragraph becomes anything else without a form asking it to: the reader
   * writes the tag where they were writing the sentence, and the completions arrive. Only a
   * single-line body is lifted — a `:` mid-paragraph is punctuation.
   */
  function promote() {
    if (!open || open.headerShown) return;
    // Only prose promotes. A code block opens with its tag line folded away, and a listing
    // whose one line happens to start with `:` is code, not a request to be retyped.
    if (kindOf(open.block) === 'code' && !open.demoted) return;
    const text = fieldText(open.field);
    if (!text.startsWith(':') || text.includes('\n')) return;
    open.headerShown = true;
    open.header.classList.remove('dx-header-hidden');
    open.header.value = text;
    setFieldText(open.field, '');
    grow(open.header);
    dress();
    shuttle(() => {
      open.header.focus();
      open.header.setSelectionRange(text.length, text.length);
    });
    suggest(open.header, false);
  }

  /**
   * Keep a textarea exactly as tall as its text, so the page never scrolls to a hidden
   * line. The rich field is an ordinary element and holds its own height.
   *
   * Never shorter than one line: an empty block measures zero, and a field of no height is
   * a block a reader cannot see the caret in or type into.
   */
  function grow(field) {
    if (isRich(field)) return;
    field.style.height = 'auto';
    const line = parseFloat(getComputedStyle(field).lineHeight) || 16;
    field.style.height = `${Math.max(field.scrollHeight, line)}px`;
  }

  /** Put the block back the way it was rendered, discarding whatever was typed. */
  function close() {
    if (!open) return;
    const { block, restore } = open;
    dismiss();
    open = null;
    block.innerHTML = restore;
    block.classList.remove('dx-editing', 'dx-writes-source');
    // The restored markup carries the run control as inert text — its listener died with
    // the original element — so it is replaced, not kept.
    for (const stale of block.querySelectorAll('.dx-run')) stale.remove();
    dressRunnables(block);
  }

  /** The tag line the save should carry: what is shown, or what a demotion cleared. */
  function effectiveHeader() {
    if (!open) return '';
    if (open.headerShown) return open.header.value;
    return open.demoted ? '' : open.original.header;
  }

  /**
   * A blur leaves the editor only when focus really left it.
   *
   * Moving between the tag line and the body, or clicking the completion menu, is still
   * writing — saving under it would close the editor out from under the reader's caret.
   */
  function leave(event) {
    if (shuttling || !open) return;
    const next = event.relatedTarget;
    if (next && (next === open.header || next === open.field)) return;
    if (next && menu && menu.holder.contains(next)) return;
    return save();
  }

  /**
   * Save what is in the open editor, and put the answer on the page.
   *
   * An untouched tag line saves through `commit` — the body operation every host has always
   * had. A rewritten one saves through `replace`, which is the retype. The returned promise
   * is what a click waits on, so opening the next block cannot overtake saving this one.
   */
  function save(then) {
    if (!open) return settling;
    const editing = open;
    const { id, field, original } = editing;
    const text = fieldText(field);
    const header = effectiveHeader();
    const retyped = header !== original.header;

    // A paragraph with nothing in it renders nothing and can never be seen again — it is
    // not content, it is residue. Finishing one deletes it instead of writing it; asking
    // it for *another* paragraph is answered by keeping the caret where it is.
    if (!retyped && text.trim() === '' && kindOf(editing.block) === 'paragraph') {
      if (then === 'insert') return settling;
      dismiss();
      open = null;
      settling = host
        .remove(id)
        .then(apply)
        .catch((error) => {
          report(editing.block, String(error));
          resume(editing);
        });
      return settling;
    }

    if (!retyped && text === original.body && then !== 'insert') {
      close();
      return settling;
    }
    dismiss();
    open = null;
    settling = (retyped ? host.replace(id, header, text, then) : host.commit(id, text, then))
      .then((result) => {
        apply(result);
        // The reader changed a runnable block and closed it: run it. The code on the page
        // is new, so the output under it is stale the moment the field closes — showing
        // what the changed code *does* is the point of having changed it.
        autoRun(id);
      })
      .catch((error) => {
        report(editing.block, String(error));
        resume(editing);
      });
    return settling;
  }

  /**
   * Run the block called `id`, if the saved page marks it runnable and the host can run.
   *
   * Called only after an *edited* save — an unchanged Escape runs nothing, and a block
   * whose retype renamed it simply is not found, which is the right amount of magic.
   */
  function autoRun(id) {
    const block = document.querySelector(`[data-block-id="${cssEscape(id)}"]`);
    if (!block || !block.classList.contains('dx-runnable')) return;
    const control = block.querySelector('summary > .dx-run');
    if (!control) return;
    runBlock(block, control);
  }

  /**
   * Put a refused save back in the reader's hands.
   *
   * The host said no, so the block still holds the field with the typed text: make it the
   * open state again, focused, so the reader can fix and retry — or press Escape and lose
   * only the attempt. Without this the field was stranded: not open, so Escape, blur, and
   * retry all answered to nothing.
   */
  function resume(editing) {
    if (open || !editing.field.isConnected) return;
    open = editing;
    editing.field.focus();
  }

  /**
   * Show the document the host handed back.
   *
   * One element is replaced, and it is the one holding the blocks. The page itself — its
   * head, its stylesheet, its scroll position, this script — is never reloaded, which is the
   * difference between a document that is being written on and a window that keeps blinking.
   */
  function apply(result) {
    if (!result || typeof result.document !== 'string') return;
    const fresh = element(result.document);
    const current = document.querySelector('.dx-doc');
    if (!fresh || !current) return;
    // The elements a pick pointed at are about to be replaced; the pick goes with them.
    boardPick = null;
    current.replaceWith(fresh);
    dressRunnables(fresh);
    dressChecks(fresh);
    dressBoards(fresh);
    growOutgrownNodes(fresh);
    if (result.focus) void enter(result.focus);
  }

  /** The first element of an HTML string, or null if it holds none. */
  function element(markup) {
    const holder = document.createElement('div');
    holder.innerHTML = markup;
    return holder.firstElementChild;
  }

  /** Run `move`, marking the focus change as intentional so blur does not save under it. */
  function shuttle(move) {
    shuttling = true;
    try {
      move();
    } finally {
      // The blur event this move causes is delivered before the microtask runs.
      Promise.resolve().then(() => { shuttling = false; });
    }
  }

  /**
   * Keyboard. Return, Escape, and Backspace are the body's whole language; the tag line
   * adds the seam between the two fields, the marks add their four chords, and an open
   * menu takes its keys first.
   */
  function onKeyDown(event) {
    // A focused box answers to the two keys every checkbox answers to, whether or not
    // anything is being written — reaching a box by keyboard and not being able to tick it
    // is the affordance stopping halfway.
    if (
      (event.key === ' ' || event.key === 'Enter') &&
      host.check &&
      event.target.matches &&
      event.target.matches('.dx-mark[data-check]')
    ) {
      event.preventDefault();
      toggleCheck(event.target);
      return;
    }

    if (!open) return;
    const { block, header, field } = open;
    const inHeader = event.target === header;
    const inField = event.target === field || (isRich(field) && field.contains(event.target));
    if (!inHeader && !inField) return;

    // Ctrl/Cmd+Space asks for the completions by name.
    if (event.key === ' ' && (event.metaKey || event.ctrlKey)) {
      event.preventDefault();
      suggest(event.target === header ? header : field, true);
      return;
    }

    // The menu speaks first: it is under the caret, and its keys are the point of it.
    if (menu) {
      if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
        event.preventDefault();
        const step = event.key === 'ArrowDown' ? 1 : -1;
        select((menu.selected + step + menu.items.length) % menu.items.length);
        return;
      }
      if (event.key === 'Tab' || event.key === 'Enter') {
        event.preventDefault();
        accept(menu.selected);
        return;
      }
      if (event.key === 'Escape') {
        event.preventDefault();
        dismiss();
        return;
      }
    }

    if (event.key === 'Escape') {
      event.preventDefault();
      close();
      return;
    }

    const finishing = event.key === 'Enter' && (event.metaKey || event.ctrlKey);
    if (finishing) {
      event.preventDefault();
      void save();
      return;
    }

    if (inHeader) {
      headerKey(event);
      return;
    }

    // The rich field's own chords: undo it rebuilds, and the marks it writes.
    if (isRich(field)) {
      const chord = (event.metaKey || event.ctrlKey) && !event.altKey;
      if (chord && event.key.toLowerCase() === 'z') {
        event.preventDefault();
        timeTravel(!event.shiftKey);
        return;
      }
      if (
        !open.block.classList.contains('dx-writes-source') &&
        formatKey(event, field)
      ) {
        event.preventDefault();
        return;
      }
    }

    const at = selection(field);

    // A code block's tag line starts folded away; ArrowUp at the very start brings it
    // back — the retype control is reachable, it just is not worn.
    if (
      !open.headerShown &&
      !open.demoted &&
      kindOf(block) === 'code' &&
      open.header.value !== '' &&
      event.key === 'ArrowUp' &&
      at.start === 0 &&
      at.end === 0
    ) {
      event.preventDefault();
      open.headerShown = true;
      open.header.classList.remove('dx-header-hidden');
      grow(open.header);
      shuttle(() => {
        open.header.focus();
        open.header.setSelectionRange(open.header.value.length, open.header.value.length);
      });
      return;
    }

    // ArrowUp or ArrowLeft at the very start of the body steps up into the tag line —
    // the two fields read as one place to write, so the caret moves like they are.
    if (
      open.headerShown &&
      (event.key === 'ArrowUp' || event.key === 'ArrowLeft') &&
      at.start === 0 &&
      at.end === 0
    ) {
      event.preventDefault();
      shuttle(() => {
        header.focus();
        header.setSelectionRange(header.value.length, header.value.length);
      });
      return;
    }

    // Return ends a paragraph and starts the next one — the thing writing *is*. A listing, a
    // list, and a drawing are the exception: a newline inside them is content, so Return
    // types one and it takes a modifier to finish. The rich field types its own newline,
    // because a contenteditable's native Return invents elements instead of characters.
    const multiline = MULTILINE.includes(effectiveKind());
    if (event.key === 'Enter') {
      if (!multiline && !event.shiftKey) {
        event.preventDefault();
        // One call, not two: the host inserts as part of committing, and a follow-up would
        // be sent about a document that had already moved on.
        void save('insert');
        return;
      }
      if (isRich(field)) {
        event.preventDefault();
        snapshot();
        spliceField(field, at.start, at.end, '\n');
        setSelection(field, at.start + 1, at.start + 1);
        suggest(field, false);
        return;
      }
      return;
    }

    // Backspace in an empty block deletes the block and leaves the reader at the end of the
    // one before it, the way it does in every editor a person has ever used.
    if (event.key === 'Backspace' && fieldText(field) === '' && at.start === 0) {
      event.preventDefault();
      void erase();
    }
  }

  /** The seam keys of the tag line: Enter commits it to the body, arrows step down into it. */
  function headerKey(event) {
    const { header, field } = open;
    const caret = header.selectionStart;
    const atEnd = caret === header.value.length && header.selectionEnd === caret;

    if (event.key === 'Enter') {
      event.preventDefault();
      const text = header.value.trim();
      if (text === '' || !text.startsWith(':')) {
        // Not a tag: the line is prose after all. It goes back to the body — ahead of what
        // is already there — and the block saves as plain text.
        open.headerShown = false;
        open.demoted = true;
        header.classList.add('dx-header-hidden');
        header.value = '';
        if (text !== '') {
          const body = fieldText(field);
          setFieldText(field, body === '' ? text : `${text}\n${body}`);
        }
        dress();
      }
      shuttle(() => {
        field.focus();
        setSelection(field, 0, 0);
      });
      return;
    }

    if ((event.key === 'ArrowDown' || event.key === 'ArrowRight') && atEnd) {
      event.preventDefault();
      shuttle(() => {
        field.focus();
        setSelection(field, 0, 0);
      });
      return;
    }

    // Delete at the end of the tag line eats the body's first character — the same join a
    // Delete performs at the end of any line.
    if (event.key === 'Delete' && atEnd && fieldText(field) !== '') {
      event.preventDefault();
      spliceField(field, 0, 1, '');
    }
  }

  /** Take the open block out, and put the reader in the block above it. */
  function erase() {
    if (!open) return settling;
    const editing = open;
    const { id, block } = editing;
    const previous = block.previousElementSibling;
    const landing = previous ? previous.getAttribute('data-block-id') : null;
    dismiss();
    open = null;
    settling = host
      .remove(id)
      .then((result) => apply(Object.assign({ focus: landing }, result || {})))
      .catch((error) => {
        // The field never left the page, so a refusal only has to hand it back: the block is
        // still being edited, and a refused delete reads where a refused save does.
        report(block, String(error));
        resume(editing);
      });
    return settling;
  }

  // -------------------------------------------------------------------------
  // Running: the one control a block wears, and only the runnable ones.
  // -------------------------------------------------------------------------

  /**
   * Put a `run` control in the label line of every runnable code block under `root`.
   *
   * The renderer marks the blocks (`dx-runnable`), the host executes them; this only adds
   * the word. A host with no `run` gets no controls — a page that offers a button nothing
   * answers is worse than a page that offers none.
   */
  function dressRunnables(root) {
    if (!host || !host.run || !root.querySelectorAll) return;
    for (const summary of root.querySelectorAll('.dx-runnable > summary')) {
      if (summary.querySelector('.dx-run')) continue;
      const control = document.createElement('button');
      control.type = 'button';
      control.className = 'dx-run';
      control.textContent = 'run';
      control.addEventListener('click', (event) => {
        // Not a fold toggle, and not an edit: the click means exactly one thing.
        event.preventDefault();
        event.stopPropagation();
        runBlock(summary.closest('[data-block-id]'), control);
      });
      summary.appendChild(control);
    }
  }

  /**
   * Make every checklist box a box a reader can tick.
   *
   * The renderer already says which item each mark is (`data-check`), so all this adds is
   * the affordance: the mark becomes a `checkbox` the pointer and the keyboard can reach.
   * Like the run control, it is added only when a host can answer for it — a read-only
   * render keeps three characters of ink and no promise it cannot keep.
   *
   * Nothing is drawn, moved, or replaced: the box a reader clicks is the same `[ ]` that was
   * already on the page.
   */
  function dressChecks(root) {
    if (!host || !host.check || !root.querySelectorAll) return;
    for (const mark of root.querySelectorAll('.dx-checklist .dx-mark[data-check]')) {
      if (mark.getAttribute('role') === 'checkbox') continue;
      mark.setAttribute('role', 'checkbox');
      mark.setAttribute('tabindex', '0');
      mark.setAttribute('aria-checked', mark.textContent.includes('x') ? 'true' : 'false');
    }
  }

  /**
   * Tick or untick the box that was clicked, and show the document that comes back.
   *
   * Which box it is comes off the mark the renderer wrote it on, and flipping it is the
   * host's — this file never reads a `[x]`, because what that marker means is the format's
   * business and a second reading of it would eventually read differently.
   */
  function toggleCheck(mark) {
    const block = mark.closest('[data-block-id]');
    const id = block && block.getAttribute('data-block-id');
    const item = Number(mark.getAttribute('data-check'));
    if (!id || !Number.isInteger(item)) return;
    settling = settling
      .then(() => host.check(id, item))
      .then(apply)
      .catch((error) => report(block, String(error)));
  }

  /**
   * Ask the host to run one block, and show the document it hands back.
   *
   * The control says `running…` while the host works, because a run takes as long as the
   * code does. The answer replaces the sheet — the output block the run wrote, or the
   * failure it recorded, arrives as content, the same way every other change does. Only a
   * host that could not run at all is reported as an error.
   */
  function runBlock(block, control) {
    if (!block || control.classList.contains('dx-running')) return;
    const id = block.getAttribute('data-block-id');
    if (!id) return;
    control.classList.add('dx-running');
    control.textContent = 'running…';
    settling = settling
      .then(() => host.run(id))
      .then(apply)
      .catch((error) => {
        if (control.isConnected) {
          control.classList.remove('dx-running');
          control.textContent = 'run';
        }
        report(block, String(error));
      });
  }

  // -------------------------------------------------------------------------
  // The board: a node editor on the sheet. This section runs the pointer — pan, zoom,
  // drag, resize, link — and asks the host to write every line, because the line grammar
  // is the engine's. Blocks inside nodes are edited by the same click everything else is.
  //
  // The geometry is the engine's as well, restated here against boxes a browser can
  // measure. `render::board` decides which side of a node an edge meets, how the edges
  // sharing one side spread along it, and the curve between them — from heights a static
  // render has to guess. Everything below makes the same decisions from the real ones, so
  // a board does not change shape the moment a reader can touch it.
  // -------------------------------------------------------------------------

  /** The four sides of a node, in the letters a node line writes them as. */
  const SIDES = ['l', 'r', 't', 'b'];

  /** Wire every board under `root`: fit its view, dress its nodes, take its pointer. */
  function dressBoards(root) {
    if (!root.querySelectorAll) return;
    const boards = [];
    if (root.classList && root.classList.contains('dx-board')) boards.push(root);
    boards.push(...root.querySelectorAll('.dx-board'));
    for (const board of boards) wireBoard(board);
  }

  /** Attach one board's machinery, once per rendered element. */
  function wireBoard(board) {
    const canvas = board.querySelector('.dx-board-canvas');
    const id = board.getAttribute('data-block-id');
    if (!canvas || !id || board.dataset.dxWired) return;
    board.dataset.dxWired = 'true';

    if (host && host.board) {
      for (const node of board.querySelectorAll('.dx-board-node')) dressNode(node);
    }

    // A board opens fitted — everything on it, scaled on both axes — and stays fitted
    // through every save, until the reader pans or zooms and the view becomes theirs.
    const remembered = boardViews.get(id);
    const view = remembered && !remembered.auto ? remembered : fitView(board, canvas);
    boardViews.set(id, view);
    applyView(canvas, view);
    layoutEdges(board);

    // The column a board sits in changes width — a window resized, a panel opened — and a
    // fit that was right when the page arrived is not right afterwards.
    if (typeof ResizeObserver === 'function') {
      new ResizeObserver(() => refit(board, canvas)).observe(board);
    }

    board.addEventListener('wheel', (event) => onBoardWheel(board, canvas, event), {
      passive: false,
    });
    board.addEventListener('pointerdown', (event) => onBoardPointer(board, canvas, event));
    board.addEventListener('dblclick', (event) => onBoardDouble(board, canvas, event));
    board.addEventListener('contextmenu', (event) => onNodeContextMenu(board, event));
    board.addEventListener('click', (event) => {
      const path = event.target.closest && event.target.closest('.dx-board-edges path');
      if (path) pickEdge(board, path);
    });
  }

  /** Fit a board again, unless the reader has taken its view over. */
  function refit(board, canvas) {
    const id = board.getAttribute('data-block-id');
    const current = boardViews.get(id);
    if (!current || !current.auto) return;
    const fitted = fitView(board, canvas);
    boardViews.set(id, fitted);
    applyView(canvas, fitted);
    layoutEdges(board);
  }

  /**
   * Give a node its editing machinery: the grip bar it is dragged and picked by, a
   * connection strip down each of its four edges, and the corner its width is resized by.
   *
   * A node's whole edge is the connection point, not a dot in one corner of it, so a line
   * can be drawn from anywhere along any side of any node to anywhere along any side of
   * another. All of it is added only when a host can answer for it, so a read-only render
   * carries none of it — the same rule as the run control.
   */
  function dressNode(node) {
    if (node.querySelector('.dx-node-grip')) return;
    const grip = document.createElement('div');
    grip.className = 'dx-node-grip';
    const label = document.createElement('span');
    label.className = 'dx-node-grip-label';
    label.textContent = node.getAttribute('data-node-id') || '';
    grip.appendChild(label);
    node.insertBefore(grip, node.firstChild);

    for (const side of SIDES) {
      const port = document.createElement('span');
      port.className = `dx-node-port dx-node-port-${side}`;
      port.dataset.side = side;
      port.title = 'drag to another node to link';
      node.appendChild(port);
    }

    const size = document.createElement('div');
    size.className = 'dx-node-size';
    size.title = 'drag to set this node’s width';
    node.appendChild(size);
  }

  // The geometry, mirroring `render::board`: boxes, the sides an edge meets, where along a
  // side it sits, and the curve between two of those points.

  /** A node's box on the canvas, in canvas pixels. */
  function boxOf(node) {
    return {
      x: node.offsetLeft,
      y: node.offsetTop,
      w: node.offsetWidth,
      h: node.offsetHeight,
    };
  }

  /** The centre of a box, which is what decides the sides of an unpinned edge. */
  function centreOf(box) {
    return { x: box.x + box.w / 2, y: box.y + box.h / 2 };
  }

  /** Which way an edge's handle reaches out of a side, as a unit vector. */
  function reachOf(side) {
    if (side === 'l') return { dx: -1, dy: 0 };
    if (side === 'r') return { dx: 1, dy: 0 };
    if (side === 't') return { dx: 0, dy: -1 };
    return { dx: 0, dy: 1 };
  }

  /** True for the sides an edge spreads along horizontally — the top and the bottom. */
  function spansAcross(side) {
    return side === 't' || side === 'b';
  }

  /** The point `fraction` of the way along `side` of `box`, and the way it reaches. */
  function anchorOn(box, side, fraction) {
    const reach = reachOf(side);
    const along = {
      l: { x: box.x, y: box.y + box.h * fraction },
      r: { x: box.x + box.w, y: box.y + box.h * fraction },
      t: { x: box.x + box.w * fraction, y: box.y },
      b: { x: box.x + box.w * fraction, y: box.y + box.h },
    }[side];
    return { x: along.x, y: along.y, dx: reach.dx, dy: reach.dy };
  }

  /**
   * The sides an unpinned edge joins: the facing pair on the axis the two boxes are
   * furthest apart along, vertical winning a tie — `render::board::auto_sides`, because a
   * board grows downwards and two boxes that overlap across are one above the other.
   */
  function autoSides(from, to) {
    const a = centreOf(from);
    const b = centreOf(to);
    const dx = b.x - a.x;
    const dy = b.y - a.y;
    if (Math.abs(dy) >= Math.abs(dx)) return dy >= 0 ? ['b', 't'] : ['t', 'b'];
    return dx >= 0 ? ['r', 'l'] : ['l', 'r'];
  }

  /** The side of `box` nearest a canvas point: which edge a line was dropped on. */
  function sideNearest(box, x, y) {
    const gap = {
      l: Math.abs(x - box.x),
      r: Math.abs(box.x + box.w - x),
      t: Math.abs(y - box.y),
      b: Math.abs(box.y + box.h - y),
    };
    return SIDES.reduce((best, side) => (gap[side] < gap[best] ? side : best), 'l');
  }

  // How far a path keeps away from a box it is not attached to, and how finely a curve is
  // sampled when asking. Both `render::board`'s numbers.
  const EDGE_CLEARANCE = 10;
  const EDGE_SAMPLES = 32;

  /** Whether a point is inside `box`, with `slack` of margin — `Rect::holds`. */
  function boxHolds(box, x, y, slack) {
    return (
      x > box.x - slack &&
      x < box.x + box.w + slack &&
      y > box.y - slack &&
      y < box.y + box.h + slack
    );
  }

  /** Whether the straight run between two points passes through `box`. */
  function segmentHits(from, to, box) {
    for (let step = 0; step <= EDGE_SAMPLES; step += 1) {
      const along = step / EDGE_SAMPLES;
      const x = from.x + (to.x - from.x) * along;
      const y = from.y + (to.y - from.y) * along;
      if (boxHolds(box, x, y, EDGE_CLEARANCE)) return true;
    }
    return false;
  }

  /**
   * Where two runs cross, as the sine of the angle they meet at, or `null` if they do not —
   * `render::board::crossing_sine`. Square crossings read as one line passing over another;
   * shallow ones read as a fork, so they are scored differently.
   */
  function crossingSine(a, b) {
    const r = { x: a[1].x - a[0].x, y: a[1].y - a[0].y };
    const s = { x: b[1].x - b[0].x, y: b[1].y - b[0].y };
    const denominator = r.x * s.y - r.y * s.x;
    if (Math.abs(denominator) < 1e-12) return null;
    const dx = b[0].x - a[0].x;
    const dy = b[0].y - a[0].y;
    const along = (dx * s.y - dy * s.x) / denominator;
    const across = (dx * r.y - dy * r.x) / denominator;
    if (along < 0.02 || along > 0.98 || across < 0.02 || across > 0.98) return null;
    const length = Math.max(1e-12, Math.hypot(r.x, r.y) * Math.hypot(s.x, s.y));
    return Math.abs(denominator / length);
  }

  /** The middle of one side of a box — where a candidate route is measured from. */
  function midpointOn(box, side) {
    return anchorOn(box, side, 0.5);
  }

  /**
   * The pair of sides an unpinned edge should join — `render::board::clearest_sides`.
   *
   * A box over a line beats any number of crossings, a shallow crossing costs more than a
   * square one, and the facing pair breaks the tie so a clear run keeps the arrangement a
   * board has always had.
   */
  function clearestSides(fromBox, toBox, obstacles, pins, settled) {
    const OCCLUSION = 100;
    const CROSSING = 4;
    const preferred = autoSides(fromBox, toBox);
    const outs = pins[0] ? [pins[0]] : SIDES;
    const intos = pins[1] ? [pins[1]] : SIDES;

    let best = { cost: Infinity, sides: preferred };
    for (const out of outs) {
      for (const into of intos) {
        const start = midpointOn(fromBox, out);
        const end = midpointOn(toBox, into);
        const blocked = obstacles.filter((box) => segmentHits(start, end, box)).length;
        let tangle = 0;
        for (const other of settled) {
          const square = crossingSine([start, end], other);
          if (square !== null) tangle += CROSSING * (1.25 - square * 0.75);
        }
        const habit = (out !== preferred[0] ? 1 : 0) + (into !== preferred[1] ? 1 : 0);
        const cost = blocked * OCCLUSION + tangle + habit;
        if (cost < best.cost) best = { cost, sides: [out, into] };
      }
    }
    return best.sides;
  }

  /**
   * The two control points of an edge's cubic, bent aside if the plain curve would hide
   * behind a box — `render::board::controls`.
   *
   * The plain curve is tried first and is what almost every edge gets. Each handle reaches
   * a share of the span, but never more than half the distance the run advances along that
   * handle's own normal — handles that overshoot each other across a short gap wobble the
   * cubic into an S-hook. When a box is in the
   * way the handles are pushed square to the line between the ends, in growing steps and
   * both directions — far past the span itself, because a wall of a node can stand taller
   * than the whole run — and the first arrangement that clears every box wins. When nothing
   * clears, the attempt that hid the fewest points of the line wins instead.
   */
  function controlsFor(from, to, obstacles) {
    const STEPS = [0, 0.35, 0.7, 1.1, 1.6, 2.2, 3.0, 4.0, 5.5, 7.5];
    const span = Math.hypot(to.x - from.x, to.y - from.y);
    const reach = Math.min(160, Math.max(30, span * 0.42));
    const capped = (end, other) => {
      const advance = (other.x - end.x) * end.dx + (other.y - end.y) * end.dy;
      return advance > 0 ? Math.min(reach, Math.max(advance * 0.5, 10)) : reach;
    };
    const fromReach = capped(from, to);
    const toReach = capped(to, from);
    const base = [
      { x: from.x + fromReach * from.dx, y: from.y + fromReach * from.dy },
      { x: to.x + toReach * to.dx, y: to.y + toReach * to.dy },
    ];
    if (!obstacles || !obstacles.length) return base;

    const length = Math.max(1e-12, span);
    const square = { x: -(to.y - from.y) / length, y: (to.x - from.x) / length };
    let least = { hidden: Infinity, candidate: base };
    for (const step of STEPS) {
      for (const direction of [1, -1]) {
        const push = step * reach * direction;
        const candidate = [
          { x: base[0].x + square.x * push, y: base[0].y + square.y * push },
          { x: base[1].x + square.x * push, y: base[1].y + square.y * push },
        ];
        const hidden = hiddenSamples(from, to, candidate, obstacles);
        if (!hidden) return candidate;
        if (hidden < least.hidden) least = { hidden, candidate };
        if (step === 0) break;
      }
    }
    return least.candidate;
  }

  /** How many sampled points of a cubic with these controls sit inside an obstacle. */
  function hiddenSamples(from, to, controls, obstacles) {
    let hidden = 0;
    for (let step = 2; step < EDGE_SAMPLES - 1; step += 1) {
      const at = cubicAt(from, to, controls, step / EDGE_SAMPLES);
      if (obstacles.some((box) => boxHolds(box, at.x, at.y, EDGE_CLEARANCE))) hidden += 1;
    }
    return hidden;
  }

  /** A point `along` a cubic, 0 at `from` and 1 at `to`. */
  function cubicAt(from, to, controls, along) {
    const rest = 1 - along;
    const a = rest * rest * rest;
    const b = 3 * rest * rest * along;
    const c = 3 * rest * along * along;
    const d = along * along * along;
    return {
      x: a * from.x + b * controls[0].x + c * controls[1].x + d * to.x,
      y: a * from.y + b * controls[0].y + c * controls[1].y + d * to.y,
    };
  }

  /**
   * The point the curve hands over to the arrowhead — `render::board::lead`, same numbers:
   * up to 16px out from the arrival anchor along its side's own normal. The cubic ends
   * here and a straight segment carries the line the rest of the way in, so the head sits
   * square to the side it arrives on and the line enters it dead down its axis — a bent
   * cubic is still turning as it arrives, and without the tail the line met the head off
   * its centre.
   */
  function leadOf(from, to) {
    const span = Math.hypot(to.x - from.x, to.y - from.y);
    const run = Math.min(16, Math.max(4, span * 0.25));
    return { x: to.x + run * to.dx, y: to.y + run * to.dy, dx: to.dx, dy: to.dy };
  }

  /**
   * The path an edge is drawn as: a cubic from the source to the lead point, then the
   * straight run into the arrowhead — `render::board::curve`, same numbers, so the line a
   * reader drags settles into the line the engine draws. `obstacles` are the boxes it must
   * not disappear behind; a drag in progress passes none.
   */
  function curveBetween(from, to, obstacles) {
    const lead = leadOf(from, to);
    const controls = controlsFor(from, lead, obstacles);
    return (
      `M ${from.x} ${from.y} ` +
      `C ${controls[0].x} ${controls[0].y}, ` +
      `${controls[1].x} ${controls[1].y}, ${lead.x} ${lead.y} ` +
      `L ${to.x} ${to.y}`
    );
  }

  /** Fit the view to the nodes: all of them, on both axes, centred where they fit. */
  function fitView(board, canvas) {
    const nodes = canvas.querySelectorAll('.dx-board-node');
    if (!nodes.length) return { s: 1, tx: BOARD_PADDING, ty: BOARD_PADDING, auto: true };
    const box = contentBox(nodes);
    const vw = board.clientWidth;
    const vh = board.clientHeight;
    const s = Math.min(
      FIT_MAX,
      Math.max(
        ZOOM_MIN,
        Math.min(
          (vw - 2 * BOARD_PADDING) / Math.max(1, box.x1 - box.x0),
          (vh - 2 * BOARD_PADDING) / Math.max(1, box.y1 - box.y0)
        )
      )
    );
    return {
      s,
      tx: (vw - s * (box.x0 + box.x1)) / 2,
      // Centred when it fits, pinned to the top when it does not: a plan is read from its
      // first node down, so the overflow belongs at the bottom.
      ty: Math.min((vh - s * (box.y0 + box.y1)) / 2, BOARD_PADDING - s * box.y0),
      auto: true,
    };
  }

  /** The nodes' bounding box in canvas coordinates. */
  function contentBox(nodes) {
    let x0 = Infinity;
    let y0 = Infinity;
    let x1 = -Infinity;
    let y1 = -Infinity;
    for (const node of nodes) {
      x0 = Math.min(x0, node.offsetLeft);
      y0 = Math.min(y0, node.offsetTop);
      x1 = Math.max(x1, node.offsetLeft + node.offsetWidth);
      y1 = Math.max(y1, node.offsetTop + node.offsetHeight);
    }
    return { x0, y0, x1, y1 };
  }

  /** Paint a view onto its canvas. */
  function applyView(canvas, view) {
    canvas.style.transform = `translate(${view.tx}px, ${view.ty}px) scale(${view.s})`;
  }

  /** Keep some of the content on screen: a pan can push it near the edge, never past it. */
  function clampView(board, canvas, view) {
    const nodes = canvas.querySelectorAll('.dx-board-node');
    if (!nodes.length) return;
    const box = contentBox(nodes);
    view.tx = Math.min(
      Math.max(view.tx, PAN_KEEP - view.s * box.x1),
      board.clientWidth - PAN_KEEP - view.s * box.x0
    );
    view.ty = Math.min(
      Math.max(view.ty, PAN_KEEP - view.s * box.y1),
      board.clientHeight - PAN_KEEP - view.s * box.y0
    );
  }

  /**
   * Pinching — or holding ⌘/Ctrl — zooms the board, toward the cursor.
   *
   * A plain wheel is left alone, because a board lives *in a page*: a reader scrolling past
   * one must not have the document stop under their fingers and the canvas zoom instead.
   * A trackpad pinch arrives as a wheel event with `ctrlKey`, which is the browser's own
   * signal for "this gesture means scale", so the two read identically.
   */
  function onBoardWheel(board, canvas, event) {
    if (!event.ctrlKey && !event.metaKey) return;
    event.preventDefault();
    const id = board.getAttribute('data-block-id');
    const view = boardViews.get(id);
    if (!view) return;
    const rect = board.getBoundingClientRect();
    const px = event.clientX - rect.left;
    const py = event.clientY - rect.top;
    const factor = Math.exp(-event.deltaY * 0.0022);
    const scaled = Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, view.s * factor));
    // The canvas point under the cursor stays under the cursor.
    view.tx = px - ((px - view.tx) / view.s) * scaled;
    view.ty = py - ((py - view.ty) / view.s) * scaled;
    view.s = scaled;
    // The view is the reader's now: nothing re-fits it under them.
    view.auto = false;
    clampView(board, canvas, view);
    applyView(canvas, view);
  }

  /**
   * The pointer, sorted by what it landed on: a grip drags its node, an edge strip draws a
   * connection, the corner handle resizes, and empty canvas pans. A press inside a node's
   * content is not the board's — it is the click that edits the block, and it passes
   * untouched.
   */
  function onBoardPointer(board, canvas, event) {
    if (event.button !== 0) return;
    const id = board.getAttribute('data-block-id');
    const view = boardViews.get(id);
    if (!view) return;

    const port = event.target.closest && event.target.closest('.dx-node-port');
    const grip = event.target.closest && event.target.closest('.dx-node-grip');
    const size = event.target.closest && event.target.closest('.dx-node-size');
    const withinNode = event.target.closest && event.target.closest('.dx-board-node');

    if (port && host && host.board) {
      dragEdge(board, canvas, view, withinNode, port, event);
    } else if (grip && host && host.board) {
      dragNode(board, canvas, view, withinNode, event);
    } else if (size && host && host.board) {
      dragSize(board, canvas, view, withinNode, event);
    } else if (!withinNode) {
      dragPan(board, canvas, view, event);
    }
  }

  /** Track one pointer from `event` until it lifts: `moved(dx, dy)`, then `done(went)`. */
  function track(event, moved, done) {
    const fromX = event.clientX;
    const fromY = event.clientY;
    let went = false;
    const onMove = (step) => {
      const dx = step.clientX - fromX;
      const dy = step.clientY - fromY;
      if (!went && Math.abs(dx) < 3 && Math.abs(dy) < 3) return;
      went = true;
      moved(dx, dy, step);
    };
    const onUp = () => {
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
      done(went);
    };
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
    event.preventDefault();
  }

  /** Drag empty paper: the view moves, nothing is written. */
  function dragPan(board, canvas, view, event) {
    const fromX = view.tx;
    const fromY = view.ty;
    track(
      event,
      (dx, dy) => {
        view.tx = fromX + dx;
        view.ty = fromY + dy;
        view.auto = false;
        clampView(board, canvas, view);
        applyView(canvas, view);
      },
      () => {}
    );
  }

  /** Drag a grip: the node follows, the edges follow it, and the drop settles the board. */
  function dragNode(board, canvas, view, node, event) {
    const fromX = node.offsetLeft;
    const fromY = node.offsetTop;
    track(
      event,
      (dx, dy) => {
        node.style.left = `${Math.round(fromX + dx / view.s)}px`;
        node.style.top = `${Math.round(fromY + dy / view.s)}px`;
        layoutEdges(board);
      },
      (went) => {
        if (!went) {
          pickNode(board, node);
          return;
        }
        arrange(board, [node]);
      }
    );
  }

  /**
   * Drag the corner: the node is reshaped, both ways, and the drop writes its box.
   *
   * A node's box is what its line says — `w` and `h` together — so reshaping is one gesture
   * and one number pair, and the frame a reader drags is the frame every other surface,
   * every PNG, and every agent will see.
   */
  function dragSize(board, canvas, view, node, event) {
    const fromW = node.offsetWidth;
    const fromH = node.offsetHeight;
    track(
      event,
      (dx, dy) => {
        node.style.width = `${Math.max(NODE_MIN_WIDTH, Math.round(fromW + dx / view.s))}px`;
        node.style.height = `${Math.max(NODE_MIN_HEIGHT, Math.round(fromH + dy / view.s))}px`;
        layoutEdges(board);
      },
      (went) => {
        if (!went) return;
        arrange(board, [node]);
      }
    );
  }

  /**
   * Double-clicking the corner fits the node to what is in it — the one measurement a
   * reader should never have to make by dragging.
   */
  function fitNodeHeight(board, node) {
    const height = contentHeight(node);
    if (!height || Math.abs(height - node.offsetHeight) < 2) return;
    node.style.height = `${height}px`;
    layoutEdges(board);
    arrange(board, [node]);
  }

  /** How tall this node would have to be to show all of its block, in canvas pixels. */
  function contentHeight(node) {
    const body = node.querySelector('.dx-board-node-body');
    if (!body) return 0;
    return Math.round(body.scrollHeight + (node.offsetHeight - body.clientHeight));
  }

  /**
   * After an edit, grow every node whose block no longer fits inside it.
   *
   * Writing into a node makes its content taller, and a frame that quietly clips the
   * sentence someone just typed is the worst thing a board could do. Growing only: a reader
   * who made a node deliberately small keeps it, and a node that already fits is left alone —
   * which is also what stops this from being a second edit every time it runs.
   */
  function growOutgrownNodes(root) {
    if (!host || !host.board || !root.querySelectorAll) return;
    for (const board of root.querySelectorAll('.dx-board')) {
      const outgrown = [];
      for (const node of board.querySelectorAll('.dx-board-node')) {
        const height = contentHeight(node);
        if (height > node.offsetHeight + 2) {
          node.style.height = `${height}px`;
          outgrown.push(node);
        }
      }
      if (!outgrown.length) continue;
      layoutEdges(board);
      arrange(board, outgrown);
    }
  }

  /**
   * Write these nodes' boxes: where they are and how big they are, in one call.
   *
   * The surface reports geometry and nothing else. Pushing whatever they landed on out of
   * the way is the *engine's* — a node's box is stated on its line, so `edit::board_arrange`
   * knows every rectangle without measuring anything, and a reader dragging and an agent
   * running `dx board` settle a board by the identical rule.
   */
  function arrange(board, nodes) {
    if (!nodes.length) return;
    boardCall(board, 'arrange', {
      arrangement: nodes
        .map((node) =>
          [
            node.getAttribute('data-node-id'),
            node.offsetLeft,
            node.offsetTop,
            node.offsetWidth,
            node.offsetHeight,
          ].join(',')
        )
        .join(' '),
    });
  }

  /**
   * Drag from a node's edge: a curve follows the pointer, snaps to the side of whatever
   * node it is over, and the drop writes the line between those two sides.
   *
   * Both ends are the reader's choice, and both are kept (`to=target:b-t`), so a
   * connection stays on the edges it was drawn between however the board is rearranged.
   */
  function dragEdge(board, canvas, view, node, port, event) {
    const svg = edgesSvg(canvas);
    const probe = document.createElementNS('http://www.w3.org/2000/svg', 'path');
    probe.setAttribute('class', 'dx-linking');
    svg.appendChild(probe);
    const fromSide = port.dataset.side || 'r';
    const from = anchorOn(boxOf(node), fromSide, 0.5);
    let target = null;
    let targetSide = '';
    track(
      event,
      (dx, dy, step) => {
        const rect = canvas.getBoundingClientRect();
        const x = (step.clientX - rect.left) / view.s;
        const y = (step.clientY - rect.top) / view.s;
        const over = nodeUnder(step.clientX, step.clientY);
        if (target && target !== over) target.classList.remove('dx-link-target');
        target = over && over !== node && canvas.contains(over) ? over : null;
        let to = { x, y, dx: -from.dx, dy: -from.dy };
        if (target) {
          target.classList.add('dx-link-target');
          const box = boxOf(target);
          targetSide = sideNearest(box, x, y);
          to = anchorOn(box, targetSide, 0.5);
        } else {
          targetSide = '';
        }
        probe.setAttribute('d', curveBetween(from, to));
      },
      (went) => {
        probe.remove();
        if (target) target.classList.remove('dx-link-target');
        if (!went || !target) return;
        boardCall(board, 'link', {
          from: node.getAttribute('data-node-id'),
          to: target.getAttribute('data-node-id'),
          fromSide,
          toSide: targetSide,
        });
      }
    );
  }

  /** The board node under a point on the screen, if the pointer is over one. */
  function nodeUnder(clientX, clientY) {
    const under = document.elementFromPoint(clientX, clientY);
    return (under && under.closest && under.closest('.dx-board-node')) || null;
  }

  /** Double-clicking empty paper starts a new node there, open and ready to write. */
  function onBoardDouble(board, canvas, event) {
    if (!host || !host.board) return;
    // The corner's own double-click fits the node to its content; everywhere else inside a
    // node belongs to the block being written in.
    const size = event.target.closest && event.target.closest('.dx-node-size');
    if (size) {
      event.preventDefault();
      fitNodeHeight(board, size.closest('.dx-board-node'));
      return;
    }
    if (event.target.closest && event.target.closest('.dx-board-node')) return;
    const id = board.getAttribute('data-block-id');
    const view = boardViews.get(id);
    if (!view) return;
    event.preventDefault();
    const rect = canvas.getBoundingClientRect();
    boardCall(board, 'add', {
      x: Math.round((event.clientX - rect.left) / view.s),
      y: Math.round((event.clientY - rect.top) / view.s),
    });
  }

  /**
   * Right-click a node for its exact box, typed rather than dragged — the same `x y w h` a
   * node's own line carries, for a placement no drag can hit by eye. Left alone while the
   * node's own block is being written in, so the browser's native menu still serves copy
   * and paste there.
   */
  function onNodeContextMenu(board, event) {
    if (!host || !host.board) return;
    const node = event.target.closest && event.target.closest('.dx-board-node');
    if (!node) return;
    if (open && open.block && node.contains(open.block) && open.block.contains(event.target)) {
      return;
    }
    event.preventDefault();
    openSizeMenu(board, node, event);
  }

  /** Open the box editor at the pointer: `set` writes the typed numbers the way a drop would. */
  function openSizeMenu(board, node, event) {
    closeSizeMenu();
    const holder = document.createElement('div');
    holder.className = 'dx-menu dx-size-menu';

    const fields = {};
    const field = (key, label, value) => {
      const row = document.createElement('label');
      row.className = 'dx-size-field';
      const caption = document.createElement('span');
      caption.textContent = label;
      const input = document.createElement('input');
      input.type = 'number';
      input.step = '1';
      input.value = String(Math.round(value));
      row.appendChild(caption);
      row.appendChild(input);
      holder.appendChild(row);
      fields[key] = input;
    };
    field('x', 'x', node.offsetLeft);
    field('y', 'y', node.offsetTop);
    field('w', 'w', node.offsetWidth);
    field('h', 'h', node.offsetHeight);

    const apply = document.createElement('button');
    apply.type = 'button';
    apply.className = 'dx-menu-item dx-size-apply';
    apply.textContent = 'set';
    holder.appendChild(apply);

    const commit = () => {
      const x = Math.round(Number(fields.x.value));
      const y = Math.round(Number(fields.y.value));
      const w = Math.round(Number(fields.w.value));
      const h = Math.round(Number(fields.h.value));
      if (![x, y, w, h].every(Number.isFinite)) return;
      node.style.left = `${x}px`;
      node.style.top = `${y}px`;
      node.style.width = `${Math.max(NODE_MIN_WIDTH, w)}px`;
      node.style.height = `${Math.max(NODE_MIN_HEIGHT, h)}px`;
      layoutEdges(board);
      closeSizeMenu();
      arrange(board, [node]);
    };
    apply.addEventListener('mousedown', (down) => {
      down.preventDefault();
      commit();
    });
    holder.addEventListener('keydown', (key) => {
      if (key.key === 'Enter') {
        key.preventDefault();
        commit();
      } else if (key.key === 'Escape') {
        key.preventDefault();
        closeSizeMenu();
      }
      key.stopPropagation();
    });

    document.body.appendChild(holder);
    holder.style.position = 'fixed';
    holder.style.left = `${Math.min(event.clientX, window.innerWidth - holder.offsetWidth - 8)}px`;
    holder.style.top = `${Math.min(event.clientY, window.innerHeight - holder.offsetHeight - 8)}px`;

    sizeMenu = { holder, board, node };
    fields.x.focus();
    fields.x.select();
    document.addEventListener('mousedown', onSizeMenuOutside, true);
  }

  /** A press outside the box editor closes it without writing anything. */
  function onSizeMenuOutside(event) {
    if (sizeMenu && !sizeMenu.holder.contains(event.target)) closeSizeMenu();
  }

  /** Take the box editor off the page. */
  function closeSizeMenu() {
    if (!sizeMenu) return;
    sizeMenu.holder.remove();
    sizeMenu = null;
    document.removeEventListener('mousedown', onSizeMenuOutside, true);
  }

  /** Pick a node: the accent hairline says which one Delete would take off the board. */
  function pickNode(board, node) {
    clearPick();
    node.classList.add('dx-picked');
    boardPick = {
      board: board.getAttribute('data-block-id'),
      node: node.getAttribute('data-node-id'),
    };
  }

  /** Pick an edge, the same way. */
  function pickEdge(board, path) {
    clearPick();
    path.classList.add('dx-picked');
    boardPick = {
      board: board.getAttribute('data-block-id'),
      from: path.getAttribute('data-from'),
      to: path.getAttribute('data-to'),
    };
  }

  /** Let go of whatever was picked. */
  function clearPick() {
    for (const marked of document.querySelectorAll('.dx-picked')) {
      marked.classList.remove('dx-picked');
    }
    boardPick = null;
  }

  /**
   * Delete takes the picked node (or edge) off the board; Escape just lets go of it.
   *
   * Only when nothing is being written: a Backspace inside a field is a Backspace.
   */
  function onBoardKey(event) {
    if (!boardPick || open || !host || !host.board) return;
    const active = document.activeElement;
    if (
      active &&
      (active.tagName === 'TEXTAREA' || active.tagName === 'INPUT' || active.isContentEditable)
    ) {
      return;
    }
    if (event.key === 'Escape') {
      clearPick();
      return;
    }
    if (event.key !== 'Delete' && event.key !== 'Backspace') return;
    event.preventDefault();
    const board = document.querySelector(
      `.dx-board[data-block-id="${cssEscape(boardPick.board)}"]`
    );
    if (!board) {
      clearPick();
      return;
    }
    if (boardPick.node) {
      boardCall(board, 'detach', { node: boardPick.node });
    } else {
      boardCall(board, 'unlink', { from: boardPick.from, to: boardPick.to });
    }
  }

  /** One operation to the host, and the re-rendered sheet back onto the page. */
  function boardCall(board, action, spec) {
    const id = board.getAttribute('data-block-id');
    settling = settling
      .then(() => host.board(id, action, spec))
      .then(apply)
      .catch((error) => report(board, String(error)));
    return settling;
  }

  /**
   * Redraw every edge against the measured boxes: which side of each node it meets, where
   * along that side it sits, and the curve between the two.
   *
   * The engine draws the same edges from guessed heights — it cannot measure content — and
   * this replaces them with the truth. A *pinned* end (`data-from-side`, written only when
   * the node line pins one) stays on the edge the reader drew it from; an unpinned end
   * takes the facing side, so an edge nobody placed re-routes as the board is rearranged.
   *
   * A side is a whole connection edge, not a point: the lines meeting one take an even
   * share of its length, ordered by where their other ends are, so a node with four
   * connections fans them out instead of stacking them on one spot and crossing them on
   * the way in.
   */
  function layoutEdges(board) {
    const canvas = board.querySelector('.dx-board-canvas');
    if (!canvas) return;
    const svg = canvas.querySelector('.dx-board-edges');
    if (!svg) return;
    // The sheet the edges are drawn on grows to cover the nodes, so a path stays
    // clickable — a browser only promises hit-testing inside the element's own box.
    const nodes = canvas.querySelectorAll('.dx-board-node');
    if (nodes.length) {
      const box = contentBox(nodes);
      svg.setAttribute('width', String(Math.max(1, box.x1 + 200)));
      svg.setAttribute('height', String(Math.max(1, box.y1 + 200)));
    }

    // Every box on the canvas, so an edge can be routed clear of the ones it does not join.
    const obstacles = new Map();
    for (const node of nodes) {
      obstacles.set(node.getAttribute('data-node-id'), boxOf(node));
    }

    const drawn = [];
    const settled = [];
    for (const path of svg.querySelectorAll('path[data-from]')) {
      const fromId = path.getAttribute('data-from');
      const toId = path.getAttribute('data-to');
      const from = nodeOn(canvas, fromId);
      const to = nodeOn(canvas, toId);
      if (!from || !to || from === to) continue;
      const boxes = [boxOf(from), boxOf(to)];
      const between = [...obstacles.entries()]
        .filter(([id]) => id !== fromId && id !== toId)
        .map(([, box]) => box);
      const sides = clearestSides(
        boxes[0],
        boxes[1],
        between,
        [path.getAttribute('data-from-side'), path.getAttribute('data-to-side')],
        settled
      );
      settled.push([midpointOn(boxes[0], sides[0]), midpointOn(boxes[1], sides[1])]);
      drawn.push({ path, boxes, between, ids: [fromId, toId], sides });
    }

    const shared = new Map();
    drawn.forEach((edge, index) => {
      for (const end of [0, 1]) {
        const key = `${edge.ids[end]}|${edge.sides[end]}`;
        const other = centreOf(edge.boxes[1 - end]);
        if (!shared.has(key)) shared.set(key, []);
        shared.get(key).push({
          index,
          end,
          along: spansAcross(edge.sides[end]) ? other.x : other.y,
        });
      }
    });

    const ends = drawn.map(() => [null, null]);
    for (const bucket of shared.values()) {
      bucket.sort((a, b) => a.along - b.along || a.index - b.index);
      bucket.forEach((slot, position) => {
        const edge = drawn[slot.index];
        ends[slot.index][slot.end] = anchorOn(
          edge.boxes[slot.end],
          edge.sides[slot.end],
          (position + 1) / (bucket.length + 1)
        );
      });
    }
    drawn.forEach((edge, index) => {
      const [from, to] = ends[index];
      edge.path.setAttribute('d', curveBetween(from, to, edge.between));
      // A label rides the middle of the curve it belongs to. Only labelled edges carry one,
      // so it is found by the pair it names rather than by counting.
      const label = svg.querySelector(
        `.dx-board-edge-label[data-from="${cssEscape(edge.ids[0])}"]` +
          `[data-to="${cssEscape(edge.ids[1])}"]`
      );
      if (label) {
        const lead = leadOf(from, to);
        const middle = cubicAt(from, lead, controlsFor(from, lead, edge.between), 0.5);
        label.setAttribute('x', String(middle.x));
        label.setAttribute('y', String(middle.y));
      }
    });
  }

  /** The node element showing `id` on this canvas, if it is there. */
  function nodeOn(canvas, id) {
    return id ? canvas.querySelector(`.dx-board-node[data-node-id="${cssEscape(id)}"]`) : null;
  }

  /** This canvas's edge sheet, created when the board has no edges drawn yet. */
  function edgesSvg(canvas) {
    let svg = canvas.querySelector('.dx-board-edges');
    if (!svg) {
      svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
      svg.setAttribute('class', 'dx-board-edges');
      svg.setAttribute('width', '1');
      svg.setAttribute('height', '1');
      canvas.insertBefore(svg, canvas.firstChild);
    }
    return svg;
  }

  // -------------------------------------------------------------------------
  // Completion: ghost text at the caret, and a small menu of what could stand there.
  // -------------------------------------------------------------------------

  /** What the reader has accepted before, newest last. */
  function history() {
    try {
      const raw = window.localStorage.getItem(HISTORY_KEY);
      const parsed = raw ? JSON.parse(raw) : null;
      if (parsed && typeof parsed === 'object') {
        return {
          kinds: Array.isArray(parsed.kinds) ? parsed.kinds : [],
          keys: Array.isArray(parsed.keys) ? parsed.keys : [],
          values: parsed.values && typeof parsed.values === 'object' ? parsed.values : {},
        };
      }
    } catch (ignored) {
      // A blocked or corrupt store means no memory, not no completions.
    }
    return { kinds: [], keys: [], values: {} };
  }

  /** Remember an accepted completion, so the vocabulary a writer uses ranks next time. */
  function remember(kind, key, value) {
    const learned = history();
    const push = (list, entry) => {
      if (!entry) return list;
      const kept = list.filter((it) => it !== entry);
      kept.push(entry);
      return kept.slice(-HISTORY_LIMIT);
    };
    learned.kinds = push(learned.kinds, kind);
    learned.keys = push(learned.keys, key);
    if (key && value) learned.values[key] = push(learned.values[key] || [], value);
    try {
      window.localStorage.setItem(HISTORY_KEY, JSON.stringify(learned));
    } catch (ignored) {
      // Remembering is a courtesy; a full or blocked store must not break typing.
    }
  }

  /** Candidates for `typed`, keeping only ones it prefixes, `typed` itself excluded. */
  function matching(candidates, typed) {
    const lower = typed.toLowerCase();
    const seen = new Set();
    const kept = [];
    for (const candidate of candidates) {
      const flat = candidate.toLowerCase();
      if (!flat.startsWith(lower) || flat === lower || seen.has(flat)) continue;
      seen.add(flat);
      kept.push(candidate);
    }
    return kept;
  }

  /**
   * What could complete at this field's caret: `{ items, start }` or null.
   *
   * Three contexts, the same ones the format has: a `::` word is a block kind; a bare word
   * on a line that opened with `::kind` is one of that kind's attributes; a word after
   * `key=` is a value for that key. Anything else completes to nothing.
   */
  function completions(field, force) {
    const at = selection(field);
    if (at.start !== at.end) return null;
    const caret = at.start;
    const value = fieldText(field);
    const lineStart = value.lastIndexOf('\n', caret - 1) + 1;
    const line = value.slice(lineStart, caret);
    const learned = history();

    const tag = line.match(/(::[a-z-]*)$/i);
    if (tag) {
      const typed = tag[1];
      const ranked = matching(KINDS.concat(learned.kinds), typed);
      if (!ranked.length && !force) return null;
      return {
        start: caret - typed.length,
        items: ranked.map((insert) => ({ insert, label: insert, detail: 'block' })),
      };
    }

    const opened = line.match(/^\s*::([a-z-]+)(\s|$)/i);
    if (opened) {
      const kind = opened[1].toLowerCase();

      const pair = line.match(/([a-z-]+)=([^\s]*)$/i);
      if (pair) {
        const key = pair[1].toLowerCase();
        const typed = pair[2];
        const pool = (VALUES[key] || []).concat(learned.values[key] || []);
        const ranked = matching(pool, typed);
        if (!ranked.length) return null;
        return {
          start: caret - typed.length,
          items: ranked.map((insert) => ({ insert, label: insert, detail: `${key} value` })),
        };
      }

      const word = line.match(/([a-z-]*)$/i);
      const typed = word ? word[1] : '';
      if (typed === '' && !force && !/\s$/.test(line)) return null;
      const pool = (ATTRS[kind] || []).concat(['id', 'class', 'hidden'], learned.keys);
      const ranked = matching(pool, typed);
      if (!ranked.length) return null;
      return {
        start: caret - typed.length,
        items: ranked.map((key) => ({
          insert: BARE_ATTRS.includes(key) ? key : `${key}=`,
          label: BARE_ATTRS.includes(key) ? key : `${key}=`,
          detail: `${kind} attribute`,
        })),
      };
    }

    // Asked by name and nothing narrower matched: the block kinds, ready to write.
    if (force) {
      return {
        start: caret,
        items: KINDS.map((insert) => ({ insert, label: insert, detail: 'block' })),
      };
    }
    return null;
  }

  /** Show (or refresh) the completion menu and ghost for `field`'s caret. */
  function suggest(field, force) {
    if (!open) return;
    const found = completions(field, force);
    if (!found || !found.items.length) {
      dismiss();
      return;
    }

    dismiss();
    const holder = document.createElement('div');
    holder.className = 'dx-menu';
    const ghost = document.createElement('div');
    ghost.className = isRich(field)
      ? 'dx-ghost dx-ghost-rich'
      : 'dx-ghost' + (field === open.header ? ' dx-ghost-header' : '');
    menu = { holder, ghost, field, items: found.items, selected: 0, start: found.start };

    found.items.slice(0, 20).forEach((item, index) => {
      const button = document.createElement('button');
      button.type = 'button';
      button.className = 'dx-menu-item';
      const label = document.createElement('span');
      label.className = 'dx-menu-label';
      label.textContent = item.label;
      const detail = document.createElement('span');
      detail.className = 'dx-menu-detail';
      detail.textContent = item.detail;
      button.appendChild(label);
      button.appendChild(detail);
      // Mousedown, not click: the caret must stay in the field the completion lands in.
      button.addEventListener('mousedown', (event) => {
        event.preventDefault();
        accept(index);
      });
      holder.appendChild(button);
    });

    open.block.appendChild(ghost);
    open.block.appendChild(holder);
    select(0);
    place();
  }

  /** Highlight item `index` and redraw the ghost text to match it. */
  function select(index) {
    if (!menu) return;
    menu.selected = index;
    const buttons = menu.holder.querySelectorAll('.dx-menu-item');
    buttons.forEach((button, at) => button.classList.toggle('dx-menu-selected', at === index));
    if (buttons[index]) buttons[index].scrollIntoView({ block: 'nearest' });

    // Ghost text: the rest of the selected completion, written where it would land, in the
    // faint ink — the typed characters keep their own. Drawn only at the end of a line:
    // mid-line it would sit on top of the characters after the caret.
    const { field, ghost, start, items } = menu;
    const caret = selection(field).start;
    const value = fieldText(field);
    const typed = value.slice(start, caret);
    const insert = items[index].insert;
    const tail = value.slice(caret);
    const atLineEnd = tail === '' || tail.startsWith('\n');
    const fits =
      atLineEnd &&
      insert.toLowerCase().startsWith(typed.toLowerCase()) &&
      insert.length > typed.length;

    if (!fits) {
      ghost.style.display = 'none';
      return;
    }
    const rest = insert.slice(typed.length);
    if (isRich(field)) {
      // The rich field draws its own characters, so the ghost is one floating word at the
      // caret's own rectangle — nothing is mirrored, nothing can misalign. Screen pixels
      // become the block's own through its zoom, which is 1 everywhere but on a board.
      ghost.textContent = rest;
      const point = caretPoint(field);
      const home = open.block.getBoundingClientRect();
      const zoom = zoomOf(open.block);
      ghost.style.left = `${(point.left - home.left) / zoom}px`;
      ghost.style.top = `${(point.top - home.top) / zoom}px`;
      ghost.style.display = 'block';
    } else {
      // A textarea's characters cannot be measured in place, so the ghost mirrors the
      // whole text — the mirrored copy transparent, the remainder in the faint ink —
      // which keeps the remainder exactly where the caret's line ends, wraps and all.
      ghost.textContent = '';
      const before = document.createElement('span');
      before.className = 'dx-ghost-mirror';
      before.textContent = value.slice(0, caret);
      const shown = document.createElement('span');
      shown.className = 'dx-ghost-rest';
      shown.textContent = rest;
      const after = document.createElement('span');
      after.className = 'dx-ghost-mirror';
      after.textContent = tail;
      ghost.append(before, shown, after);
      ghost.style.top = `${field.offsetTop}px`;
      ghost.style.left = `${field.offsetLeft}px`;
      ghost.style.width = `${field.offsetWidth}px`;
      ghost.style.height = `${field.offsetHeight}px`;
      ghost.style.display = 'block';
    }
  }

  /** The caret's viewport rectangle in `field`, by the cheapest honest means each kind has. */
  function caretPoint(field) {
    if (isRich(field)) {
      const picked = window.getSelection();
      if (picked && picked.rangeCount > 0 && field.contains(picked.anchorNode)) {
        const range = picked.getRangeAt(0).cloneRange();
        range.collapse(false);
        const rects = range.getClientRects();
        const rect = rects.length
          ? rects[rects.length - 1]
          : range.getBoundingClientRect();
        if (rect && (rect.top !== 0 || rect.left !== 0 || rect.height !== 0)) {
          return { left: rect.left, top: rect.top, bottom: rect.bottom };
        }
      }
      const rect = field.getBoundingClientRect();
      return { left: rect.left, top: rect.top, bottom: rect.bottom };
    }

    const style = getComputedStyle(field);
    const lineHeight = parseFloat(style.lineHeight) || 16;
    const caret = field.selectionStart;
    const value = field.value;
    const lineStart = value.lastIndexOf('\n', caret - 1) + 1;
    const lines = value.slice(0, lineStart).split('\n').length - 1;
    const context =
      caretPoint.context || (caretPoint.context = document.createElement('canvas').getContext('2d'));
    context.font = `${style.fontWeight} ${style.fontSize} ${style.fontFamily}`;
    const width = context.measureText(value.slice(lineStart, caret)).width;
    const rect = field.getBoundingClientRect();
    return {
      left: rect.left + width,
      top: rect.top + lines * lineHeight,
      bottom: rect.top + (lines + 1) * lineHeight,
    };
  }

  /**
   * Put the menu under the caret's line — or above it, when the page below has run out.
   *
   * The card must never leave the sheet's width or slide under the viewport's edge: a menu
   * the reader has to scroll to is a menu that closed the moment they tried.
   */
  function place() {
    if (!menu) return;
    const { field, holder } = menu;
    const point = caretPoint(field);
    const home = open.block.getBoundingClientRect();
    // The caret rectangle is screen pixels; the card is positioned in the block's own.
    // On a zoomed board the two differ by the canvas scale.
    const zoom = zoomOf(open.block);

    const widest = Math.max(0, open.block.clientWidth - holder.offsetWidth);
    holder.style.left = `${Math.min(Math.max(0, (point.left - home.left) / zoom), widest)}px`;

    const below = (point.bottom - home.top) / zoom + 4;
    const room = window.innerHeight - point.bottom;
    if (
      holder.offsetHeight + 12 > room &&
      (point.top - home.top) / zoom > holder.offsetHeight + 8
    ) {
      holder.style.top = `${(point.top - home.top) / zoom - holder.offsetHeight - 4}px`;
    } else {
      holder.style.top = `${below}px`;
    }
  }

  /**
   * How many screen pixels one of `element`'s own pixels paints as.
   *
   * 1 everywhere except inside a board's scaled canvas, which is the one place on the
   * sheet a CSS transform sits between local coordinates and the reader's screen.
   */
  function zoomOf(element) {
    return element.offsetWidth
      ? element.getBoundingClientRect().width / element.offsetWidth
      : 1;
  }

  /** Accept item `index`: write it at the caret and remember it. */
  function accept(index) {
    if (!menu) return;
    const { field, start, items } = menu;
    const item = items[index];
    const caret = selection(field).start;
    if (isRich(field)) snapshot();
    spliceField(field, start, caret, item.insert);
    const landing = start + item.insert.length;
    setSelection(field, landing, landing);
    dismiss();

    if (item.detail === 'block') {
      remember(item.insert, null, null);
    } else if (item.detail.endsWith('attribute')) {
      remember(null, item.insert.replace(/=$/, ''), null);
    } else if (item.detail.endsWith('value')) {
      remember(null, item.detail.replace(/ value$/, ''), item.insert);
    }
    field.focus();
    setSelection(field, landing, landing);
    suggest(field, false);
  }

  /** Take the menu and the ghost off the page. */
  function dismiss() {
    if (!menu) return;
    menu.ghost.remove();
    menu.holder.remove();
    menu = null;
  }

  /** Escape an id for use in a selector, on hosts too old to have `CSS.escape`. */
  function cssEscape(value) {
    if (window.CSS && typeof window.CSS.escape === 'function') return window.CSS.escape(value);
    return String(value).replace(/["\\\]]/g, '\\$&');
  }

  /** Say what went wrong where the reader is looking, rather than in a console they are not. */
  function report(block, message) {
    const note = document.createElement('div');
    note.className = 'dx-edit-error';
    note.textContent = message;
    const sheet = document.querySelector('.dx-doc');
    if (block && block.isConnected) block.after(note);
    else if (sheet) sheet.appendChild(note);
    else return;
    setTimeout(() => note.remove(), 6000);
  }

  window.dxEditor = { attach };
})();
