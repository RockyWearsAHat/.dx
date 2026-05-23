function normalizeLineEndings(value: string | number | boolean | null | undefined | object): string {
  return String(value || '').replace(/\r\n/g, '\n').trimEnd();
}

function stripTagWrapper(source: string, tagName: string): string {
  const pattern = new RegExp(`^<${tagName}>([\\s\\S]*)<\\/${tagName}>$`, 'i');
  const match = pattern.exec(source.trim());
  return match ? String(match[1] || '').trim() : '';
}

function parseMarkdownChecklist(lines: string[]): string[] | null {
  const items: string[] = [];

  for (const line of lines) {
    const match = /^\s*[-*]\s+\[([ xX])]\s+(.*)$/.exec(line);
    if (!match) {
      return null;
    }
    const checked = String(match[1] || '').toLowerCase() === 'x';
    const text = String(match[2] || '').trim();
    if (!text) {
      continue;
    }
    items.push(`[${checked ? 'x' : ' '}] ${text}`);
  }

  return items.length > 0 ? items : null;
}

function parseMarkdownBulleted(lines: string[]): string[] | null {
  const items: string[] = [];

  for (const line of lines) {
    const match = /^\s*[-*]\s+(.*)$/.exec(line);
    if (!match) {
      return null;
    }
    const text = String(match[1] || '').trim();
    if (!text) {
      continue;
    }
    items.push(`- ${text}`);
  }

  return items.length > 0 ? items : null;
}

function parseMarkdownNumbered(lines: string[]): string[] | null {
  const items: string[] = [];

  for (const line of lines) {
    const match = /^\s*\d+[.)]\s+(.*)$/.exec(line);
    if (!match) {
      return null;
    }
    const text = String(match[1] || '').trim();
    if (!text) {
      continue;
    }
    items.push(`1. ${text}`);
  }

  return items.length > 0 ? items : null;
}

function normalizeCodeFence(source: string): string | null {
  const match = /^```([a-zA-Z0-9_-]*)\n([\s\S]*?)\n```$/.exec(source.trim());
  if (!match) {
    return null;
  }

  const language = String(match[1] || '').trim();
  const body = String(match[2] || '').trimEnd();
  const header = language ? `::code language=${language}` : '::code';
  return `${header}\n${body}\n::end`;
}

function normalizeHtmlLike(source: string): string | null {
  const trimmed = source.trim();

  const headingMatch = /^<h([1-6])>([\s\S]*?)<\/h\1>$/i.exec(trimmed);
  if (headingMatch) {
    const level = Number(headingMatch[1]);
    const text = String(headingMatch[2] || '').trim();
    return `::heading level=${level}\n${text}\n::end`;
  }

  if (/^<hr\s*\/?>(?:\s*)$/i.test(trimmed)) {
    return '::rule';
  }

  const paragraphText = stripTagWrapper(trimmed, 'p');
  if (paragraphText) {
    return `::paragraph\n${paragraphText}\n::end`;
  }

  const quoteText = stripTagWrapper(trimmed, 'blockquote');
  if (quoteText) {
    return `::quote\n${quoteText}\n::end`;
  }

  const codeMatch = /^<pre><code(?:\s+class=["']?language-([a-zA-Z0-9_-]+)["']?)?>([\s\S]*?)<\/code><\/pre>$/i.exec(trimmed);
  if (codeMatch) {
    const language = String(codeMatch[1] || '').trim();
    const body = String(codeMatch[2] || '').trimEnd();
    const header = language ? `::code language=${language}` : '::code';
    return `${header}\n${body}\n::end`;
  }

  return null;
}

function normalizeMarkdownLike(source: string): string | null {
  const trimmed = source.trim();

  const headingMatch = /^(#{1,6})\s+(.*)$/.exec(trimmed);
  if (headingMatch) {
    const headingText = headingMatch[2] ?? '';
    if (headingText.includes('\n')) {
      return null;
    }
    const level = (headingMatch[1] ?? '').length;
    const text = headingText.trim();
    return `::heading level=${level}\n${text}\n::end`;
  }

  if (/^(---|\*\*\*)$/.test(trimmed)) {
    return '::rule';
  }

  const quoteLines = source.split('\n').map((line) => line.trim());
  if (quoteLines.length > 0 && quoteLines.every((line) => /^>\s?/.test(line))) {
    const quote = quoteLines.map((line) => line.replace(/^>\s?/, '')).join('\n').trim();
    return `::quote\n${quote}\n::end`;
  }

  const lines = source.split('\n').filter((line) => String(line || '').trim());
  if (lines.length > 0) {
    const checklistItems = parseMarkdownChecklist(lines);
    if (checklistItems) {
      return `::checklist\n${checklistItems.join('\n')}\n::end`;
    }

    const bulletedItems = parseMarkdownBulleted(lines);
    if (bulletedItems) {
      return `::bulleted-list\n${bulletedItems.join('\n')}\n::end`;
    }

    const numberedItems = parseMarkdownNumbered(lines);
    if (numberedItems) {
      return `::numbered-list\n${numberedItems.join('\n')}\n::end`;
    }
  }

  const codeFence = normalizeCodeFence(source);
  if (codeFence) {
    return codeFence;
  }

  return null;
}

export function normalizeBlockSourceInput(rawSource: string | number | boolean | null | undefined | object): string {
  const source = normalizeLineEndings(rawSource);
  const trimmed = source.trim();

  if (!trimmed) {
    return '';
  }

  if (trimmed.startsWith('::')) {
    return source;
  }

  const html = normalizeHtmlLike(source);
  if (html) {
    return html;
  }

  const markdown = normalizeMarkdownLike(source);
  if (markdown) {
    return markdown;
  }

  return source;
}
