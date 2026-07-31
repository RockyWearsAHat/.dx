declare namespace wasm_bindgen {
    /* tslint:disable */
    /* eslint-disable */

    /**
     * The editable text of one block — what a surface puts in the field when a reader clicks it.
     *
     * The same [`doc_core::edit`] the `dx` command line calls, so a block edited in the VS Code
     * webview and the same block edited in DX.app are edited by one implementation.
     *
     * Returns an error naming the ids that do exist when `id` names no block.
     */
    export function block_source(text: string, id: string): string;

    /**
     * Build a search index over a set of documents and run a query in one call.
     *
     * `docs_json` is a JSON array of `[path, document]` pairs, where each `document` is a
     * [`dto::DocumentDto`]. `query` is the search string. The result is a JSON array of
     * `{ "path": string, "score": number }` hits, sorted by descending score with ascending
     * path as a stable tie-break (exactly [`doc_core::search::SearchIndex::search`]). An empty
     * query yields an empty array. Returns an error if `docs_json` is malformed.
     */
    export function build_index_and_search(docs_json: string, query: string): string;

    /**
     * Compress bytes into a `dxz` frame, the store's framed format.
     *
     * `doc_core::compress` chooses the smallest encoding (currently DEFLATE, or stored when
     * nothing beats it) and names the codec in the frame's four magic bytes. Infallible; it
     * returns the frame as a `Uint8Array`.
     */
    export function compress(input: Uint8Array): Uint8Array;

    /**
     * Decompress any `dxz` frame, whichever codec its magic names — including `DXZ1` (LZSS),
     * which is no longer written but is decoded forever.
     *
     * Returns the original bytes, or an error if `frame` is not a valid `dxz` frame.
     */
    export function decompress(frame: Uint8Array): Uint8Array;

    /**
     * Add a block of `kind` after the block called `after`, or at the top when `after` is empty.
     *
     * Returns JSON `{"source": "<canonical .dx>", "id": "<the new block's id>"}` — the id is
     * what lets the caller put the reader's cursor in the block they just created.
     *
     * Returns an error when `kind` is not authorable or `after` names no block.
     */
    export function insert_block(text: string, after: string, kind: string, body: string): string;

    /**
     * Rebuild canonical `.dx` source from ordered per-block chunk texts (a JSON array of
     * strings), the inverse of [`split_chunks`].
     *
     * Returns an error if `texts_json` is not a JSON array of strings.
     */
    export function join_chunks(texts_json: string): string;

    /**
     * The canonical `.dx` source of one document held in a `DXCP1` pack.
     *
     * This is the whole reason a pack is committed rather than kept in a database: given the
     * pack bytes and a workspace-relative path, any host — a browser extension reading a
     * repository on github.com, an editor, a build — can recover the true document without the
     * `dx` binary, a SQLite file, or a network service. The pack is the content; this is how it
     * is read.
     *
     * Returns an error when the bytes are not a pack, or when the pack holds no such path.
     */
    export function pack_document(pack: Uint8Array, path: string): string;

    /**
     * Every document path a `DXCP1` pack carries, as a JSON array of strings.
     *
     * Useful for saying *what is* in a pack when a lookup misses — a reader who asked for the
     * wrong path should be told the right ones, not handed nothing.
     */
    export function pack_paths(pack: Uint8Array): string;

    /**
     * Parse DOCSRC (`.dx`) source text into a canonical document, returned as JSON.
     *
     * `path` is the document's path. Like the core [`doc_core::format::parse`], it does not
     * influence the canonical block output — a filename-derived title is a host concern, not
     * part of the format core — but it is accepted so a host can pass what it already has and
     * keep its call sites uniform. `text` is the raw `.dx` source.
     *
     * The returned JSON is a [`dto::DocumentDto`]: canonical blocks (unique ids, clamped
     * heading levels, recovered inline forms) plus any `@doc` header metadata.
     */
    export function parse(path: string, text: string): string;

    /**
     * The HTML one block renders to with `body` in it, saving nothing.
     *
     * This is what keeps a page rendered while a reader writes on it: the surface hands over
     * the characters currently in the field and gets back the block as it will be read. The
     * same [`doc_core::edit::preview_block`] DX.app reaches through `dx render --block`, so
     * what a reader sees mid-sentence in an editor and on a Mac is the same markup.
     *
     * `theme` is `auto`, `light`, or `dark`; `document_css` opts into the document's own
     * `::style` blocks, matching [`render_html`] so a previewed block is styled like the page
     * it sits in.
     *
     * Returns an error naming the ids that do exist when `id` names no block.
     */
    export function preview_block(text: string, id: string, body: string, theme: string, document_css: boolean): string;

    /**
     * Take one block out, returning the document's canonical source without it.
     *
     * Returns an error naming the ids that do exist when `id` names no block.
     */
    export function remove_block(text: string, id: string): string;

    /**
     * Render `.dx` source to a self-contained HTML page.
     *
     * This is the same [`doc_core::render::html`] the CLI and the screenshotter call, so a
     * document shown in an editor webview is byte-identical to the one a person opens in a
     * browser or an agent sees as an image. `theme` is `auto`, `light`, or `dark`; `fragment`
     * emits just the document container (for embedding in an existing page) instead of a full
     * document; `document_css` opts into the document's own `::style` blocks.
     */
    export function render_html(text: string, theme: string, fragment: boolean, document_css: boolean): string;

    /**
     * Outline `.dx` source: a JSON array of one entry per block.
     *
     * Each entry is `{ id, kind, level, preview, chars, runnable }` — the map a caller needs
     * to jump to, fetch, or edit one part of a document.
     */
    export function render_outline(text: string): string;

    /**
     * Slice one section out of `.dx` source, returning canonical DOCSRC for that part.
     *
     * `selector` is a block id: a heading id yields that whole section, any other id yields
     * that block plus its captured output. Returns an empty string when nothing matches.
     */
    export function render_section(text: string, selector: string): string;

    /**
     * Render `.dx` source to Markdown, the text view an agent or a diff reads.
     *
     * `include_ids` prefixes each block with a `<!-- block:<id> <kind> -->` marker.
     */
    export function render_text(text: string, include_ids: boolean): string;

    /**
     * Replace one block's body, returning the whole document's canonical source.
     *
     * Every other block comes back byte-identical.
     *
     * Returns an error naming the ids that do exist when `id` names no block.
     */
    export function set_block(text: string, id: string, body: string): string;

    /**
     * Compute the lowercase hex SHA-1 digest of `input`, byte-identical to the reference.
     */
    export function sha1_hex(input: Uint8Array): string;

    /**
     * Compute the lowercase hex SHA-256 digest of `input`, byte-identical to the reference.
     */
    export function sha256_hex(input: Uint8Array): string;

    /**
     * Split a document (JSON, [`dto::DocumentDto`] shape) into its content-addressed chunks,
     * returned as JSON: `[{"hash": "<sha256 hex>", "text": "<canonical block source>"}, …]`.
     *
     * This is the same split the native store uses, so an editor and the CLI address a block
     * identically. Returns an error if `doc_json` is not valid JSON of the expected shape.
     */
    export function split_chunks(doc_json: string): string;

    /**
     * Render a document (JSON, [`dto::DocumentDto`] shape) back to canonical DOCSRC text.
     *
     * The blocks are re-normalized before stringify, exactly as the core
     * [`doc_core::format::stringify`] does, so the result is the canonical `.dx` serialization
     * regardless of minor input irregularities. Returns an error if `doc_json` is not valid
     * JSON of the expected shape.
     */
    export function stringify(doc_json: string): string;

    /**
     * The stylesheet [`render_html`] pages are styled with.
     *
     * A host that embeds a rendered fragment — a webview, a page on github.com — needs the
     * same CSS the standalone page inlines, or the document would read as one thing in one
     * place and another somewhere else.
     */
    export function stylesheet(): string;

}
declare type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

declare interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly block_source: (a: number, b: number, c: number, d: number, e: number) => void;
    readonly build_index_and_search: (a: number, b: number, c: number, d: number, e: number) => void;
    readonly compress: (a: number, b: number, c: number) => void;
    readonly decompress: (a: number, b: number, c: number) => void;
    readonly insert_block: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number) => void;
    readonly join_chunks: (a: number, b: number, c: number) => void;
    readonly pack_document: (a: number, b: number, c: number, d: number, e: number) => void;
    readonly pack_paths: (a: number, b: number, c: number) => void;
    readonly parse: (a: number, b: number, c: number, d: number, e: number) => void;
    readonly preview_block: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number) => void;
    readonly remove_block: (a: number, b: number, c: number, d: number, e: number) => void;
    readonly render_html: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => void;
    readonly render_outline: (a: number, b: number, c: number) => void;
    readonly render_section: (a: number, b: number, c: number, d: number, e: number) => void;
    readonly render_text: (a: number, b: number, c: number, d: number) => void;
    readonly set_block: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => void;
    readonly sha1_hex: (a: number, b: number, c: number) => void;
    readonly sha256_hex: (a: number, b: number, c: number) => void;
    readonly split_chunks: (a: number, b: number, c: number) => void;
    readonly stringify: (a: number, b: number, c: number) => void;
    readonly stylesheet: (a: number) => void;
    readonly __wbindgen_add_to_stack_pointer: (a: number) => number;
    readonly __wbindgen_export: (a: number, b: number) => number;
    readonly __wbindgen_export2: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_export3: (a: number, b: number, c: number) => void;
}

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
declare function wasm_bindgen (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
