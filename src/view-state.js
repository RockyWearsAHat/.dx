function sanitizeAppearance(input) {
    const appearance = input && typeof input === 'object' ? input : {};
    const paper = String(appearance.paper || 'white');
    const density = String(appearance.density || 'comfortable');
    const scaleRaw = Number(appearance.scale);
    const scale = Number.isFinite(scaleRaw) ? Math.min(115, Math.max(90, Math.round(scaleRaw))) : 100;
    return {
        paper: ['white', 'cream', 'slate'].includes(paper) ? paper : 'white',
        density: ['comfortable', 'compact'].includes(density) ? density : 'comfortable',
        scale,
    };
}
function sanitizeViewport(input) {
    const viewport = input && typeof input === 'object' ? input : {};
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
export function normalizeDocumentViewState(input) {
    const entry = input && typeof input === 'object' ? input : {};
    const theme = String(entry.theme || 'auto');
    const resolvedTheme = String(entry.resolvedTheme || 'dark');
    const sourceText = String(entry.sourceText || '');
    const effectiveCss = String(entry.effectiveCss || '');
    return {
        theme: ['auto', 'light', 'dark'].includes(theme) ? theme : 'auto',
        resolvedTheme: ['light', 'dark'].includes(resolvedTheme) ? resolvedTheme : 'dark',
        appearance: sanitizeAppearance(entry.appearance),
        viewport: sanitizeViewport(entry.viewport),
        effectiveCss,
        sourceText,
    };
}
export function mergeDocumentViewState(baseState, patchState) {
    const base = normalizeDocumentViewState(baseState);
    const patch = patchState && typeof patchState === 'object' ? patchState : {};
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
export function readDocumentViewState(db, documentId) {
    if (!db || !Number.isFinite(Number(documentId))) {
        return null;
    }
    try {
        const row = db.prepare(`SELECT view_state_json FROM documents WHERE id = ?`).get(documentId);
        if (!row || !row.view_state_json) {
            return null;
        }
        const entry = JSON.parse(row.view_state_json);
        if (!entry || typeof entry !== 'object') {
            return null;
        }
        return normalizeDocumentViewState(entry);
    }
    catch {
        return null;
    }
}
