"use strict";
// webview-surface-controller.ts
// Source: edit this file, then run `npm run build:surface` to regenerate the
// compiled JS that the webview loads at runtime.
Object.defineProperty(exports, "__esModule", { value: true });
exports.createSurfaceController = createSurfaceController;
// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------
function createSurfaceController(options) {
    const { getDocModel, getCurrentDocPath, getFsmViewState, getDocumentHistoryDepths, getCurrentTheme, getResolvedTheme, isEditModeEnabled, getFocusedBlockIndex, summarizeBlockTypeCounts, setEditMode, openBlockSource, closeBlockSource, undoLastFsmTransition, commitOpenSourcesForHistory, performGlobalUndo, performGlobalRedo, } = options;
    // --- Private DOM helpers ------------------------------------------------
    function findBlockWrap(index) {
        return document.querySelector(`.block-wrap[data-block-index="${index}"]`);
    }
    function focusBlock(index) {
        const wrap = findBlockWrap(index);
        if (!wrap)
            return false;
        const view = wrap.querySelector('.block-view');
        if (!(view instanceof HTMLElement))
            return false;
        view.focus();
        return true;
    }
    function scrollToBlock(index, behavior = 'instant') {
        const wrap = findBlockWrap(index);
        if (!wrap)
            return false;
        wrap.scrollIntoView({
            block: 'center',
            behavior: behavior === 'smooth' ? 'smooth' : 'auto',
        });
        return true;
    }
    function withAnimationFrame() {
        return new Promise((resolve) => {
            requestAnimationFrame(() => resolve());
        });
    }
    // --- Public surface methods ---------------------------------------------
    function captureSurfaceSnapshot(opts = {}) {
        const includeText = opts.includeText !== false;
        const includeStyles = Boolean(opts.includeStyles);
        const page = document.querySelector('.page');
        const pageRect = page ? page.getBoundingClientRect() : null;
        const wraps = Array.from(document.querySelectorAll('.block-wrap'));
        const docModel = getDocModel();
        const blocks = wraps.map((wrap) => {
            const index = Number.parseInt(wrap.dataset['blockIndex'] ?? '', 10);
            const view = wrap.querySelector('.block-view');
            const srcWrap = wrap.querySelector('.block-src-wrapper');
            const rect = wrap.getBoundingClientRect();
            const block = Number.isFinite(index) && docModel && Array.isArray(docModel.blocks)
                ? (docModel.blocks[index] ?? null)
                : null;
            const computed = view && includeStyles ? window.getComputedStyle(view) : null;
            return {
                index: Number.isFinite(index) ? index : -1,
                id: block?.id ?? '',
                type: block?.type ?? 'unknown',
                rect: {
                    x: Math.round(rect.x),
                    y: Math.round(rect.y),
                    width: Math.round(rect.width),
                    height: Math.round(rect.height),
                },
                inViewport: rect.bottom > 0 && rect.top < window.innerHeight,
                sourceOpen: Boolean(srcWrap && srcWrap.style.display === 'block'),
                text: includeText
                    ? block
                        ? Array.isArray(block.items)
                            ? block.items
                                .map((item) => typeof item === 'object' && item
                                ? item.text ?? ''
                                : String(item))
                                .join(' | ')
                            : String(block.text ?? block.alt ?? block.src ?? '')
                        : String(view ? view.textContent ?? '' : '')
                    : undefined,
                style: computed
                    ? {
                        fontSize: computed.fontSize,
                        lineHeight: computed.lineHeight,
                        color: computed.color,
                        backgroundColor: computed.backgroundColor,
                    }
                    : undefined,
            };
        });
        return {
            documentPath: getCurrentDocPath(),
            capturedAt: new Date().toISOString(),
            theme: getCurrentTheme(),
            resolvedTheme: getResolvedTheme(),
            editMode: isEditModeEnabled(),
            fsm: getFsmViewState(),
            documentHistory: getDocumentHistoryDepths(),
            viewport: {
                width: window.innerWidth,
                height: window.innerHeight,
                scrollX: window.scrollX,
                scrollY: window.scrollY,
            },
            page: pageRect
                ? {
                    x: Math.round(pageRect.x),
                    y: Math.round(pageRect.y),
                    width: Math.round(pageRect.width),
                    height: Math.round(pageRect.height),
                }
                : null,
            focusedBlockIndex: getFocusedBlockIndex(),
            blockCounts: summarizeBlockTypeCounts(),
            blocks,
        };
    }
    async function runSurfaceAction(payload = {}) {
        const action = String(payload.action ?? '').trim();
        if (!action) {
            throw new Error('Surface action is required.');
        }
        if (action === 'setEditMode') {
            setEditMode(Boolean(payload.enabled));
        }
        else if (action === 'scrollBy') {
            const deltaY = Number(payload.deltaY ?? 0);
            window.scrollBy({ top: Number.isFinite(deltaY) ? deltaY : 0, behavior: 'auto' });
        }
        else if (action === 'scrollTo') {
            const top = Number(payload.top ?? 0);
            window.scrollTo({ top: Number.isFinite(top) ? top : 0, behavior: 'auto' });
        }
        else if (action === 'scrollToBlock') {
            const index = Number.parseInt(String(payload.blockIndex ?? '-1'), 10);
            if (!Number.isFinite(index) || !scrollToBlock(index, String(payload.behavior ?? 'instant'))) {
                throw new Error('Unable to scroll to target block.');
            }
        }
        else if (action === 'focusBlock') {
            const index = Number.parseInt(String(payload.blockIndex ?? '-1'), 10);
            if (!Number.isFinite(index) || !focusBlock(index)) {
                throw new Error('Unable to focus target block.');
            }
        }
        else if (action === 'openBlockSource') {
            const index = Number.parseInt(String(payload.blockIndex ?? '-1'), 10);
            if (!Number.isFinite(index)) {
                throw new Error('A valid block index is required.');
            }
            openBlockSource(index);
        }
        else if (action === 'closeBlockSource') {
            const index = Number.parseInt(String(payload.blockIndex ?? '-1'), 10);
            if (!Number.isFinite(index)) {
                throw new Error('A valid block index is required.');
            }
            closeBlockSource(index, payload.commit !== false);
        }
        else if (action === 'undoState') {
            if (!undoLastFsmTransition()) {
                throw new Error('No FSM transition available to undo.');
            }
        }
        else if (action === 'undoDocument') {
            commitOpenSourcesForHistory();
            if (!performGlobalUndo()) {
                throw new Error('No document edit available to undo.');
            }
        }
        else if (action === 'redoDocument') {
            commitOpenSourcesForHistory();
            if (!performGlobalRedo()) {
                throw new Error('No document edit available to redo.');
            }
        }
        else {
            throw new Error(`Unknown surface action: ${action}`);
        }
        await withAnimationFrame();
        return captureSurfaceSnapshot(payload);
    }
    return { captureSurfaceSnapshot, runSurfaceAction };
}
