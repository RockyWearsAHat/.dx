function normalizeClassName(value) {
  return String(value || '')
    .split(/\s+/)
    .map((token) => token.trim())
    .filter(Boolean)
    .join(' ');
}

function normalizeText(value) {
  return String(value || '').trim();
}

function blockTextContent(block) {
  if (!block || typeof block !== 'object') {
    return '';
  }

  if (Array.isArray(block.items)) {
    return block.items.map((item) => {
      if (item && typeof item === 'object') {
        return String(item.text || '').trim();
      }

      return String(item || '').trim();
    }).join('\n');
  }

  if (block.type === 'image') {
    return normalizeText(block.alt || block.src || '');
  }

  return normalizeText(block.text || '');
}

function blockKind(block) {
  const type = normalizeText(block?.type || 'paragraph').toLowerCase();

  if (type === 'heading') return 'heading';
  if (type === 'paragraph') return 'text';
  if (type === 'quote') return 'quote';
  if (type === 'code') return 'code';
  if (type === 'image') return 'media';
  if (type === 'bulleted-list' || type === 'numbered-list' || type === 'checklist') return 'list';
  if (type === 'rule') return 'separator';
  return 'text';
}

function estimateVisualWeight(block) {
  const type = normalizeText(block?.type || 'paragraph').toLowerCase();

  if (type === 'heading') {
    const level = Math.max(1, Math.min(6, Number(block.level || 1)));
    return 0.95 - (level - 1) * 0.1;
  }

  if (type === 'image') return 0.85;
  if (type === 'code') return 0.7;
  if (type === 'quote') return 0.6;
  if (type === 'bulleted-list' || type === 'numbered-list' || type === 'checklist') return 0.55;
  if (type === 'rule') return 0.2;
  return 0.45;
}

function toVisualBlock(block, index) {
  const type = normalizeText(block?.type || 'paragraph').toLowerCase();
  const textContent = blockTextContent(block);
  const lines = textContent ? textContent.split('\n') : [];

  const visual = {
    index,
    id: normalizeText(block?.id || `block-${index + 1}`),
    type,
    kind: blockKind(block),
    className: normalizeClassName(block?.className || block?.class),
    textPreview: textContent.slice(0, 180),
    textLength: textContent.length,
    lineCount: lines.length,
    visualWeight: Number(estimateVisualWeight(block).toFixed(2)),
  };

  if (type === 'heading') {
    visual.level = Math.max(1, Math.min(6, Number(block.level || 1)));
  }

  if (type === 'code') {
    visual.language = normalizeText(block.language || '');
  }

  if (type === 'image') {
    visual.src = normalizeText(block.src || '');
    visual.alt = normalizeText(block.alt || '');
  }

  if (Array.isArray(block.items)) {
    visual.itemCount = block.items.length;
  }

  return visual;
}

function collectDocumentStats(blocks) {
  const stats = {
    totalBlocks: blocks.length,
    headings: 0,
    paragraphs: 0,
    lists: 0,
    codeBlocks: 0,
    images: 0,
    quotes: 0,
    separators: 0,
  };

  for (const block of blocks) {
    const type = normalizeText(block?.type || 'paragraph').toLowerCase();

    if (type === 'heading') stats.headings += 1;
    else if (type === 'paragraph') stats.paragraphs += 1;
    else if (type === 'bulleted-list' || type === 'numbered-list' || type === 'checklist') stats.lists += 1;
    else if (type === 'code') stats.codeBlocks += 1;
    else if (type === 'image') stats.images += 1;
    else if (type === 'quote') stats.quotes += 1;
    else if (type === 'rule') stats.separators += 1;
  }

  return stats;
}

function designIssues(document, visualBlocks, stats) {
  const issues = [];
  const headingCount = stats.headings;
  const firstBlock = visualBlocks[0] || null;
  const paragraphTextBlocks = visualBlocks.filter((block) => block.type === 'paragraph');
  const longParagraphs = paragraphTextBlocks.filter((block) => block.textLength > 320);
  const hasCode = stats.codeBlocks > 0;
  const hasLists = stats.lists > 0;
  const hasMedia = stats.images > 0;
  const hasQuote = stats.quotes > 0;

  if (!firstBlock || firstBlock.type !== 'heading' || Number(firstBlock.level || 0) !== 1) {
    issues.push({
      code: 'missing-primary-heading',
      severity: 'high',
      message: 'Document should begin with a level-1 heading for strong hierarchy and retrieval anchors.',
    });
  }

  if (headingCount < 2) {
    issues.push({
      code: 'flat-structure',
      severity: 'high',
      message: 'Add section headings to improve scanability and semantic segmentation.',
    });
  }

  if (longParagraphs.length > 0) {
    issues.push({
      code: 'dense-paragraphs',
      severity: 'medium',
      message: `${longParagraphs.length} paragraph block(s) are long; split into shorter blocks or lists for readability.`,
    });
  }

  if (!hasLists) {
    issues.push({
      code: 'no-scannable-lists',
      severity: 'medium',
      message: 'Consider adding at least one list block for quick scanning and actionability.',
    });
  }

  if (!hasCode && !hasMedia && !hasQuote) {
    issues.push({
      code: 'single-modality',
      severity: 'low',
      message: 'Document is text-only. Consider code, quote, or image blocks for visual rhythm.',
    });
  }

  if (!Array.isArray(document?.tags) || document.tags.length === 0) {
    issues.push({
      code: 'missing-tags',
      severity: 'low',
      message: 'Add tags to improve retrieval and categorization.',
    });
  }

  return issues;
}

function designRecommendations(stats) {
  const recommendations = [];

  if (stats.headings >= 2) {
    recommendations.push('Preserve heading hierarchy (H1 then H2/H3) to keep navigation and retrieval stable.');
  }

  if (stats.lists > 0) {
    recommendations.push('Keep key process/content points in lists to maintain scan speed.');
  }

  if (stats.codeBlocks > 0) {
    recommendations.push('Retain code blocks for exactness; pair each with one contextual sentence nearby.');
  }

  recommendations.push('Prefer concise paragraph blocks and explicit block ids/classes for targeted style refinement.');
  return recommendations;
}

function estimateBlockHeight(block) {
  const type = String(block.type || 'paragraph');
  const lineCount = Math.max(1, Number(block.lineCount || 1));
  const itemCount = Math.max(0, Number(block.itemCount || 0));

  if (type === 'heading') {
    const level = Math.max(1, Math.min(6, Number(block.level || 1)));
    return 56 - (level - 1) * 5;
  }

  if (type === 'paragraph') {
    return 26 + lineCount * 22;
  }

  if (type === 'quote') {
    return 40 + lineCount * 20;
  }

  if (type === 'code') {
    return 48 + lineCount * 20;
  }

  if (type === 'bulleted-list' || type === 'numbered-list' || type === 'checklist') {
    return 20 + Math.max(itemCount, lineCount) * 24;
  }

  if (type === 'image') {
    return 220;
  }

  if (type === 'rule') {
    return 22;
  }

  return 28 + lineCount * 20;
}

function buildSurfaceModel(visualBlocks) {
  const startX = 46;
  const contentWidth = 860;
  const defaultGap = 14;
  const headingGap = 18;
  const nodes = [];
  let cursorY = 34;

  for (const block of visualBlocks) {
    const height = estimateBlockHeight(block);
    const gapAfter = block.type === 'heading' ? headingGap : defaultGap;
    const node = {
      index: block.index,
      id: block.id,
      type: block.type,
      rect: {
        x: startX,
        y: cursorY,
        width: contentWidth,
        height,
      },
      visualWeight: block.visualWeight,
      textLength: block.textLength,
    };

    nodes.push(node);
    cursorY += height + gapAfter;
  }

  return {
    viewport: {
      width: 952,
      height: Math.max(720, cursorY + 48),
      contentPadding: {
        top: 34,
        left: 46,
        right: 46,
        bottom: 72,
      },
    },
    nodes,
  };
}

export function buildVisualModel(document) {
  const blocks = Array.isArray(document?.blocks) ? document.blocks : [];
  const visualBlocks = blocks.map((block, index) => toVisualBlock(block, index));
  const stats = collectDocumentStats(blocks);
  const issues = designIssues(document, visualBlocks, stats);
  const quality = Math.max(0, Math.min(100, 100 - (issues.filter((issue) => issue.severity === 'high').length * 18) - (issues.filter((issue) => issue.severity === 'medium').length * 10) - (issues.filter((issue) => issue.severity === 'low').length * 5)));
  const surface = buildSurfaceModel(visualBlocks);

  return {
    title: normalizeText(document?.title || ''),
    summary: normalizeText(document?.summary || ''),
    tags: Array.isArray(document?.tags) ? document.tags : [],
    meta: document?.meta && typeof document.meta === 'object' ? document.meta : {},
    stats,
    visualHierarchy: visualBlocks,
    headings: visualBlocks
      .filter((block) => block.type === 'heading')
      .map((block) => ({ id: block.id, level: block.level || 1, text: block.textPreview })),
    media: visualBlocks
      .filter((block) => block.type === 'image')
      .map((block) => ({ id: block.id, src: block.src || '', alt: block.alt || '' })),
    designQuality: {
      score: quality,
      issues,
      recommendations: designRecommendations(stats),
    },
    surface,
  };
}
