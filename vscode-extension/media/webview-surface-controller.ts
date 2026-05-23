// webview-surface-controller.ts
// Source: edit this file, then run `npm run build:surface` to regenerate the
// compiled JS that the webview loads at runtime.

// ---------------------------------------------------------------------------
// Shared shape types
// ---------------------------------------------------------------------------

export interface DocBlock {
  id?: string;
  type?: string;
  items?: Array<{ text?: string } | string>;
  text?: string;
  alt?: string;
  src?: string;
}

export interface DocModel {
  blocks: DocBlock[];
}

export interface BlockRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface BlockSnapshot {
  index: number;
  id: string;
  type: string;
  rect: BlockRect;
  inViewport: boolean;
  sourceOpen: boolean;
  text?: string;
  style?: {
    fontSize: string;
    lineHeight: string;
    color: string;
    backgroundColor: string;
  };
}

export interface BlockTypeCounts {
  total: number;
  headings: number;
  paragraphs: number;
  lists: number;
  codeBlocks: number;
  images: number;
  quotes: number;
  rules: number;
}

export interface FsmViewState {
  documentState: string;
  saveState: string;
  historyLength: number;
  lastTransition: string | number | boolean | null | undefined | object;
}

export interface DocumentHistoryDepths {
  undoDepth: number;
  redoDepth: number;
}

export interface SurfaceSnapshotOptions {
  includeText?: boolean;
  includeStyles?: boolean;
}

export interface SurfaceSnapshot {
  documentPath: string;
  capturedAt: string;
  theme: string;
  resolvedTheme: string;
  editMode: boolean;
  fsm: FsmViewState;
  documentHistory: DocumentHistoryDepths;
  viewport: {
    width: number;
    height: number;
    scrollX: number;
    scrollY: number;
  };
  page: BlockRect | null;
  focusedBlockIndex: number | null;
  blockCounts: BlockTypeCounts;
  blocks: BlockSnapshot[];
}

export interface SurfaceActionPayload extends SurfaceSnapshotOptions {
  action?: string;
  enabled?: boolean;
  deltaY?: number;
  top?: number;
  blockIndex?: number | string;
  behavior?: string;
  commit?: boolean;
}

// ---------------------------------------------------------------------------
// Dependency contract — every dep is injected; none are globals
// ---------------------------------------------------------------------------

export interface SurfaceControllerOptions {
  getDocModel(): DocModel | null;
  getCurrentDocPath(): string;
  getFsmViewState(): FsmViewState;
  getDocumentHistoryDepths(): DocumentHistoryDepths;
  getCurrentTheme(): string;
  getResolvedTheme(): string;
  isEditModeEnabled(): boolean;
  getFocusedBlockIndex(): number | null;
  summarizeBlockTypeCounts(): BlockTypeCounts;
  setEditMode(enabled: boolean): void;
  openBlockSource(index: number): void;
  closeBlockSource(index: number, commit: boolean): void;
  undoLastFsmTransition(): boolean;
  commitOpenSourcesForHistory(): void;
  performGlobalUndo(): string | null;
  performGlobalRedo(): string | null;
}

// ---------------------------------------------------------------------------
// Public controller interface
// ---------------------------------------------------------------------------

export interface SurfaceController {
  captureSurfaceSnapshot(options?: SurfaceSnapshotOptions): SurfaceSnapshot;
  runSurfaceAction(payload?: SurfaceActionPayload): Promise<SurfaceSnapshot>;
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

export function createSurfaceController(options: SurfaceControllerOptions): SurfaceController {
  const {
    getDocModel,
    getCurrentDocPath,
    getFsmViewState,
    getDocumentHistoryDepths,
    getCurrentTheme,
    getResolvedTheme,
    isEditModeEnabled,
    getFocusedBlockIndex,
    summarizeBlockTypeCounts,
    setEditMode,
    openBlockSource,
    closeBlockSource,
    undoLastFsmTransition,
    commitOpenSourcesForHistory,
    performGlobalUndo,
    performGlobalRedo,
  } = options;

  // --- Private DOM helpers ------------------------------------------------

  function findBlockWrap(index: number): Element | null {
    return document.querySelector(`.block-wrap[data-block-index="${index}"]`);
  }

  function focusBlock(index: number): boolean {
    const wrap = findBlockWrap(index);
    if (!wrap) return false;
    const view = wrap.querySelector('.block-view');
    if (!(view instanceof HTMLElement)) return false;
    view.focus();
    return true;
  }

  function scrollToBlock(index: number, behavior: string = 'instant'): boolean {
    const wrap = findBlockWrap(index);
    if (!wrap) return false;
    wrap.scrollIntoView({
      block: 'center',
      behavior: behavior === 'smooth' ? 'smooth' : 'auto',
    });
    return true;
  }

  function withAnimationFrame(): Promise<void> {
    return new Promise<void>((resolve) => {
      requestAnimationFrame(() => resolve());
    });
  }

  // --- Public surface methods ---------------------------------------------

  function captureSurfaceSnapshot(opts: SurfaceSnapshotOptions = {}): SurfaceSnapshot {
    const includeText = opts.includeText !== false;
    const includeStyles = Boolean(opts.includeStyles);
    const page = document.querySelector('.page');
    const pageRect = page ? page.getBoundingClientRect() : null;
    const wraps = Array.from(document.querySelectorAll('.block-wrap'));
    const docModel = getDocModel();

    const blocks: BlockSnapshot[] = wraps.map((wrap) => {
      const index = Number.parseInt((wrap as HTMLElement).dataset['blockIndex'] ?? '', 10);
      const view = wrap.querySelector('.block-view');
      const srcWrap = wrap.querySelector('.block-src-wrapper');
      const rect = wrap.getBoundingClientRect();
      const block =
        Number.isFinite(index) && docModel && Array.isArray(docModel.blocks)
          ? (docModel.blocks[index] ?? null)
          : null;
      const computed = view && includeStyles ? window.getComputedStyle(view) : null;

      return {
        index: Number.isFinite(index) ? index : -1,
        id: block?.id ?? '',
        type: block?.type ?? 'string | number | boolean | null | undefined | object',
        rect: {
          x: Math.round(rect.x),
          y: Math.round(rect.y),
          width: Math.round(rect.width),
          height: Math.round(rect.height),
        },
        inViewport: rect.bottom > 0 && rect.top < window.innerHeight,
        sourceOpen: Boolean(srcWrap && (srcWrap as HTMLElement).style.display === 'block'),
        text: includeText
          ? block
            ? Array.isArray(block.items)
              ? block.items
                  .map((item) =>
                    typeof item === 'object' && item
                      ? (item as { text?: string }).text ?? ''
                      : String(item),
                  )
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

  async function runSurfaceAction(payload: SurfaceActionPayload = {}): Promise<SurfaceSnapshot> {
    const action = String(payload.action ?? '').trim();

    if (!action) {
      throw new Error('Surface action is required.');
    }

    if (action === 'setEditMode') {
      setEditMode(Boolean(payload.enabled));
    } else if (action === 'scrollBy') {
      const deltaY = Number(payload.deltaY ?? 0);
      window.scrollBy({ top: Number.isFinite(deltaY) ? deltaY : 0, behavior: 'auto' });
    } else if (action === 'scrollTo') {
      const top = Number(payload.top ?? 0);
      window.scrollTo({ top: Number.isFinite(top) ? top : 0, behavior: 'auto' });
    } else if (action === 'scrollToBlock') {
      const index = Number.parseInt(String(payload.blockIndex ?? '-1'), 10);
      if (!Number.isFinite(index) || !scrollToBlock(index, String(payload.behavior ?? 'instant'))) {
        throw new Error('Unable to scroll to target block.');
      }
    } else if (action === 'focusBlock') {
      const index = Number.parseInt(String(payload.blockIndex ?? '-1'), 10);
      if (!Number.isFinite(index) || !focusBlock(index)) {
        throw new Error('Unable to focus target block.');
      }
    } else if (action === 'openBlockSource') {
      const index = Number.parseInt(String(payload.blockIndex ?? '-1'), 10);
      if (!Number.isFinite(index)) {
        throw new Error('A valid block index is required.');
      }
      openBlockSource(index);
    } else if (action === 'closeBlockSource') {
      const index = Number.parseInt(String(payload.blockIndex ?? '-1'), 10);
      if (!Number.isFinite(index)) {
        throw new Error('A valid block index is required.');
      }
      closeBlockSource(index, payload.commit !== false);
    } else if (action === 'undoState') {
      if (!undoLastFsmTransition()) {
        throw new Error('No FSM transition available to undo.');
      }
    } else if (action === 'undoDocument') {
      commitOpenSourcesForHistory();
      if (!performGlobalUndo()) {
        throw new Error('No document edit available to undo.');
      }
    } else if (action === 'redoDocument') {
      commitOpenSourcesForHistory();
      if (!performGlobalRedo()) {
        throw new Error('No document edit available to redo.');
      }
    } else {
      throw new Error(`Unknown surface action: ${action}`);
    }

    await withAnimationFrame();
    return captureSurfaceSnapshot(payload);
  }

  return { captureSurfaceSnapshot, runSurfaceAction };
}
