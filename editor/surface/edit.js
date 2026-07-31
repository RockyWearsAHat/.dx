/**
 * Writing on a rendered page — the one implementation, for every surface that has a reader
 * in front of it.
 *
 * DX.app and the VS Code webview both load this file. Neither one has its own version of
 * "what clicking a paragraph does", because two versions of that is two products: a document
 * would behave one way on a Mac and another in an editor, and the difference would be
 * discovered by a person who had already lost a sentence to it.
 *
 * # The page never stops being the page
 * Two rules, and everything here is one of them.
 *
 * **The block keeps rendering while it is written.** What a reader types is `.dx` source,
 * and source is not always what it says: `**strong**`, a list's lines, a drawing's markup.
 * So the rendered block stays exactly where it was on the sheet and follows the field as the
 * characters change — the engine draws it, the way it will be read. The one case that needs
 * no such thing is a block that renders to the characters it is written in, which is most
 * prose: there the field simply *is* the block, set in the block's own type, and the page
 * does not move at all when a reader starts writing.
 *
 * **Saving replaces content, never the page.** A commit hands back the re-rendered document
 * and one element is swapped for it. Nothing navigates, so nothing flashes, the scroll does
 * not move, and there is no position to restore afterwards — the reader is still looking at
 * the place they were looking at.
 *
 * # What it does not do
 * It does not parse `.dx`, render it, or decide what a block is. It knows how to run a caret
 * and a keyboard. Everything else is asked of the host, which answers through `doc-core` —
 * the same operations an agent calls (`dx source`, `dx set`, `dx insert`, `dx remove`,
 * `dx render --block`), so a person typing on the page and an agent editing the file are
 * doing the identical thing.
 *
 * # The host contract
 * `attach(host)` where host provides, each returning a promise:
 *
 * - `source(id)` — the exact characters of one block, to put in the field
 * - `draw(id, text)` — that block's HTML as the page would carry it, if its body were
 *   `text`. Saves nothing. A host that does not provide this gets a field and no live
 *   rendering, which is the old behavior and still correct.
 * - `commit(id, text, then)` — save one block and hand back the document. `then` is
 *   `'insert'` when the reader pressed Return, which asks for the next paragraph in the
 *   *same* call, because a second round trip would race the first one's re-render.
 * - `remove(id)` — take a block out, and hand back the document
 *
 * `commit` and `remove` resolve with `{ document, focus }`: `document` is the re-rendered
 * `.dx-doc` container, and `focus` is the id to open afterwards, if any.
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
  const MULTILINE = ['code', 'bulleted-list', 'numbered-list', 'checklist', 'html', 'svg', 'mermaid'];

  /**
   * How long to wait after a keystroke before redrawing the block.
   *
   * Long enough that a fast typist does not ask the engine to draw every letter, short
   * enough that stopping to look shows the result already there.
   */
  const REDRAW_DELAY = 90;

  /**
   * The typographic properties the field borrows from the block it stands in for.
   *
   * Copied from the live element rather than restated in the stylesheet, so a heading edits
   * at heading size and code at code size **because the renderer said so** — there is no
   * second copy of the type scale here to fall out of step with `doc-core`'s.
   */
  const TYPE = [
    'fontFamily',
    'fontSize',
    'fontStyle',
    'fontWeight',
    'lineHeight',
    'letterSpacing',
    'color',
    'textAlign',
    'whiteSpace',
    'tabSize',
  ];

  /** Box properties the field borrows, so hiding the block does not move the page. */
  const BOX = [
    'marginTop',
    'marginBottom',
    'paddingTop',
    'paddingBottom',
    'paddingLeft',
    'paddingRight',
    'borderLeftWidth',
    'borderLeftStyle',
    'borderLeftColor',
  ];

  /** The host, once attached. */
  let host = null;

  /** The block currently open, if any: `{ block, id, field, write, restore, original }`. */
  let open = null;

  /** Whatever the last save is still doing, so a click cannot overtake it. */
  let settling = Promise.resolve();

  /** Redraw generation, so a slow answer cannot overwrite a newer one. */
  let generation = 0;

  /** Pending redraw timer. */
  let timer = null;

  /**
   * Start editing on this page.
   *
   * @param {object} bridge - the host contract described above.
   */
  function attach(bridge) {
    host = bridge;
    document.addEventListener('click', onClick, true);
    document.addEventListener('keydown', onKeyDown, true);
    document.body.classList.add('dx-editable');
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
    if (block.classList.contains('dx-code')) return 'code';
    if (block.classList.contains('dx-output')) return 'output';
    if (block.classList.contains('dx-output-rendered')) return 'output';
    if (block.classList.contains('dx-nav')) return 'nav';
    if (block.classList.contains('dx-html')) return 'html';
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

  /** Open the block that was clicked. */
  function onClick(event) {
    // A link is a link. Opening an editor under someone trying to follow one is the kind of
    // thing that makes an editable page feel like a trap.
    if (event.target.closest('a')) return;
    // The fold on a code block is a control, not text: let it open the listing.
    if (event.target.closest('summary')) return;
    // Clicks inside the field the reader is already in belong to the caret.
    if (open && event.target.closest('.dx-write') === open.write) return;

    const block = blockAt(event.target);
    if (!block) return;
    if (open && open.block === block) return;

    event.preventDefault();
    event.stopPropagation();
    void enter(block.getAttribute('data-block-id'));
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
   * Put a field on the block, and keep the block rendered beside it.
   *
   * The field is the block's sibling, never its child: a `<div>` inside a `<p>` is not a
   * thing a browser will keep, and a drawing has no inside to put a textarea in. Being a
   * sibling is also what lets the rendered block stay untouched while the field grows.
   */
  async function edit(block) {
    const id = block.getAttribute('data-block-id');
    if (!id) return;
    // Normally nothing is open here — a save already closed it. A save the host *refused*
    // is the exception: it stays open so the reader can retry, and moving on to another
    // block abandons the attempt rather than leaving two fields on the page.
    if (open) close();

    let text = '';
    try {
      text = await host.source(id);
    } catch (error) {
      report(block, String(error));
      return;
    }

    const field = document.createElement('textarea');
    field.className = 'dx-field';
    field.value = text;
    field.rows = 1;
    field.spellcheck = kindOf(block) !== 'code';

    const write = document.createElement('div');
    write.className = 'dx-write';
    write.appendChild(field);
    block.classList.add('dx-editing');
    block.after(write);

    open = { block, id, field, write, restore: block.outerHTML, original: text };
    await redraw();
    field.focus();
    field.setSelectionRange(text.length, text.length);

    field.addEventListener('input', () => {
      grow(field);
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => void redraw(), REDRAW_DELAY);
    });
    field.addEventListener('blur', () => void save());
  }

  /**
   * Draw the block as the characters currently in the field would be read, and decide which
   * of the two shapes the page is in.
   *
   * The whole "always rendering" claim is this function. It runs once on open and after every
   * pause in typing, and it never touches the document — the host renders a body it was
   * handed, and saves nothing.
   */
  async function redraw() {
    if (!open || !host.draw) {
      if (open) stand(true);
      return;
    }
    const mine = ++generation;
    const { id, field } = open;

    let markup = '';
    try {
      markup = await host.draw(id, field.value);
    } catch {
      // A block that cannot be drawn is still a block that can be typed into. The field
      // stands in, the reader keeps writing, and the save will say what is wrong.
      if (open && mine === generation) stand(true);
      return;
    }
    if (!open || mine !== generation) return;

    const drawn = element(markup);
    if (drawn) {
      drawn.classList.add('dx-editing');
      open.block.replaceWith(drawn);
      open.block = drawn;
    }
    stand(standsIn(open.block, kindOf(open.block)));
  }

  /**
   * Whether the field should stand in for the block instead of sitting under it.
   *
   * It stands in when the block renders to the characters it is written in — plain prose, and
   * a code listing, which is a fold over its own source. Anything the renderer *made* — a
   * bullet, a strong word, a table, a drawing — stays on the page, because that is the thing
   * the reader is looking at while they type.
   */
  function standsIn(block, kind) {
    if (kind === 'code') return true;
    return !block.querySelector(':scope *:not(br)');
  }

  /**
   * Put the page into one of its two shapes.
   *
   * Standing in: the block is out of flow and the field takes its place, its type, and its
   * margins, so the line the reader was reading is the line they are writing. Otherwise: the
   * block stays drawn where it is and the field is a written note under it.
   */
  function stand(standing) {
    if (!open) return;
    const { block, field, write } = open;
    write.classList.toggle('dx-standin', standing);
    write.classList.toggle('dx-ruled', standing && mirror(block, field, write));
    block.classList.toggle('dx-hidden', standing);
    grow(field);
  }

  /**
   * The element whose type the field should borrow.
   *
   * A code block is a fold around a listing, and the listing is what is being typed — so the
   * type to write in is the `<pre>`'s, not the fold's. The *box* still comes from the block,
   * which is what carries the margin rule the listing is set in from.
   */
  function typeSource(block) {
    return block.querySelector('pre') || block;
  }

  /**
   * Dress the field as the block it is standing in for, or undress it when it is not.
   *
   * @returns whether the block draws its own rule down the margin — a quotation, a listing —
   *   in which case the field inherits that instead of being given the editing hairline, and
   *   the page keeps exactly one line where it had one.
   */
  function mirror(block, field, write) {
    if (!write.classList.contains('dx-standin')) {
      for (const property of TYPE) field.style[property] = '';
      for (const property of BOX) write.style[property] = '';
      return false;
    }
    const type = getComputedStyle(typeSource(block));
    for (const property of TYPE) field.style[property] = type[property];
    // A field wraps whatever its block did; `pre` would put a code line under the horizon.
    if (field.style.whiteSpace === 'pre') field.style.whiteSpace = 'pre-wrap';

    const box = getComputedStyle(block);
    for (const property of BOX) write.style[property] = box[property];
    return parseFloat(box.borderLeftWidth) > 0;
  }

  /**
   * Keep the field exactly as tall as its text, so the page never scrolls to a hidden line.
   *
   * Never shorter than one line: an empty block measures zero, and a field of no height is a
   * block a reader cannot see the caret in or type into.
   */
  function grow(field) {
    field.style.height = 'auto';
    const line = parseFloat(getComputedStyle(field).lineHeight) || 16;
    field.style.height = `${Math.max(field.scrollHeight, line)}px`;
  }

  /** Put the block back the way it was rendered, discarding whatever was typed. */
  function close() {
    if (!open) return;
    const { block, write, restore } = open;
    open = null;
    generation += 1;
    if (timer) clearTimeout(timer);
    write.remove();
    const original = element(restore);
    if (original) {
      original.classList.remove('dx-editing', 'dx-hidden');
      block.replaceWith(original);
    } else {
      block.classList.remove('dx-editing', 'dx-hidden');
    }
  }

  /**
   * Save what is in the open field, and put the answer on the page.
   *
   * The returned promise is what a click waits on, so opening the next block cannot overtake
   * saving this one.
   */
  function save(then) {
    if (!open) return settling;
    const editing = open;
    const { id, field, original } = editing;
    const text = field.value;
    if (text === original && then !== 'insert') {
      close();
      return settling;
    }
    open = null;
    generation += 1;
    if (timer) clearTimeout(timer);
    settling = host
      .commit(id, text, then)
      .then(apply)
      .catch((error) => {
        report(editing.block, String(error));
        resume(editing);
      });
    return settling;
  }

  /**
   * Put a refused save back in the reader's hands.
   *
   * The host said no, so the page still holds the field and the block exactly as they were
   * mid-edit: make them the open state again, with the typed text and the caret, so the
   * reader can fix and retry — or press Escape and lose only the attempt. Without this the
   * field was stranded: not open, so Escape, blur, and retry all answered to nothing.
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
    current.replaceWith(fresh);
    if (result.focus) void enter(result.focus);
  }

  /** The first element of an HTML string, or null if it holds none. */
  function element(markup) {
    const holder = document.createElement('div');
    holder.innerHTML = markup;
    return holder.firstElementChild;
  }

  /** Keyboard: Return, Escape, and Backspace are the whole language. */
  function onKeyDown(event) {
    if (!open) return;
    const { block, field } = open;

    if (event.key === 'Escape') {
      event.preventDefault();
      close();
      return;
    }

    // Return ends a paragraph and starts the next one — the thing writing *is*. A listing, a
    // list, and a drawing are the exception: a newline inside them is content, so Return
    // types one and it takes a modifier to finish.
    const finishing = event.key === 'Enter' && (event.metaKey || event.ctrlKey);
    const multiline = MULTILINE.includes(kindOf(block));
    if (event.key === 'Enter' && !multiline && !event.shiftKey && !finishing) {
      event.preventDefault();
      // One call, not two: the host inserts as part of committing, and a follow-up would be
      // sent about a document that had already moved on.
      void save('insert');
      return;
    }
    if (finishing) {
      event.preventDefault();
      void save();
      return;
    }

    // Backspace in an empty block deletes the block and leaves the reader at the end of the
    // one before it, the way it does in every editor a person has ever used.
    if (event.key === 'Backspace' && field.value === '' && field.selectionStart === 0) {
      event.preventDefault();
      void erase();
    }
  }

  /** Take the open block out, and put the reader in the block above it. */
  function erase() {
    if (!open) return settling;
    const editing = open;
    const { id, block, write } = editing;
    const previous = block.previousElementSibling;
    const landing = previous ? previous.getAttribute('data-block-id') : null;
    open = null;
    generation += 1;
    if (timer) clearTimeout(timer);
    write.remove();
    settling = host
      .remove(id)
      .then((result) => apply(Object.assign({ focus: landing }, result || {})))
      .catch((error) => {
        // The field came off the page before the call; a refusal puts it back where it
        // was, so the block is still being edited rather than stuck half-hidden. It goes
        // back *before* the note, so a refused delete reads where a refused save does:
        // block, what went wrong, the field still holding the text.
        if (block.isConnected) block.after(write);
        report(block, String(error));
        resume(editing);
      });
    return settling;
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
