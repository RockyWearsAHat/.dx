export interface DocumentViewAppearance {
  paper: 'white' | 'cream' | 'slate';
  density: 'comfortable' | 'compact';
  scale: number;
}

export interface DocumentViewViewport {
  width: number | null;
  height: number | null;
  pixelRatio: number | null;
  zoomLevel: number;
  zoomFactor: number;
}

export interface DocumentViewState {
  theme: 'auto' | 'light' | 'dark';
  resolvedTheme: 'light' | 'dark';
  appearance: DocumentViewAppearance;
  viewport: DocumentViewViewport;
  effectiveCss: string;
  sourceText: string;
}

function sanitizeAppearance(input: string | number | boolean | null | undefined | object): DocumentViewAppearance {
  const appearance = (input && typeof input === 'object' ? input : {}) as Record<string, string | number | boolean | null | undefined | object>;
  const paperRaw = String(appearance.paper || 'white');
  const densityRaw = String(appearance.density || 'comfortable');
  const scaleRaw = Number(appearance.scale);
  const scale = Number.isFinite(scaleRaw) ? Math.min(115, Math.max(90, Math.round(scaleRaw))) : 100;
  const paper: DocumentViewAppearance['paper'] = ['white', 'cream', 'slate'].includes(paperRaw)
    ? (paperRaw as DocumentViewAppearance['paper'])
    : 'white';
  const density: DocumentViewAppearance['density'] = ['comfortable', 'compact'].includes(densityRaw)
    ? (densityRaw as DocumentViewAppearance['density'])
    : 'comfortable';

  return {
    paper,
    density,
    scale,
  };
}

function sanitizeViewport(input: string | number | boolean | null | undefined | object): DocumentViewViewport {
  const viewport = (input && typeof input === 'object' ? input : {}) as Record<string, string | number | boolean | null | undefined | object>;
  const widthRaw = Number(viewport.width);
  const heightRaw = Number(viewport.height);
  const pixelRatioRaw = Number(viewport.pixelRatio);
  const zoomLevelRaw = Number(viewport.zoomLevel);
  const zoomFactorRaw = Number(viewport.zoomFactor);

  const width = Number.isFinite(widthRaw) && widthRaw > 0 ? Math.round(widthRaw) : null;
  const height = Number.isFinite(heightRaw) && heightRaw > 0 ? Math.round(heightRaw) : null;
  const pixelRatio = Number.isFinite(pixelRatioRaw) && pixelRatioRaw > 0 ? pixelRatioRaw : null;
  const zoomLevel = Number.isFinite(zoomLevelRaw) ? zoomLevelRaw : 0;
  const zoomFactor = Number.isFinite(zoomFactorRaw) && zoomFactorRaw > 0
    ? zoomFactorRaw
    : Math.pow(1.2, zoomLevel);

  return {
    width,
    height,
    pixelRatio,
    zoomLevel,
    zoomFactor,
  };
}

export function normalizeDocumentViewState(input: string | number | boolean | null | undefined | object): DocumentViewState {
  const entry = (input && typeof input === 'object' ? input : {}) as Record<string, string | number | boolean | null | undefined | object>;
  const themeRaw = String(entry.theme || 'auto');
  const resolvedThemeRaw = String(entry.resolvedTheme || 'dark');
  const sourceText = String(entry.sourceText || '');
  const effectiveCss = String(entry.effectiveCss || '');
  const theme: DocumentViewState['theme'] = ['auto', 'light', 'dark'].includes(themeRaw)
    ? (themeRaw as DocumentViewState['theme'])
    : 'auto';
  const resolvedTheme: DocumentViewState['resolvedTheme'] = ['light', 'dark'].includes(resolvedThemeRaw)
    ? (resolvedThemeRaw as DocumentViewState['resolvedTheme'])
    : 'dark';

  return {
    theme,
    resolvedTheme,
    appearance: sanitizeAppearance(entry.appearance),
    viewport: sanitizeViewport(entry.viewport),
    effectiveCss,
    sourceText,
  };
}

export function mergeDocumentViewState(baseState: string | number | boolean | null | undefined | object, patchState: string | number | boolean | null | undefined | object): DocumentViewState {
  const base = normalizeDocumentViewState(baseState);
  const patch = (patchState && typeof patchState === 'object' ? patchState : {}) as Record<string, string | number | boolean | null | undefined | object>;

  const merged = {
    theme: patch.theme ?? base.theme,
    resolvedTheme: patch.resolvedTheme ?? base.resolvedTheme,
    appearance: {
      ...base.appearance,
      ...(patch.appearance && typeof patch.appearance === 'object' ? patch.appearance : {}),
    },
    viewport: {
      ...base.viewport,
      ...(patch.viewport && typeof patch.viewport === 'object' ? patch.viewport : {}),
    },
    effectiveCss: patch.effectiveCss ?? base.effectiveCss,
    sourceText: patch.sourceText ?? base.sourceText,
  };

  return normalizeDocumentViewState(merged);
}

export function readDocumentViewState(db: { prepare: (query: string) => { get: (id: number) => { view_state_json?: string } | undefined } } | null | undefined, documentId: string | number | boolean | null | undefined | object): DocumentViewState | null {
  if (!db || !Number.isFinite(Number(documentId))) {
    return null;
  }

  try {
    const row = db.prepare(`SELECT view_state_json FROM documents WHERE id = ?`).get(Number(documentId));
    if (!row || !row.view_state_json) {
      return null;
    }

    const entry = JSON.parse(row.view_state_json);
    if (!entry || typeof entry !== 'object') {
      return null;
    }
    return normalizeDocumentViewState(entry);
  } catch {
    return null;
  }
}