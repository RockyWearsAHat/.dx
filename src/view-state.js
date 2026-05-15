import path from 'node:path';
import { readFile } from 'node:fs/promises';

function normalizeDocPath(value) {
  return String(value || '').replace(/\\/g, '/').replace(/^\/+/, '');
}

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

export async function readDocumentViewState(workspaceRoot, relativePath) {
  const root = String(workspaceRoot || '').trim();
  const rel = normalizeDocPath(relativePath);

  if (!root || !rel) {
    return null;
  }

  const viewStatePath = path.join(root, '.doc', 'view-state.json');

  try {
    const raw = await readFile(viewStatePath, 'utf8');
    const parsed = JSON.parse(raw);
    const documents = parsed && typeof parsed === 'object' && parsed.documents && typeof parsed.documents === 'object'
      ? parsed.documents
      : null;

    if (!documents) {
      return null;
    }

    const entry = documents[rel];

    if (!entry || typeof entry !== 'object') {
      return null;
    }

    const theme = String(entry.theme || 'auto');
    const resolvedTheme = String(entry.resolvedTheme || 'dark');
    const effectiveCss = String(entry.effectiveCss || '');
    const sourceText = String(entry.sourceText || '');

    return {
      theme: ['auto', 'light', 'dark'].includes(theme) ? theme : 'auto',
      resolvedTheme: ['light', 'dark'].includes(resolvedTheme) ? resolvedTheme : 'dark',
      appearance: sanitizeAppearance(entry.appearance),
      effectiveCss,
      sourceText,
    };
  } catch {
    return null;
  }
}