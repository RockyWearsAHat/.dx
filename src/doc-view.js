function escapeHtml(value) {
  return String(value || '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

function renderBlock(block) {
  const type = String(block?.type || 'paragraph').toLowerCase();

  if (type === 'heading') {
    const level = Math.max(1, Math.min(6, Number(block?.level || 1)));
    const text = escapeHtml(block?.text || '');
    return `<h${level}>${text}</h${level}>`;
  }

  if (type === 'bulleted-list' || type === 'list') {
    const items = Array.isArray(block?.items) ? block.items : [];
    return `<ul>${items.map((item) => `<li>${escapeHtml(item?.text || item || '')}</li>`).join('')}</ul>`;
  }

  if (type === 'numbered-list') {
    const items = Array.isArray(block?.items) ? block.items : [];
    return `<ol>${items.map((item) => `<li>${escapeHtml(item?.text || item || '')}</li>`).join('')}</ol>`;
  }

  if (type === 'checklist') {
    const items = Array.isArray(block?.items) ? block.items : [];
    return `<ul class="checklist">${items.map((item) => {
      const checked = Boolean(item?.checked);
      const text = escapeHtml(item?.text || item || '');
      return `<li><input type="checkbox" disabled ${checked ? 'checked' : ''} /><span>${text}</span></li>`;
    }).join('')}</ul>`;
  }

  if (type === 'quote') {
    return `<blockquote><p>${escapeHtml(block?.text || '')}</p></blockquote>`;
  }

  if (type === 'code') {
    const code = escapeHtml(block?.text || '');
    return `<pre><code>${code}</code></pre>`;
  }

  if (type === 'image') {
    const src = escapeHtml(block?.src || '');
    const alt = escapeHtml(block?.alt || '');
    const caption = alt ? `<figcaption>${alt}</figcaption>` : '';
    return `<figure><img src="${src}" alt="${alt}" loading="lazy" />${caption}</figure>`;
  }

  if (type === 'rule') {
    return '<hr />';
  }

  return `<p>${escapeHtml(block?.text || '')}</p>`;
}

export function renderDocumentViewHtml(document) {
  const title = String(document?.title || document?.relativePath || 'Untitled Document');
  const summary = String(document?.summary || '').trim();
  const tags = Array.isArray(document?.tags) ? document.tags : [];
  const blocks = Array.isArray(document?.blocks) ? document.blocks : [];

  const tagsMarkup = tags.length
    ? `<div class="tags">${tags.map((tag) => `<span class="tag">${escapeHtml(tag)}</span>`).join('')}</div>`
    : '';

  const summaryMarkup = summary ? `<p class="summary">${escapeHtml(summary)}</p>` : '';

  const blocksMarkup = blocks.map((block, index) => {
    const type = escapeHtml(block?.type || 'paragraph');
    return `<section class="block" data-index="${index}" data-type="${type}">${renderBlock(block)}</section>`;
  }).join('\n');

  return `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>${escapeHtml(title)}</title>
  <style>
    :root {
      --bg: #f4f6fb;
      --paper: #ffffff;
      --ink: #1c2431;
      --muted: #607086;
      --line: #d4ddeb;
    }
    @media (prefers-color-scheme: dark) {
      :root {
        --bg: #0f141d;
        --paper: #161d29;
        --ink: #ecf2fb;
        --muted: #9ba8ba;
        --line: #2a3549;
      }
    }
    html, body { margin: 0; background: var(--bg); color: var(--ink); }
    body { font: 16px/1.6 "Iowan Old Style", "Palatino Linotype", Palatino, serif; }
    main { max-width: 900px; margin: 28px auto; padding: 28px 34px 52px; background: var(--paper); border: 1px solid var(--line); border-radius: 14px; }
    h1, h2, h3, h4, h5, h6 { line-height: 1.2; margin: 0.1em 0 0.5em; }
    p, ul, ol, pre, blockquote, figure { margin: 0 0 0.9rem; }
    .summary { color: var(--muted); margin-top: -0.2rem; }
    .tags { display: flex; gap: 0.4rem; flex-wrap: wrap; margin-bottom: 1rem; }
    .tag { font: 12px/1.4 "Avenir Next", "Segoe UI", sans-serif; border: 1px solid var(--line); border-radius: 999px; padding: 0.08rem 0.5rem; color: var(--muted); }
    blockquote { border-left: 3px solid var(--line); color: var(--muted); padding-left: 0.8rem; }
    pre { border: 1px solid var(--line); border-radius: 8px; padding: 12px; overflow-x: auto; background: color-mix(in srgb, var(--paper) 92%, var(--bg)); }
    img { max-width: 100%; border: 1px solid var(--line); border-radius: 8px; }
    .checklist { list-style: none; padding: 0; }
    .checklist li { display: flex; gap: 0.5rem; align-items: baseline; }
  </style>
</head>
<body>
  <main data-doc-path="${escapeHtml(document?.relativePath || '')}">
    <h1>${escapeHtml(title)}</h1>
    ${summaryMarkup}
    ${tagsMarkup}
    ${blocksMarkup}
  </main>
</body>
</html>`;
}