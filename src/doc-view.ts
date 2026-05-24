import { parseSourceBlocks, splitClassNames } from '../vscode-extension/media/doc-pipeline.js';
import type { ChecklistItem, PipelineBlock } from '../vscode-extension/media/doc-pipeline.js';
import type { DocumentViewState } from './view-state.js';

interface RenderDocument {
  title?: string;
  relativePath?: string;
  source?: string;
  blocks?: PipelineBlock[];
}

interface RenderOptions {
  theme?: DocumentViewState['theme'];
  resolvedTheme?: DocumentViewState['resolvedTheme'];
  appearance?: DocumentViewState['appearance'] | null;
  effectiveCss?: string;
}

type TemplateValues = Record<string, string>;

function escapeHtml(value: string | number | boolean | null | undefined | object): string {
  return String(value || '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

function escapeStyleTagContent(value: string | number | boolean | null | undefined | object): string {
  return String(value).replace(/<\/(style)/gi, '<\\/$1');
}

function sanitizeRichMarkup(value: string | number | boolean | null | undefined | object): string {
  let text = String(value || '');
  text = text.replace(/<script[\s\S]*?>[\s\S]*?<\/script>/gi, '');
  text = text.replace(/\son[a-z]+\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]+)/gi, '');
  text = text.replace(/\s(?:href|src|xlink:href)\s*=\s*(['"])\s*javascript:[\s\S]*?\1/gi, '');
  text = text.replace(/\s(?:href|src|xlink:href)\s*=\s*javascript:[^\s>]+/gi, '');
  return text;
}

function extractSvgMarkup(value: string | number | boolean | null | undefined | object): string {
  const text = String(value || '');
  const match = /<svg[\s\S]*?<\/svg>/i.exec(text);
  return match ? match[0] : '';
}

function interpolateTemplateText(text: string, values: TemplateValues): string {
  return String(text || '').replace(/\{\{\s*([a-zA-Z0-9_.-]+)\s*\}\}/g, (_full, key) => {
    if (!Object.prototype.hasOwnProperty.call(values, key)) {
      return '';
    }

    return values[key] as string;
  });
}

function collectTemplateValues(blocks: PipelineBlock[]): TemplateValues {
  const values: TemplateValues = {};

  for (const block of blocks) {
    if (String(block?.type || '').toLowerCase() !== 'script') {
      continue;
    }

    const scriptType = String(block?.scriptType || '').trim().toLowerCase();

    if (scriptType && scriptType !== 'application/json') {
      continue;
    }

    const body = String(block?.text || '').trim();
    if (!body) {
      continue;
    }

    try {
      const parsed = JSON.parse(body) as Record<string, unknown>;

      for (const [key, value] of Object.entries(parsed)) {
        if (value === null || value === undefined || typeof value === 'object') {
          continue;
        }

        values[key] = String(value);
      }
    } catch {
    }
  }

  return values;
}

function toStringItems(items: Array<string | ChecklistItem> | undefined): string[] {
  if (!Array.isArray(items)) return [];
  return items.map((item) => {
    if (typeof item === 'object' && item !== null) {
      return String(item.text || '').trim();
    }
    return String(item || '').trim();
  }).filter(Boolean);
}

function decorateRootTag(tag: string, block: PipelineBlock | undefined) {
  const attrs = [];
  const classes = splitClassNames(block?.className);

  if (block?.id) {
    attrs.push(`id="${escapeHtml(block.id)}"`);
    attrs.push(`data-block-id="${escapeHtml(block.id)}"`);
  }

  if (classes.length > 0) {
    const unique = Array.from(new Set(classes));
    attrs.push(`class="${escapeHtml(unique.join(' '))}"`);
  }

  return attrs.length > 0 ? `<${tag} ${attrs.join(' ')}>` : `<${tag}>`;
}

function getDecoratedAttrs(block: PipelineBlock | undefined, baseClasses: string[] = []): string {
  const classes = [...baseClasses, ...splitClassNames(block?.className)];
  const attrs = [];

  if (block?.id) {
    attrs.push(`id="${escapeHtml(block.id)}"`);
    attrs.push(`data-block-id="${escapeHtml(block.id)}"`);
  }

  if (classes.length > 0) {
    const unique = Array.from(new Set(classes.filter(Boolean)));
    if (unique.length > 0) {
      attrs.push(`class="${escapeHtml(unique.join(' '))}"`);
    }
  }

  return attrs.length > 0 ? ` ${attrs.join(' ')}` : '';
}

function renderBlock(block: PipelineBlock, templateValues: TemplateValues): string {
  const type = String(block?.type || 'paragraph').toLowerCase();

  if (type === 'style' || type === 'stylesheet' || type === 'script') {
    return '';
  }

  if (type === 'heading') {
    const level = Math.max(1, Math.min(6, Number(block?.level || 1)));
    const text = escapeHtml(block?.text || '');
    const open = decorateRootTag(`h${level}`, block);
    return `${open}${text}</h${level}>`;
  }

  if (type === 'bulleted-list' || type === 'list') {
    const items = toStringItems(block?.items);
    const open = decorateRootTag('ul', block);
    return `${open}${items.map((item) => `<li>${escapeHtml(item)}</li>`).join('')}</ul>`;
  }

  if (type === 'numbered-list') {
    const items = toStringItems(block?.items);
    const open = decorateRootTag('ol', block);
    return `${open}${items.map((item) => `<li>${escapeHtml(item)}</li>`).join('')}</ol>`;
  }

  if (type === 'checklist') {
    const items = Array.isArray(block?.items) ? block.items : [];
    const attrs = getDecoratedAttrs(block, ['checklist-wrap']);
    return `<ul${attrs}>${items.map((item) => {
      const normalized = typeof item === 'object' && item !== null
        ? { checked: Boolean(item.checked), text: String(item.text || '') }
        : { checked: false, text: String(item || '') };
      const checked = normalized.checked;
      const text = escapeHtml(normalized.text);
      return `<li><input type="checkbox" disabled ${checked ? 'checked' : ''} /><span${checked ? ' class="check-done"' : ''}>${text}</span></li>`;
    }).join('')}</ul>`;
  }

  if (type === 'quote') {
    const open = decorateRootTag('blockquote', block);
    return `${open}<p>${escapeHtml(block?.text || '')}</p></blockquote>`;
  }

  if (type === 'code') {
    const language = String(block?.language || '').trim().toLowerCase();
    const rawText = interpolateTemplateText(String(block?.text || ''), templateValues);

    if (language === 'svg') {
      const svgMarkup = extractSvgMarkup(rawText);
      if (svgMarkup) {
        const attrs = getDecoratedAttrs(block, ['svg-wrap']);
        return `<div${attrs}>${sanitizeRichMarkup(svgMarkup)}</div>`;
      }
    }

    if (language === 'html') {
      const attrs = getDecoratedAttrs(block, ['html-wrap']);
      return `<div${attrs}>${sanitizeRichMarkup(rawText)}</div>`;
    }

    const code = escapeHtml(rawText);
    const open = decorateRootTag('pre', block);
    return `${open}${code}</pre>`;
  }

  if (type === 'svg') {
    const svgMarkup = extractSvgMarkup(interpolateTemplateText(String(block?.text || ''), templateValues));
    if (svgMarkup) {
      const attrs = getDecoratedAttrs(block, ['svg-wrap']);
      return `<div${attrs}>${sanitizeRichMarkup(svgMarkup)}</div>`;
    }

    const open = decorateRootTag('pre', block);
    return `${open}${escapeHtml(block?.text || '')}</pre>`;
  }

  if (type === 'html') {
    const attrs = getDecoratedAttrs(block, ['html-wrap']);
    const html = interpolateTemplateText(String(block?.text || ''), templateValues);
    return `<div${attrs}>${sanitizeRichMarkup(html)}</div>`;
  }

  if (type === 'graph' || type === 'mermaid') {
    const text = interpolateTemplateText(String(block?.text || ''), templateValues);
    const svgMarkup = extractSvgMarkup(text);

    if (svgMarkup) {
      const attrs = getDecoratedAttrs(block, ['graph-wrap']);
      return `<div${attrs}>${sanitizeRichMarkup(svgMarkup)}</div>`;
    }

    const open = decorateRootTag('pre', block);
    return `${open}${escapeHtml(text)}</pre>`;
  }

  if (type === 'image') {
    const src = escapeHtml(block?.src || '');
    const alt = escapeHtml(block?.alt || '');
    const caption = alt ? `<figcaption>${alt}</figcaption>` : '';
    const attrs = getDecoratedAttrs(block, ['image-wrap']);
    return `<figure${attrs}><img src="${src}" alt="${alt}" loading="lazy" />${caption}</figure>`;
  }

  if (type === 'rule') {
    const attrs = getDecoratedAttrs(block);
    return `<hr${attrs} />`;
  }

  const open = decorateRootTag('p', block);
  return `${open}${escapeHtml(interpolateTemplateText(String(block?.text || ''), templateValues))}</p>`;
}

export function renderDocumentViewHtml(document: RenderDocument, {
  theme = 'auto',
  resolvedTheme = 'dark',
  appearance = null,
  effectiveCss = '',
}: RenderOptions = {}) {
  // Blocks are expected to be pre-validated and canonical.
  // The caller is responsible for parsing/validation upstream.
  // This function renders them as-is without roundtrip validation.
  const title = String(document?.title || document?.relativePath || 'Untitled Document');
  const parsedBlocks = parseSourceBlocks(String(document?.source || ''));
  const blocks: PipelineBlock[] = parsedBlocks.length > 0
    ? parsedBlocks
    : (Array.isArray(document?.blocks) ? document.blocks : []);
  const templateValues = collectTemplateValues(blocks);
  const styleBlocks = blocks
    .filter((block) => String(block?.type || '').toLowerCase() === 'style')
    .map((block) => String(block?.text || '').trim())
    .filter(Boolean);
  const stylesheetLinks = blocks
    .filter((block) => String(block?.type || '').toLowerCase() === 'stylesheet')
    .map((block) => String(block?.href || block?.src || '').trim())
    .filter(Boolean);
  const documentCss = String(effectiveCss || '').trim();
  const paper = String(appearance?.paper || 'white');
  const density = String(appearance?.density || 'comfortable');
  const scaleRaw = Number(appearance?.scale);
  const scale = Number.isFinite(scaleRaw) ? Math.min(115, Math.max(90, Math.round(scaleRaw))) : 100;

  const blocksMarkup = blocks.map((block, index) => {
    const type = escapeHtml(block?.type || 'paragraph');
    const aria = escapeHtml(`Block ${index + 1}: ${type}`);
    const wrapClasses = ['block-wrap'];
    const hidden = Boolean(block?.hidden);
    for (const token of splitClassNames(block.className)) {
      wrapClasses.push(token);
    }
    if (block?.id) {
      wrapClasses.push(String(block.id));
    }
    if (hidden) {
      wrapClasses.push('is-hidden');
    }
    const wrapClassAttr = escapeHtml(Array.from(new Set(wrapClasses)).join(' '));
    const hiddenAttrs = hidden ? ' hidden aria-hidden="true" data-block-hidden="true"' : '';
    return `<div class="${wrapClassAttr}" data-block-index="${index}" data-block-type="${type}"${hiddenAttrs}><div class="block-view" role="article" tabindex="0" aria-label="${aria}">${renderBlock(block, templateValues)}</div></div>`;
  }).join('\n');
  const styleMarkup = styleBlocks
    .map((css, index) => `<style data-doc-style="${index + 1}">${escapeStyleTagContent(css)}</style>`)
    .join('\n  ');
  const stylesheetMarkup = stylesheetLinks
    .map((href) => `<link rel="stylesheet" href="${escapeHtml(href)}" />`)
    .join('\n  ');

  return `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>${escapeHtml(title)}</title>
  ${stylesheetMarkup}
  ${styleMarkup}
  <style>${documentCss}</style>
</head>
<body data-theme="${escapeHtml(theme)}" data-resolved-theme="${escapeHtml(resolvedTheme)}" data-paper="${escapeHtml(paper)}" data-density="${escapeHtml(density)}" data-doc-default-style="off">
  <main class="page" data-doc-path="${escapeHtml(document?.relativePath || '')}" data-edit-mode="false" data-ready="true">
    <div id="blocks">${blocksMarkup}</div>
  </main>
  <style>:root{--editor-scale:${escapeHtml(String(scale / 100))};}</style>
</body>
</html>`;
}