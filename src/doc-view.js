import { parseSourceBlocks, splitClassNames } from '../vscode-extension/media/doc-pipeline.js';
function escapeHtml(value) {
    return String(value || '')
        .replace(/&/g, '&amp;')
        .replace(/</g, '&lt;')
        .replace(/>/g, '&gt;')
        .replace(/"/g, '&quot;')
        .replace(/'/g, '&#39;');
}
function escapeStyleTagContent(value) {
    return String(value).replace(/<\/(style)/gi, '<\\/$1');
}
function toStringItems(items) {
    if (!Array.isArray(items))
        return [];
    return items.map((item) => String(item?.text || item || '').trim()).filter(Boolean);
}
function decorateRootTag(tag, block) {
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
function getDecoratedAttrs(block, baseClasses = []) {
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
function renderBlock(block) {
    const type = String(block?.type || 'paragraph').toLowerCase();
    if (type === 'style' || type === 'stylesheet') {
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
            const checked = Boolean(item?.checked);
            const text = escapeHtml(item?.text || item || '');
            return `<li><input type="checkbox" disabled ${checked ? 'checked' : ''} /><span${checked ? ' class="check-done"' : ''}>${text}</span></li>`;
        }).join('')}</ul>`;
    }
    if (type === 'quote') {
        const open = decorateRootTag('blockquote', block);
        return `${open}<p>${escapeHtml(block?.text || '')}</p></blockquote>`;
    }
    if (type === 'code') {
        const code = escapeHtml(block?.text || '');
        const open = decorateRootTag('pre', block);
        return `${open}${code}</pre>`;
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
    return `${open}${escapeHtml(block?.text || '')}</p>`;
}
export function renderDocumentViewHtml(document, { theme = 'auto', resolvedTheme = 'dark', appearance = null, effectiveCss = '', } = {}) {
    // Blocks are expected to be pre-validated and canonical.
    // The caller is responsible for parsing/validation upstream.
    // This function renders them as-is without roundtrip validation.
    const title = String(document?.title || document?.relativePath || 'Untitled Document');
    const parsedBlocks = parseSourceBlocks(String(document?.source || ''));
    const blocks = parsedBlocks.length > 0
        ? parsedBlocks
        : (Array.isArray(document?.blocks) ? document.blocks : []);
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
        for (const token of splitClassNames(block?.className)) {
            wrapClasses.push(token);
        }
        if (block?.id) {
            wrapClasses.push(String(block.id));
        }
        const wrapClassAttr = escapeHtml(Array.from(new Set(wrapClasses)).join(' '));
        return `<div class="${wrapClassAttr}" data-block-index="${index}" data-block-type="${type}"><div class="block-view" role="article" tabindex="0" aria-label="${aria}">${renderBlock(block)}</div></div>`;
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
