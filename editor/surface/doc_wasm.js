let wasm_bindgen = (function(exports) {
    let script_src;
    if (typeof document !== 'undefined' && document.currentScript !== null) {
        script_src = new URL(document.currentScript.src, location.href).toString();
    }

    /**
     * The canonical `::kind attrs` opening line of one block — the header an editing surface
     * shows above the body, exactly as the writer puts it in the file.
     *
     * Returns an error naming the ids that do exist when `id` names no block.
     * @param {string} text
     * @param {string} id
     * @returns {string}
     */
    function block_header(text, id) {
        let deferred4_0;
        let deferred4_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            const ptr0 = passStringToWasm0(text, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len0 = WASM_VECTOR_LEN;
            const ptr1 = passStringToWasm0(id, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len1 = WASM_VECTOR_LEN;
            wasm.block_header(retptr, ptr0, len0, ptr1, len1);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            var r2 = getDataViewMemory0().getInt32(retptr + 4 * 2, true);
            var r3 = getDataViewMemory0().getInt32(retptr + 4 * 3, true);
            var ptr3 = r0;
            var len3 = r1;
            if (r3) {
                ptr3 = 0; len3 = 0;
                throw takeObject(r2);
            }
            deferred4_0 = ptr3;
            deferred4_1 = len3;
            return getStringFromWasm0(ptr3, len3);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export3(deferred4_0, deferred4_1, 1);
        }
    }
    exports.block_header = block_header;

    /**
     * The editable text of one block — what a surface puts in the field when a reader clicks it.
     *
     * The same [`doc_core::edit`] the `dx` command line calls, so a block edited in the VS Code
     * webview and the same block edited in DX.app are edited by one implementation.
     *
     * Returns an error naming the ids that do exist when `id` names no block.
     * @param {string} text
     * @param {string} id
     * @returns {string}
     */
    function block_source(text, id) {
        let deferred4_0;
        let deferred4_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            const ptr0 = passStringToWasm0(text, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len0 = WASM_VECTOR_LEN;
            const ptr1 = passStringToWasm0(id, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len1 = WASM_VECTOR_LEN;
            wasm.block_source(retptr, ptr0, len0, ptr1, len1);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            var r2 = getDataViewMemory0().getInt32(retptr + 4 * 2, true);
            var r3 = getDataViewMemory0().getInt32(retptr + 4 * 3, true);
            var ptr3 = r0;
            var len3 = r1;
            if (r3) {
                ptr3 = 0; len3 = 0;
                throw takeObject(r2);
            }
            deferred4_0 = ptr3;
            deferred4_1 = len3;
            return getStringFromWasm0(ptr3, len3);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export3(deferred4_0, deferred4_1, 1);
        }
    }
    exports.block_source = block_source;

    /**
     * Lay out several nodes at once: `spec` is `node,x,y[,width[,height]]` per node, separated
     * by spaces — a whole board arranged in one edit, one undo step, one re-render.
     *
     * Returns an error when the board is missing, is not a `::board` block, or `spec` holds an
     * item that is not a placement.
     * @param {string} text
     * @param {string} board
     * @param {string} spec
     * @returns {string}
     */
    function board_arrange(text, board, spec) {
        let deferred5_0;
        let deferred5_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            const ptr0 = passStringToWasm0(text, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len0 = WASM_VECTOR_LEN;
            const ptr1 = passStringToWasm0(board, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len1 = WASM_VECTOR_LEN;
            const ptr2 = passStringToWasm0(spec, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len2 = WASM_VECTOR_LEN;
            wasm.board_arrange(retptr, ptr0, len0, ptr1, len1, ptr2, len2);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            var r2 = getDataViewMemory0().getInt32(retptr + 4 * 2, true);
            var r3 = getDataViewMemory0().getInt32(retptr + 4 * 3, true);
            var ptr4 = r0;
            var len4 = r1;
            if (r3) {
                ptr4 = 0; len4 = 0;
                throw takeObject(r2);
            }
            deferred5_0 = ptr4;
            deferred5_1 = len4;
            return getStringFromWasm0(ptr4, len4);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export3(deferred5_0, deferred5_1, 1);
        }
    }
    exports.board_arrange = board_arrange;

    /**
     * Take a node off a board: its line, every edge pointing at it, and — when the block it
     * showed was hidden and no other board shows it — the block itself.
     *
     * Returns an error when the board is missing or no line names `node`.
     * @param {string} text
     * @param {string} board
     * @param {string} node
     * @returns {string}
     */
    function board_detach(text, board, node) {
        let deferred5_0;
        let deferred5_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            const ptr0 = passStringToWasm0(text, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len0 = WASM_VECTOR_LEN;
            const ptr1 = passStringToWasm0(board, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len1 = WASM_VECTOR_LEN;
            const ptr2 = passStringToWasm0(node, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len2 = WASM_VECTOR_LEN;
            wasm.board_detach(retptr, ptr0, len0, ptr1, len1, ptr2, len2);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            var r2 = getDataViewMemory0().getInt32(retptr + 4 * 2, true);
            var r3 = getDataViewMemory0().getInt32(retptr + 4 * 3, true);
            var ptr4 = r0;
            var len4 = r1;
            if (r3) {
                ptr4 = 0; len4 = 0;
                throw takeObject(r2);
            }
            deferred5_0 = ptr4;
            deferred5_1 = len4;
            return getStringFromWasm0(ptr4, len4);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export3(deferred5_0, deferred5_1, 1);
        }
    }
    exports.board_detach = board_detach;

    /**
     * Every edge on a board, routed against boxes an editing surface measured.
     *
     * This and [`board_edge_preview`] are the whole reason a geometry engine loads inside an
     * editing surface's own realm at all: `render::board`'s curves are recomputed on every
     * pointer move a reader makes, which a per-frame round trip through a host bridge cannot
     * keep up with. Unlike [`board_place`]/[`board_link`]/[`board_arrange`]/[`board_detach`],
     * this changes no document — it takes boxes a surface measured and answers with numbers,
     * and calling it a thousand times leaves nothing different.
     *
     * `spec` is JSON: `{"scale": 0.82, "nodes": [{"id":"a","x":0,"y":0,"w":280,"h":160}, …],
     * "edges": [{"from":"a","to":"b","fromSide":"b","toSide":null,"label":"then"}, …]}`.
     * `fromSide`/`toSide` are the ends a reader *pinned* (`data-from-side` on the rendered
     * path), omitted or `null` for an end the router chooses; on the answer they are what it
     * chose. Answers a JSON array, one entry per routed edge — an edge naming a `from` or `to`
     * this call has no node for is silently absent, since there is nothing to route it
     * against — each carrying its own `from`/`to` so a caller matches an entry back to its
     * request rather than relying on array order. `docs/board-geometry.dx` is the full account.
     *
     * Returns an error when `spec` is not valid JSON of the shape above, or names a side that
     * is not one.
     * @param {string} spec
     * @returns {string}
     */
    function board_edge_layout(spec) {
        let deferred3_0;
        let deferred3_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            const ptr0 = passStringToWasm0(spec, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len0 = WASM_VECTOR_LEN;
            wasm.board_edge_layout(retptr, ptr0, len0);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            var r2 = getDataViewMemory0().getInt32(retptr + 4 * 2, true);
            var r3 = getDataViewMemory0().getInt32(retptr + 4 * 3, true);
            var ptr2 = r0;
            var len2 = r1;
            if (r3) {
                ptr2 = 0; len2 = 0;
                throw takeObject(r2);
            }
            deferred3_0 = ptr2;
            deferred3_1 = len2;
            return getStringFromWasm0(ptr2, len2);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export3(deferred3_0, deferred3_1, 1);
        }
    }
    exports.board_edge_layout = board_edge_layout;

    /**
     * The line a reader is dragging out of a node's side, before it lands.
     *
     * `spec` is JSON: `{"from":{"box":{"x":0,"y":0,"w":280,"h":160},"side":"r"},
     * "to":{"box":{...},"side":"t"}}` when the pointer is over a node the line would snap to,
     * or `"to":{"x":413,"y":220}` while it is following the pointer over open paper.
     * `obstacles` is an optional array of boxes to dodge — normally omitted, since a preview
     * that swerved around boxes mid-drag would visibly jump under the pointer.
     *
     * Answers JSON `{"path": "M … C …, …, … L …"}` — the same shape [`board_edge_layout`]'s
     * `path` carries, ready to set as an SVG `<path d>` directly.
     *
     * Returns an error when `spec` is not valid JSON of the shape above, or names a side that
     * is not one.
     * @param {string} spec
     * @returns {string}
     */
    function board_edge_preview(spec) {
        let deferred3_0;
        let deferred3_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            const ptr0 = passStringToWasm0(spec, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len0 = WASM_VECTOR_LEN;
            wasm.board_edge_preview(retptr, ptr0, len0);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            var r2 = getDataViewMemory0().getInt32(retptr + 4 * 2, true);
            var r3 = getDataViewMemory0().getInt32(retptr + 4 * 3, true);
            var ptr2 = r0;
            var len2 = r1;
            if (r3) {
                ptr2 = 0; len2 = 0;
                throw takeObject(r2);
            }
            deferred3_0 = ptr2;
            deferred3_1 = len2;
            return getStringFromWasm0(ptr2, len2);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export3(deferred3_0, deferred3_1, 1);
        }
    }
    exports.board_edge_preview = board_edge_preview;

    /**
     * Draw (`linked` true) or erase (`linked` false) the edge from `from` to `to` on a board.
     *
     * `from_side` and `to_side` are the edges of the two nodes the line joins — `left`,
     * `right`, `top`, `bottom`, or their initials — and empty leaves that end to the renderer.
     *
     * Returns an error when the board or either end is missing, or when the two ends are the
     * same node.
     * @param {string} text
     * @param {string} board
     * @param {string} from
     * @param {string} to
     * @param {boolean} linked
     * @param {string} from_side
     * @param {string} to_side
     * @returns {string}
     */
    function board_link(text, board, from, to, linked, from_side, to_side) {
        let deferred8_0;
        let deferred8_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            const ptr0 = passStringToWasm0(text, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len0 = WASM_VECTOR_LEN;
            const ptr1 = passStringToWasm0(board, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len1 = WASM_VECTOR_LEN;
            const ptr2 = passStringToWasm0(from, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len2 = WASM_VECTOR_LEN;
            const ptr3 = passStringToWasm0(to, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len3 = WASM_VECTOR_LEN;
            const ptr4 = passStringToWasm0(from_side, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len4 = WASM_VECTOR_LEN;
            const ptr5 = passStringToWasm0(to_side, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len5 = WASM_VECTOR_LEN;
            wasm.board_link(retptr, ptr0, len0, ptr1, len1, ptr2, len2, ptr3, len3, linked, ptr4, len4, ptr5, len5);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            var r2 = getDataViewMemory0().getInt32(retptr + 4 * 2, true);
            var r3 = getDataViewMemory0().getInt32(retptr + 4 * 3, true);
            var ptr7 = r0;
            var len7 = r1;
            if (r3) {
                ptr7 = 0; len7 = 0;
                throw takeObject(r2);
            }
            deferred8_0 = ptr7;
            deferred8_1 = len7;
            return getStringFromWasm0(ptr7, len7);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export3(deferred8_0, deferred8_1, 1);
        }
    }
    exports.board_link = board_link;

    /**
     * Put a node at `x`,`y` on a board, sized `w` by `h` — moving its line, adding one, or
     * (when `node` is empty) creating a fresh node with a hidden paragraph ready to be
     * written. `w`/`h` are canvas pixels; `0` keeps what the line already says. The board
     * settles afterwards, so the placed node keeps its spot and nothing is left covered.
     *
     * The same [`doc_core::edit::board_place`] the `dx board` command runs, so a node dragged
     * in an editor and one placed by an agent land as the identical line. An editing surface
     * only ever states measured pixels, so this door speaks numbers; the `page`/`fit` rules
     * are spelled in the node line itself and through `dx board`.
     *
     * Returns JSON `{"source": "<canonical .dx>", "id": "<the node's id>"}`.
     *
     * Returns an error when the board is missing or is not a `::board` block.
     * @param {string} text
     * @param {string} board
     * @param {string} node
     * @param {number} x
     * @param {number} y
     * @param {number} w
     * @param {number} h
     * @returns {string}
     */
    function board_place(text, board, node, x, y, w, h) {
        let deferred5_0;
        let deferred5_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            const ptr0 = passStringToWasm0(text, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len0 = WASM_VECTOR_LEN;
            const ptr1 = passStringToWasm0(board, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len1 = WASM_VECTOR_LEN;
            const ptr2 = passStringToWasm0(node, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len2 = WASM_VECTOR_LEN;
            wasm.board_place(retptr, ptr0, len0, ptr1, len1, ptr2, len2, x, y, w, h);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            var r2 = getDataViewMemory0().getInt32(retptr + 4 * 2, true);
            var r3 = getDataViewMemory0().getInt32(retptr + 4 * 3, true);
            var ptr4 = r0;
            var len4 = r1;
            if (r3) {
                ptr4 = 0; len4 = 0;
                throw takeObject(r2);
            }
            deferred5_0 = ptr4;
            deferred5_1 = len4;
            return getStringFromWasm0(ptr4, len4);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export3(deferred5_0, deferred5_1, 1);
        }
    }
    exports.board_place = board_place;

    /**
     * One field's text as decorated HTML: the source with its marks styled in place.
     *
     * What the editing surface shows *inside* the field — `**bold**` set in bold with the
     * `**` still on the line. [`doc_core::render::field_html`] keeps every character of the
     * input in the output's text, which is what lets the surface map caret offsets between
     * the two. DX.app reaches the same renderer through `dx render --field`.
     * @param {string} text
     * @returns {string}
     */
    function field_html(text) {
        let deferred2_0;
        let deferred2_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            const ptr0 = passStringToWasm0(text, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len0 = WASM_VECTOR_LEN;
            wasm.field_html(retptr, ptr0, len0);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred2_0 = r0;
            deferred2_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export3(deferred2_0, deferred2_1, 1);
        }
    }
    exports.field_html = field_html;

    /**
     * Add a block of `kind` after the block called `after`, or at the top when `after` is empty.
     *
     * Returns JSON `{"source": "<canonical .dx>", "id": "<the new block's id>"}` — the id is
     * what lets the caller put the reader's cursor in the block they just created.
     *
     * Returns an error when `kind` is not authorable or `after` names no block.
     * @param {string} text
     * @param {string} after
     * @param {string} kind
     * @param {string} body
     * @returns {string}
     */
    function insert_block(text, after, kind, body) {
        let deferred6_0;
        let deferred6_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            const ptr0 = passStringToWasm0(text, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len0 = WASM_VECTOR_LEN;
            const ptr1 = passStringToWasm0(after, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len1 = WASM_VECTOR_LEN;
            const ptr2 = passStringToWasm0(kind, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len2 = WASM_VECTOR_LEN;
            const ptr3 = passStringToWasm0(body, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len3 = WASM_VECTOR_LEN;
            wasm.insert_block(retptr, ptr0, len0, ptr1, len1, ptr2, len2, ptr3, len3);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            var r2 = getDataViewMemory0().getInt32(retptr + 4 * 2, true);
            var r3 = getDataViewMemory0().getInt32(retptr + 4 * 3, true);
            var ptr5 = r0;
            var len5 = r1;
            if (r3) {
                ptr5 = 0; len5 = 0;
                throw takeObject(r2);
            }
            deferred6_0 = ptr5;
            deferred6_1 = len5;
            return getStringFromWasm0(ptr5, len5);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export3(deferred6_0, deferred6_1, 1);
        }
    }
    exports.insert_block = insert_block;

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
     * @param {Uint8Array} pack
     * @param {string} path
     * @returns {string}
     */
    function pack_document(pack, path) {
        let deferred4_0;
        let deferred4_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            const ptr0 = passArray8ToWasm0(pack, wasm.__wbindgen_export);
            const len0 = WASM_VECTOR_LEN;
            const ptr1 = passStringToWasm0(path, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len1 = WASM_VECTOR_LEN;
            wasm.pack_document(retptr, ptr0, len0, ptr1, len1);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            var r2 = getDataViewMemory0().getInt32(retptr + 4 * 2, true);
            var r3 = getDataViewMemory0().getInt32(retptr + 4 * 3, true);
            var ptr3 = r0;
            var len3 = r1;
            if (r3) {
                ptr3 = 0; len3 = 0;
                throw takeObject(r2);
            }
            deferred4_0 = ptr3;
            deferred4_1 = len3;
            return getStringFromWasm0(ptr3, len3);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export3(deferred4_0, deferred4_1, 1);
        }
    }
    exports.pack_document = pack_document;

    /**
     * Every document path a `DXCP1` pack carries, as a JSON array of strings.
     *
     * Useful for saying *what is* in a pack when a lookup misses — a reader who asked for the
     * wrong path should be told the right ones, not handed nothing.
     * @param {Uint8Array} pack
     * @returns {string}
     */
    function pack_paths(pack) {
        let deferred3_0;
        let deferred3_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            const ptr0 = passArray8ToWasm0(pack, wasm.__wbindgen_export);
            const len0 = WASM_VECTOR_LEN;
            wasm.pack_paths(retptr, ptr0, len0);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            var r2 = getDataViewMemory0().getInt32(retptr + 4 * 2, true);
            var r3 = getDataViewMemory0().getInt32(retptr + 4 * 3, true);
            var ptr2 = r0;
            var len2 = r1;
            if (r3) {
                ptr2 = 0; len2 = 0;
                throw takeObject(r2);
            }
            deferred3_0 = ptr2;
            deferred3_1 = len2;
            return getStringFromWasm0(ptr2, len2);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export3(deferred3_0, deferred3_1, 1);
        }
    }
    exports.pack_paths = pack_paths;

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
     * @param {string} path
     * @param {string} text
     * @returns {string}
     */
    function parse(path, text) {
        let deferred3_0;
        let deferred3_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            const ptr0 = passStringToWasm0(path, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len0 = WASM_VECTOR_LEN;
            const ptr1 = passStringToWasm0(text, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len1 = WASM_VECTOR_LEN;
            wasm.parse(retptr, ptr0, len0, ptr1, len1);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred3_0 = r0;
            deferred3_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export3(deferred3_0, deferred3_1, 1);
        }
    }
    exports.parse = parse;

    /**
     * What a gathering host still has to fetch, as JSON
     * `[{"kind": "file" | "document", "path": string}, …]`, deduplicated.
     *
     * `held` is the same `resources` JSON [`render_html`] takes, plus an `"absent"` array of
     * the paths already tried and not got. The host loops — ask, fetch what comes back, add it
     * (or mark it absent), ask again — until the answer is `[]`, and then renders against what
     * it gathered. The walk itself is [`doc_core::resolve::Provided::pending`]: which
     * references open it, how a gathered page extends it, and when it stops are decided there,
     * once, for every host.
     * @param {string} text
     * @param {string | null} [held]
     * @returns {string}
     */
    function pending(text, held) {
        let deferred3_0;
        let deferred3_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            const ptr0 = passStringToWasm0(text, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len0 = WASM_VECTOR_LEN;
            var ptr1 = isLikeNone(held) ? 0 : passStringToWasm0(held, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            var len1 = WASM_VECTOR_LEN;
            wasm.pending(retptr, ptr0, len0, ptr1, len1);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred3_0 = r0;
            deferred3_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export3(deferred3_0, deferred3_1, 1);
        }
    }
    exports.pending = pending;

    /**
     * The digest a `.dx` pointer line records, or `""` when `text` is document content.
     *
     * A host that opens files off a disk has to know which of them are pointers into the store,
     * and it asks rather than matching a pattern of its own: [`doc_core::pointer`] is the one
     * recognizer, so a file is a pointer to every surface or to none.
     * @param {string} text
     * @returns {string}
     */
    function pointer_digest(text) {
        let deferred2_0;
        let deferred2_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            const ptr0 = passStringToWasm0(text, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len0 = WASM_VECTOR_LEN;
            wasm.pointer_digest(retptr, ptr0, len0);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred2_0 = r0;
            deferred2_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export3(deferred2_0, deferred2_1, 1);
        }
    }
    exports.pointer_digest = pointer_digest;

    /**
     * Take one block out, returning the document's canonical source without it.
     *
     * Returns an error naming the ids that do exist when `id` names no block.
     * @param {string} text
     * @param {string} id
     * @returns {string}
     */
    function remove_block(text, id) {
        let deferred4_0;
        let deferred4_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            const ptr0 = passStringToWasm0(text, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len0 = WASM_VECTOR_LEN;
            const ptr1 = passStringToWasm0(id, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len1 = WASM_VECTOR_LEN;
            wasm.remove_block(retptr, ptr0, len0, ptr1, len1);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            var r2 = getDataViewMemory0().getInt32(retptr + 4 * 2, true);
            var r3 = getDataViewMemory0().getInt32(retptr + 4 * 3, true);
            var ptr3 = r0;
            var len3 = r1;
            if (r3) {
                ptr3 = 0; len3 = 0;
                throw takeObject(r2);
            }
            deferred4_0 = ptr3;
            deferred4_1 = len3;
            return getStringFromWasm0(ptr3, len3);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export3(deferred4_0, deferred4_1, 1);
        }
    }
    exports.remove_block = remove_block;

    /**
     * Render `.dx` source to a self-contained HTML page.
     *
     * This is the same [`doc_core::render::html`] the CLI and the screenshotter call, so a
     * document shown in an editor webview is byte-identical to the one a person opens in a
     * browser or an agent sees as an image. `theme` is `auto`, `light`, or `dark`; `fragment`
     * emits just the document container (for embedding in an existing page) instead of a full
     * document. The document's own `::style` blocks are applied either way.
     *
     * `resources` answers the document's references ([`pending`] says what to gather): JSON
     * `{"files": {path: text}, "documents": {path: source}}`. A host with nothing to give
     * omits it, and every reference renders as its honest sentence — the page never shows a
     * referenced listing as silently empty. Malformed JSON is treated the same way, which
     * keeps the mistake visible on the page instead of hidden behind a blank block.
     * @param {string} text
     * @param {string} theme
     * @param {boolean} fragment
     * @param {string | null} [resources]
     * @returns {string}
     */
    function render_html(text, theme, fragment, resources) {
        let deferred4_0;
        let deferred4_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            const ptr0 = passStringToWasm0(text, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len0 = WASM_VECTOR_LEN;
            const ptr1 = passStringToWasm0(theme, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len1 = WASM_VECTOR_LEN;
            var ptr2 = isLikeNone(resources) ? 0 : passStringToWasm0(resources, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            var len2 = WASM_VECTOR_LEN;
            wasm.render_html(retptr, ptr0, len0, ptr1, len1, fragment, ptr2, len2);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred4_0 = r0;
            deferred4_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export3(deferred4_0, deferred4_1, 1);
        }
    }
    exports.render_html = render_html;

    /**
     * Outline `.dx` source: a JSON array of one entry per block.
     *
     * Each entry is `{ id, kind, level, preview, chars, runnable }` — the map a caller needs
     * to jump to, fetch, or edit one part of a document.
     * @param {string} text
     * @returns {string}
     */
    function render_outline(text) {
        let deferred2_0;
        let deferred2_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            const ptr0 = passStringToWasm0(text, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len0 = WASM_VECTOR_LEN;
            wasm.render_outline(retptr, ptr0, len0);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred2_0 = r0;
            deferred2_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export3(deferred2_0, deferred2_1, 1);
        }
    }
    exports.render_outline = render_outline;

    /**
     * Render `.dx` source to Markdown, the text view an agent or a diff reads.
     *
     * `include_ids` prefixes each block with a `<!-- block:<id> <kind> -->` marker.
     * @param {string} text
     * @param {boolean} include_ids
     * @returns {string}
     */
    function render_text(text, include_ids) {
        let deferred2_0;
        let deferred2_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            const ptr0 = passStringToWasm0(text, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len0 = WASM_VECTOR_LEN;
            wasm.render_text(retptr, ptr0, len0, include_ids);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred2_0 = r0;
            deferred2_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export3(deferred2_0, deferred2_1, 1);
        }
    }
    exports.render_text = render_text;

    /**
     * Replace one block wholesale — header and body — so a reader retyping a header retypes
     * the block. An empty `header` means the body is plain text, read the way the file itself
     * would read it.
     *
     * Returns JSON `{"source": "<canonical .dx>", "id": "<the replacement's id>"}` — the id is
     * where the reader's cursor belongs afterwards, and it survives edits whose header named
     * no id of its own.
     *
     * Returns an error when `id` names no block, the header names an unknown kind or `output`,
     * or it claims an id another block holds.
     * @param {string} text
     * @param {string} id
     * @param {string} header
     * @param {string} body
     * @returns {string}
     */
    function replace_block(text, id, header, body) {
        let deferred6_0;
        let deferred6_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            const ptr0 = passStringToWasm0(text, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len0 = WASM_VECTOR_LEN;
            const ptr1 = passStringToWasm0(id, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len1 = WASM_VECTOR_LEN;
            const ptr2 = passStringToWasm0(header, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len2 = WASM_VECTOR_LEN;
            const ptr3 = passStringToWasm0(body, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len3 = WASM_VECTOR_LEN;
            wasm.replace_block(retptr, ptr0, len0, ptr1, len1, ptr2, len2, ptr3, len3);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            var r2 = getDataViewMemory0().getInt32(retptr + 4 * 2, true);
            var r3 = getDataViewMemory0().getInt32(retptr + 4 * 3, true);
            var ptr5 = r0;
            var len5 = r1;
            if (r3) {
                ptr5 = 0; len5 = 0;
                throw takeObject(r2);
            }
            deferred6_0 = ptr5;
            deferred6_1 = len5;
            return getStringFromWasm0(ptr5, len5);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export3(deferred6_0, deferred6_1, 1);
        }
    }
    exports.replace_block = replace_block;

    /**
     * Replace one block's body, returning the whole document's canonical source.
     *
     * Every other block comes back byte-identical.
     *
     * Returns an error naming the ids that do exist when `id` names no block.
     * @param {string} text
     * @param {string} id
     * @param {string} body
     * @returns {string}
     */
    function set_block(text, id, body) {
        let deferred5_0;
        let deferred5_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            const ptr0 = passStringToWasm0(text, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len0 = WASM_VECTOR_LEN;
            const ptr1 = passStringToWasm0(id, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len1 = WASM_VECTOR_LEN;
            const ptr2 = passStringToWasm0(body, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len2 = WASM_VECTOR_LEN;
            wasm.set_block(retptr, ptr0, len0, ptr1, len1, ptr2, len2);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            var r2 = getDataViewMemory0().getInt32(retptr + 4 * 2, true);
            var r3 = getDataViewMemory0().getInt32(retptr + 4 * 3, true);
            var ptr4 = r0;
            var len4 = r1;
            if (r3) {
                ptr4 = 0; len4 = 0;
                throw takeObject(r2);
            }
            deferred5_0 = ptr4;
            deferred5_1 = len4;
            return getStringFromWasm0(ptr4, len4);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export3(deferred5_0, deferred5_1, 1);
        }
    }
    exports.set_block = set_block;

    /**
     * Compute the lowercase hex SHA-256 digest of `input`, byte-identical to the reference.
     * @param {Uint8Array} input
     * @returns {string}
     */
    function sha256_hex(input) {
        let deferred2_0;
        let deferred2_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            const ptr0 = passArray8ToWasm0(input, wasm.__wbindgen_export);
            const len0 = WASM_VECTOR_LEN;
            wasm.sha256_hex(retptr, ptr0, len0);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred2_0 = r0;
            deferred2_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export3(deferred2_0, deferred2_1, 1);
        }
    }
    exports.sha256_hex = sha256_hex;

    /**
     * Render a document (JSON, [`dto::DocumentDto`] shape) back to canonical DOCSRC text.
     *
     * The blocks are re-normalized before stringify, exactly as the core
     * [`doc_core::format::stringify`] does, so the result is the canonical `.dx` serialization
     * regardless of minor input irregularities. Returns an error if `doc_json` is not valid
     * JSON of the expected shape.
     * @param {string} doc_json
     * @returns {string}
     */
    function stringify(doc_json) {
        let deferred3_0;
        let deferred3_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            const ptr0 = passStringToWasm0(doc_json, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len0 = WASM_VECTOR_LEN;
            wasm.stringify(retptr, ptr0, len0);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            var r2 = getDataViewMemory0().getInt32(retptr + 4 * 2, true);
            var r3 = getDataViewMemory0().getInt32(retptr + 4 * 3, true);
            var ptr2 = r0;
            var len2 = r1;
            if (r3) {
                ptr2 = 0; len2 = 0;
                throw takeObject(r2);
            }
            deferred3_0 = ptr2;
            deferred3_1 = len2;
            return getStringFromWasm0(ptr2, len2);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export3(deferred3_0, deferred3_1, 1);
        }
    }
    exports.stringify = stringify;

    /**
     * The stylesheet [`render_html`] pages are styled with.
     *
     * A host that embeds a rendered fragment — a webview, a page on github.com — needs the
     * same CSS the standalone page inlines, or the document would read as one thing in one
     * place and another somewhere else.
     * @returns {string}
     */
    function stylesheet() {
        let deferred1_0;
        let deferred1_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.stylesheet(retptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred1_0 = r0;
            deferred1_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export3(deferred1_0, deferred1_1, 1);
        }
    }
    exports.stylesheet = stylesheet;

    /**
     * Tick or untick the box at position `item` of the checklist called `id`, counting from
     * zero — the position the renderer writes on every mark as `data-check`.
     *
     * The same [`doc_core::edit::toggle_check`] the `dx check` command runs, so a box clicked
     * on the page and one ticked by an agent flip the identical marker. Every other item, and
     * every other block, comes back byte-identical.
     *
     * Returns an error when `id` names no block, names one that is not a checklist, or when the
     * checklist has no item at `item`.
     * @param {string} text
     * @param {string} id
     * @param {number} item
     * @returns {string}
     */
    function toggle_check(text, id, item) {
        let deferred4_0;
        let deferred4_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            const ptr0 = passStringToWasm0(text, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len0 = WASM_VECTOR_LEN;
            const ptr1 = passStringToWasm0(id, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len1 = WASM_VECTOR_LEN;
            wasm.toggle_check(retptr, ptr0, len0, ptr1, len1, item);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            var r2 = getDataViewMemory0().getInt32(retptr + 4 * 2, true);
            var r3 = getDataViewMemory0().getInt32(retptr + 4 * 3, true);
            var ptr3 = r0;
            var len3 = r1;
            if (r3) {
                ptr3 = 0; len3 = 0;
                throw takeObject(r2);
            }
            deferred4_0 = ptr3;
            deferred4_1 = len3;
            return getStringFromWasm0(ptr3, len3);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export3(deferred4_0, deferred4_1, 1);
        }
    }
    exports.toggle_check = toggle_check;

    /**
     * The authoring vocabulary and node geometry an editing surface offers, as JSON.
     *
     * `editor/surface/edit.js` carries these facts as constants because a completion menu and
     * a drag handle need them before any call goes out. This export is what pins that mirror
     * to the engine: [`doc_core::surface`] decides the kinds, the attributes, and the smallest
     * box a node may be, and `editor/vscode/test/vocabulary.test.mjs` fails when the surface
     * and this answer disagree.
     * @returns {string}
     */
    function vocabulary() {
        let deferred1_0;
        let deferred1_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.vocabulary(retptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred1_0 = r0;
            deferred1_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export3(deferred1_0, deferred1_1, 1);
        }
    }
    exports.vocabulary = vocabulary;
    function __wbg_get_imports() {
        const import0 = {
            __proto__: null,
            __wbindgen_cast_0000000000000001: function(arg0, arg1) {
                // Cast intrinsic for `Ref(String) -> Externref`.
                const ret = getStringFromWasm0(arg0, arg1);
                return addHeapObject(ret);
            },
        };
        return {
            __proto__: null,
            "./doc_wasm_bg.js": import0,
        };
    }

    function addHeapObject(obj) {
        if (heap_next === heap.length) heap.push(heap.length + 1);
        const idx = heap_next;
        heap_next = heap[idx];

        heap[idx] = obj;
        return idx;
    }

    function dropObject(idx) {
        if (idx < 1028) return;
        heap[idx] = heap_next;
        heap_next = idx;
    }

    let cachedDataViewMemory0 = null;
    function getDataViewMemory0() {
        if (cachedDataViewMemory0 === null || cachedDataViewMemory0.buffer.detached === true || (cachedDataViewMemory0.buffer.detached === undefined && cachedDataViewMemory0.buffer !== wasm.memory.buffer)) {
            cachedDataViewMemory0 = new DataView(wasm.memory.buffer);
        }
        return cachedDataViewMemory0;
    }

    function getStringFromWasm0(ptr, len) {
        return decodeText(ptr >>> 0, len);
    }

    let cachedUint8ArrayMemory0 = null;
    function getUint8ArrayMemory0() {
        if (cachedUint8ArrayMemory0 === null || cachedUint8ArrayMemory0.byteLength === 0) {
            cachedUint8ArrayMemory0 = new Uint8Array(wasm.memory.buffer);
        }
        return cachedUint8ArrayMemory0;
    }

    function getObject(idx) { return heap[idx]; }

    let heap = new Array(1024).fill(undefined);
    heap.push(undefined, null, true, false);

    let heap_next = heap.length;

    function isLikeNone(x) {
        return x === undefined || x === null;
    }

    function passArray8ToWasm0(arg, malloc) {
        const ptr = malloc(arg.length * 1, 1) >>> 0;
        getUint8ArrayMemory0().set(arg, ptr / 1);
        WASM_VECTOR_LEN = arg.length;
        return ptr;
    }

    function passStringToWasm0(arg, malloc, realloc) {
        if (realloc === undefined) {
            const buf = cachedTextEncoder.encode(arg);
            const ptr = malloc(buf.length, 1) >>> 0;
            getUint8ArrayMemory0().subarray(ptr, ptr + buf.length).set(buf);
            WASM_VECTOR_LEN = buf.length;
            return ptr;
        }

        let len = arg.length;
        let ptr = malloc(len, 1) >>> 0;

        const mem = getUint8ArrayMemory0();

        let offset = 0;

        for (; offset < len; offset++) {
            const code = arg.charCodeAt(offset);
            if (code > 0x7F) break;
            mem[ptr + offset] = code;
        }
        if (offset !== len) {
            if (offset !== 0) {
                arg = arg.slice(offset);
            }
            ptr = realloc(ptr, len, len = offset + arg.length * 3, 1) >>> 0;
            const view = getUint8ArrayMemory0().subarray(ptr + offset, ptr + len);
            const ret = cachedTextEncoder.encodeInto(arg, view);

            offset += ret.written;
            ptr = realloc(ptr, len, offset, 1) >>> 0;
        }

        WASM_VECTOR_LEN = offset;
        return ptr;
    }

    function takeObject(idx) {
        const ret = getObject(idx);
        dropObject(idx);
        return ret;
    }

    let cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true, fatal: true });
    cachedTextDecoder.decode();
    function decodeText(ptr, len) {
        return cachedTextDecoder.decode(getUint8ArrayMemory0().subarray(ptr, ptr + len));
    }

    const cachedTextEncoder = new TextEncoder();

    if (!('encodeInto' in cachedTextEncoder)) {
        cachedTextEncoder.encodeInto = function (arg, view) {
            const buf = cachedTextEncoder.encode(arg);
            view.set(buf);
            return {
                read: arg.length,
                written: buf.length
            };
        };
    }

    let WASM_VECTOR_LEN = 0;

    let wasmModule, wasmInstance, wasm;
    function __wbg_finalize_init(instance, module) {
        wasmInstance = instance;
        wasm = instance.exports;
        wasmModule = module;
        cachedDataViewMemory0 = null;
        cachedUint8ArrayMemory0 = null;
        return wasm;
    }

    async function __wbg_load(module, imports) {
        if (typeof Response === 'function' && module instanceof Response) {
            if (typeof WebAssembly.instantiateStreaming === 'function') {
                try {
                    return await WebAssembly.instantiateStreaming(module, imports);
                } catch (e) {
                    const validResponse = module.ok && expectedResponseType(module.type);

                    if (validResponse && module.headers.get('Content-Type') !== 'application/wasm') {
                        console.warn("`WebAssembly.instantiateStreaming` failed because your server does not serve Wasm with `application/wasm` MIME type. Falling back to `WebAssembly.instantiate` which is slower. Original error:\n", e);

                    } else { throw e; }
                }
            }

            const bytes = await module.arrayBuffer();
            return await WebAssembly.instantiate(bytes, imports);
        } else {
            const instance = await WebAssembly.instantiate(module, imports);

            if (instance instanceof WebAssembly.Instance) {
                return { instance, module };
            } else {
                return instance;
            }
        }

        function expectedResponseType(type) {
            switch (type) {
                case 'basic': case 'cors': case 'default': return true;
            }
            return false;
        }
    }

    function initSync(module) {
        if (wasm !== undefined) return wasm;


        if (module !== undefined) {
            if (Object.getPrototypeOf(module) === Object.prototype) {
                ({module} = module)
            } else {
                console.warn('using deprecated parameters for `initSync()`; pass a single object instead')
            }
        }

        const imports = __wbg_get_imports();
        if (!(module instanceof WebAssembly.Module)) {
            module = new WebAssembly.Module(module);
        }
        const instance = new WebAssembly.Instance(module, imports);
        return __wbg_finalize_init(instance, module);
    }

    async function __wbg_init(module_or_path) {
        if (wasm !== undefined) return wasm;


        if (module_or_path !== undefined) {
            if (Object.getPrototypeOf(module_or_path) === Object.prototype) {
                ({module_or_path} = module_or_path)
            } else {
                console.warn('using deprecated parameters for the initialization function; pass a single object instead')
            }
        }

        if (module_or_path === undefined && script_src !== undefined) {
            module_or_path = script_src.replace(/\.js$/, "_bg.wasm");
        }
        const imports = __wbg_get_imports();

        if (typeof module_or_path === 'string' || (typeof Request === 'function' && module_or_path instanceof Request) || (typeof URL === 'function' && module_or_path instanceof URL)) {
            module_or_path = fetch(module_or_path);
        }

        const { instance, module } = await __wbg_load(await module_or_path, imports);

        return __wbg_finalize_init(instance, module);
    }

    return Object.assign(__wbg_init, { initSync }, exports);
})({ __proto__: null });
