"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.createBlockRenderer = createBlockRenderer;
function isBulletedListType(type) {
    return type === 'list' || type === 'bulleted-list';
}
function isNumberedListType(type) {
    return type === 'numbered-list';
}
function parseInlineLinks(text) {
    const tokens = [];
    const pattern = /\[([^\]]+)\]\(([^)]+)\)/g;
    let lastIndex = 0;
    let match = pattern.exec(text);
    while (match !== null) {
        if (match.index > lastIndex) {
            tokens.push({ type: 'text', value: text.slice(lastIndex, match.index) });
        }
        tokens.push({ type: 'link', label: match[1], href: match[2] });
        lastIndex = match.index + match[0].length;
        match = pattern.exec(text);
    }
    if (lastIndex < text.length) {
        tokens.push({ type: 'text', value: text.slice(lastIndex) });
    }
    return tokens;
}
function createBlockRenderer(options = {}) {
    const applyBlockViewPresentation = typeof options.applyBlockViewPresentation === 'function'
        ? options.applyBlockViewPresentation
        : () => { };
    const listItemText = typeof options.listItemText === 'function'
        ? options.listItemText
        : (item) => String(item || '');
    const splitClassNames = typeof options.splitClassNames === 'function'
        ? options.splitClassNames
        : () => [];
    const getWorkspaceBaseUri = typeof options.getWorkspaceBaseUri === 'function'
        ? options.getWorkspaceBaseUri
        : () => '';
    function setInlineContent(el, text) {
        const tokens = parseInlineLinks(String(text || ''));
        if (tokens.length === 1 && tokens[0].type === 'text') {
            el.textContent = tokens[0].value;
            return;
        }
        el.textContent = '';
        for (const token of tokens) {
            if (token.type === 'text') {
                el.appendChild(document.createTextNode(token.value));
            }
            else {
                const a = document.createElement('a');
                a.className = 'doc-link';
                a.textContent = token.label;
                a.dataset.docHref = token.href;
                a.href = '#';
                el.appendChild(a);
            }
        }
    }
    function buildRenderedContent(block) {
        function applyBlockDecorations(element) {
            if (!element)
                return element;
            if (block.id) {
                element.id = block.id;
                element.dataset.blockId = block.id;
            }
            for (const token of splitClassNames(block.className)) {
                element.classList.add(token);
            }
            return element;
        }
        if (block.type === 'heading') {
            const level = Math.min(6, Math.max(1, Number(block.level || 1)));
            const heading = document.createElement('h' + level);
            heading.textContent = block.text || '';
            return applyBlockDecorations(heading);
        }
        if (block.type === 'paragraph') {
            const paragraph = document.createElement('p');
            setInlineContent(paragraph, block.text || '');
            return applyBlockDecorations(paragraph);
        }
        if (block.type === 'image') {
            const figure = document.createElement('figure');
            figure.className = 'image-wrap';
            const image = document.createElement('img');
            const rawSrc = String(block.src || '');
            const workspaceBaseUri = String(getWorkspaceBaseUri() || '');
            if (rawSrc.startsWith('data:') || rawSrc.startsWith('http://') || rawSrc.startsWith('https://') || rawSrc.startsWith('vscode-')) {
                image.src = rawSrc;
            }
            else if (rawSrc && workspaceBaseUri) {
                image.src = workspaceBaseUri + '/' + rawSrc.replace(/^\//, '');
            }
            else {
                image.src = rawSrc;
            }
            image.alt = block.alt || '';
            image.loading = 'lazy';
            figure.appendChild(image);
            if (block.alt) {
                const figcaption = document.createElement('figcaption');
                figcaption.textContent = block.alt;
                figure.appendChild(figcaption);
            }
            return applyBlockDecorations(figure);
        }
        if (block.type === 'code') {
            const pre = document.createElement('pre');
            pre.textContent = block.text || '';
            return applyBlockDecorations(pre);
        }
        if (block.type === 'style' || block.type === 'stylesheet') {
            const hidden = document.createElement('span');
            hidden.hidden = true;
            hidden.setAttribute('aria-hidden', 'true');
            return applyBlockDecorations(hidden);
        }
        if (block.type === 'rule') {
            return applyBlockDecorations(document.createElement('hr'));
        }
        if (block.type === 'quote') {
            const quote = document.createElement('blockquote');
            const paragraph = document.createElement('p');
            setInlineContent(paragraph, block.text || '');
            quote.appendChild(paragraph);
            return applyBlockDecorations(quote);
        }
        if (isBulletedListType(block.type)) {
            const ul = document.createElement('ul');
            (block.items || []).forEach((item) => {
                const li = document.createElement('li');
                setInlineContent(li, listItemText(item));
                ul.appendChild(li);
            });
            return applyBlockDecorations(ul);
        }
        if (isNumberedListType(block.type)) {
            const ol = document.createElement('ol');
            (block.items || []).forEach((item) => {
                const li = document.createElement('li');
                setInlineContent(li, listItemText(item));
                ol.appendChild(li);
            });
            return applyBlockDecorations(ol);
        }
        if (block.type === 'checklist') {
            const ul = document.createElement('ul');
            ul.className = 'checklist-wrap';
            (block.items || []).forEach((item, itemIndex) => {
                const li = document.createElement('li');
                const checkbox = document.createElement('input');
                checkbox.type = 'checkbox';
                checkbox.checked = Boolean(item.checked);
                checkbox.dataset.itemIndex = String(itemIndex);
                const span = document.createElement('span');
                setInlineContent(span, item.text);
                if (item.checked) {
                    span.className = 'check-done';
                }
                li.appendChild(checkbox);
                li.appendChild(span);
                ul.appendChild(li);
            });
            return applyBlockDecorations(ul);
        }
        const fallback = document.createElement('p');
        fallback.textContent = block.text || '';
        return applyBlockDecorations(fallback);
    }
    function buildBlockWrap(block, index) {
        const wrap = document.createElement('div');
        wrap.className = 'block-wrap';
        wrap.dataset.blockIndex = String(index);
        for (const token of splitClassNames(block && block.className)) {
            wrap.classList.add(token);
        }
        if (block && block.id) {
            wrap.dataset.blockId = String(block.id);
        }
        const view = document.createElement('div');
        view.className = 'block-view';
        view.setAttribute('role', 'article');
        view.setAttribute('tabindex', '0');
        view.setAttribute('aria-label', `Block ${index + 1}: ${block.type}`);
        applyBlockViewPresentation(view, block);
        view.appendChild(buildRenderedContent(block));
        const srcWrap = document.createElement('div');
        srcWrap.className = 'block-src-wrapper';
        srcWrap.style.display = 'none';
        const header = document.createElement('textarea');
        header.className = 'block-edit-header';
        header.setAttribute('aria-label', 'Edit block header');
        header.spellcheck = false;
        header.wrap = 'off';
        header.style.display = 'none';
        const bodyWrap = document.createElement('div');
        bodyWrap.className = 'block-src-body-wrap';
        const mirror = document.createElement('div');
        mirror.className = 'block-src-mirror';
        mirror.setAttribute('aria-hidden', 'true');
        const source = document.createElement('textarea');
        source.className = 'block-src';
        source.setAttribute('aria-label', 'Edit block content');
        source.spellcheck = false;
        source.wrap = 'off';
        const menu = document.createElement('div');
        menu.className = 'autocomplete-menu';
        menu.setAttribute('role', 'listbox');
        menu.setAttribute('aria-label', 'Autocomplete suggestions');
        menu.style.display = 'none';
        bodyWrap.appendChild(mirror);
        bodyWrap.appendChild(source);
        bodyWrap.appendChild(menu);
        srcWrap.appendChild(header);
        srcWrap.appendChild(bodyWrap);
        wrap.appendChild(view);
        wrap.appendChild(srcWrap);
        return wrap;
    }
    function getRawSourceForEditor(rawSource) {
        return String(rawSource || '')
            .replace(/\r\n/g, '\n')
            .replace(/\n::end\s*$/, '');
    }
    function getRawSourceFromEditor(editorValue) {
        return String(editorValue || '').replace(/\r\n/g, '\n');
    }
    function splitBlockSourceForEditor(rawSource, blockType) {
        const normalized = String(rawSource || '').replace(/\r\n/g, '\n');
        const trimmed = normalized.trim();
        if (!trimmed.startsWith('::')) {
            return {
                headerSource: '',
                bodySource: normalized,
                footerSource: '',
            };
        }
        const lines = normalized.split('\n');
        const headerSource = String(lines[0] || '').trimEnd();
        let endIdx = lines.length;
        for (let index = 1; index < lines.length; index += 1) {
            if (lines[index].trim() === '::end') {
                endIdx = index;
                break;
            }
        }
        return {
            headerSource,
            bodySource: lines.slice(1, endIdx).join('\n'),
            footerSource: blockType === 'rule' ? '' : (endIdx < lines.length ? '::end' : ''),
        };
    }
    function buildRawSourceFromEditorParts(headerSource, bodySource, footerSource) {
        const header = String(headerSource || '').trimEnd();
        const body = String(bodySource || '').replace(/\r\n/g, '\n');
        const footer = String(footerSource || '').trim();
        if (!header) {
            return body;
        }
        const parts = [header];
        if (body.length > 0) {
            parts.push(body);
        }
        if (footer) {
            parts.push(footer);
        }
        return parts.join('\n');
    }
    function getBlockHeaderEditor(textarea) {
        if (!textarea)
            return null;
        if (textarea.classList && textarea.classList.contains('block-edit-header')) {
            return textarea;
        }
        const srcWrap = textarea.closest('.block-src-wrapper');
        return srcWrap ? srcWrap.querySelector('.block-edit-header') : null;
    }
    function getHeaderSourceFromEditor(textarea) {
        const headerEditor = getBlockHeaderEditor(textarea);
        if (!headerEditor) {
            return String(textarea?.dataset?.headerSource || '');
        }
        return getRawSourceFromEditor(headerEditor.value || '');
    }
    function renderEditableHeader(textarea) {
        if (!textarea)
            return;
        const headerEl = getBlockHeaderEditor(textarea);
        const headerSource = String(textarea.dataset.headerSource || '');
        if (!headerEl) {
            return;
        }
        if (!headerSource.trim()) {
            headerEl.value = '';
            headerEl.style.display = 'none';
            headerEl.classList.remove('tag-mode', 'paragraph-mode');
            headerEl.removeAttribute('title');
            headerEl.style.height = '0px';
            return;
        }
        headerEl.value = headerSource;
        headerEl.style.display = 'block';
        headerEl.setAttribute('title', 'Type block tag (for example ::heading level=1) or plain text');
        headerEl.style.height = '0px';
        headerEl.style.height = headerEl.scrollHeight + 'px';
        const inTagMode = String(headerEl.value || '').startsWith(':');
        headerEl.classList.toggle('tag-mode', inTagMode);
        headerEl.classList.toggle('paragraph-mode', !inTagMode);
    }
    function applyEditableBodyPresentation(wrap, block) {
        if (!wrap || !block)
            return;
        const view = wrap.querySelector('.block-view');
        const source = wrap.querySelector('.block-src');
        if (!view || !source)
            return;
        const rendered = view.firstElementChild;
        if (rendered && rendered.id) {
            rendered.dataset.originalEditingId = rendered.id;
            rendered.removeAttribute('id');
        }
        const previousClasses = String(source.dataset.editPresentationClasses || '').split(' ').filter(Boolean);
        if (previousClasses.length > 0) {
            source.classList.remove(...previousClasses);
        }
        const nextClasses = [`block-src-type-${String(block.type || 'paragraph').toLowerCase()}`];
        if (block.type === 'heading') {
            nextClasses.push(`block-src-heading-${Math.min(6, Math.max(1, Number(block.level || 1)))}`);
        }
        const customClasses = splitClassNames(block.className);
        nextClasses.push(...customClasses);
        source.classList.add(...nextClasses);
        source.dataset.editPresentationClasses = nextClasses.join(' ');
        if (block.id) {
            source.id = block.id;
        }
        else {
            source.removeAttribute('id');
        }
        renderEditableHeader(source);
    }
    function clearEditableBodyPresentation(wrap) {
        if (!wrap)
            return;
        const view = wrap.querySelector('.block-view');
        const source = wrap.querySelector('.block-src');
        const header = wrap.querySelector('.block-edit-header');
        if (!source)
            return;
        const previousClasses = String(source.dataset.editPresentationClasses || '').split(' ').filter(Boolean);
        if (previousClasses.length > 0) {
            source.classList.remove(...previousClasses);
        }
        delete source.dataset.editPresentationClasses;
        source.removeAttribute('id');
        if (header) {
            header.textContent = '';
            header.style.display = 'none';
        }
        const rendered = view ? view.firstElementChild : null;
        if (rendered && rendered.dataset.originalEditingId) {
            rendered.id = rendered.dataset.originalEditingId;
            delete rendered.dataset.originalEditingId;
        }
    }
    return {
        applyEditableBodyPresentation,
        buildBlockWrap,
        buildRawSourceFromEditorParts,
        buildRenderedContent,
        clearEditableBodyPresentation,
        getBlockHeaderEditor,
        getHeaderSourceFromEditor,
        getRawSourceForEditor,
        getRawSourceFromEditor,
        renderEditableHeader,
        splitBlockSourceForEditor,
    };
}
