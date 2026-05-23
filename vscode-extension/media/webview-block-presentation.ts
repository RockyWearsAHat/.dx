interface PresentationBlock {
  type?: string;
  level?: number;
  language?: string;
}

interface BlockPresentationMeta {
  blockType: string;
  markdownToken: string;
  htmlOpenTag: string;
  htmlCloseTag: string;
}

function isBulletedListType(type: string): boolean {
  return type === 'list' || type === 'bulleted-list';
}

function isNumberedListType(type: string): boolean {
  return type === 'numbered-list';
}

export function getBlockPresentationMeta(block: PresentationBlock): BlockPresentationMeta {
  const type = String(block && block.type ? block.type : 'paragraph').toLowerCase();

  if (type === 'heading') {
    const level = Math.min(6, Math.max(1, Number(block && block.level ? block.level : 1)));
    return {
      blockType: type,
      markdownToken: '#'.repeat(level) + ' ',
      htmlOpenTag: `<h${level}>`,
      htmlCloseTag: `</h${level}>`,
    };
  }

  if (type === 'paragraph') {
    return {
      blockType: type,
      markdownToken: '',
      htmlOpenTag: '<p>',
      htmlCloseTag: '</p>',
    };
  }

  if (type === 'image') {
    return {
      blockType: type,
      markdownToken: '![]()',
      htmlOpenTag: '<figure>',
      htmlCloseTag: '</figure>',
    };
  }

  if (type === 'code') {
    const language = String(block && block.language ? block.language : '').trim();
    return {
      blockType: type,
      markdownToken: language ? `\`\`\`${language}` : '\`\`\`',
      htmlOpenTag: '<pre><code>',
      htmlCloseTag: '</code></pre>',
    };
  }

  if (type === 'quote') {
    return {
      blockType: type,
      markdownToken: '> ',
      htmlOpenTag: '<blockquote>',
      htmlCloseTag: '</blockquote>',
    };
  }

  if (isBulletedListType(type)) {
    return {
      blockType: type,
      markdownToken: '- ',
      htmlOpenTag: '<ul>',
      htmlCloseTag: '</ul>',
    };
  }

  if (isNumberedListType(type)) {
    return {
      blockType: type,
      markdownToken: '1. ',
      htmlOpenTag: '<ol>',
      htmlCloseTag: '</ol>',
    };
  }

  if (type === 'checklist') {
    return {
      blockType: type,
      markdownToken: '- [ ] ',
      htmlOpenTag: '<ul class="checklist">',
      htmlCloseTag: '</ul>',
    };
  }

  if (type === 'rule') {
    return {
      blockType: type,
      markdownToken: '---',
      htmlOpenTag: '<hr>',
      htmlCloseTag: '',
    };
  }

  if (type === 'style' || type === 'stylesheet') {
    return {
      blockType: type,
      markdownToken: '```css',
      htmlOpenTag: '<style>',
      htmlCloseTag: '</style>',
    };
  }

  return {
    blockType: type,
    markdownToken: '',
    htmlOpenTag: `<${type}>`,
    htmlCloseTag: `</${type}>`,
  };
}

export function applyBlockViewPresentation(view: HTMLElement | null, block: PresentationBlock): void {
  if (!view) return;
  const meta = getBlockPresentationMeta(block);
  view.dataset.blockType = meta.blockType;

  if (meta.markdownToken) {
    view.dataset.mdToken = meta.markdownToken;
  } else {
    delete view.dataset.mdToken;
  }

  if (meta.htmlOpenTag) {
    view.dataset.htmlOpenTag = meta.htmlOpenTag;
  } else {
    delete view.dataset.htmlOpenTag;
  }

  if (meta.htmlCloseTag) {
    view.dataset.htmlCloseTag = meta.htmlCloseTag;
  } else {
    delete view.dataset.htmlCloseTag;
  }
}
