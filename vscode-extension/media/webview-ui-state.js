"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.createUiStateController = createUiStateController;
function createUiStateController(options = {}) {
    const appearanceStorageKey = String(options.appearanceStorageKey || 'docdb.appearance.v1');
    const editModeStorageKey = String(options.editModeStorageKey || 'docdb.edit-mode.v1');
    const DOC_VIEW_STATES = options.DOC_VIEW_STATES || {};
    const transitionDocViewState = typeof options.transitionDocViewState === 'function'
        ? options.transitionDocViewState
        : () => false;
    const getDocViewState = typeof options.getDocViewState === 'function'
        ? options.getDocViewState
        : () => DOC_VIEW_STATES.BOOTSTRAPPING;
    const isEditModeState = typeof options.isEditModeState === 'function'
        ? options.isEditModeState
        : () => false;
    const syncEditModeIndicators = typeof options.syncEditModeIndicators === 'function'
        ? options.syncEditModeIndicators
        : () => { };
    const postMessage = typeof options.postMessage === 'function'
        ? options.postMessage
        : () => { };
    const publishViewState = typeof options.publishViewState === 'function'
        ? options.publishViewState
        : () => { };
    const getActiveScopedCss = typeof options.getActiveScopedCss === 'function'
        ? options.getActiveScopedCss
        : () => '';
    const closeInlineCssSurface = typeof options.closeInlineCssSurface === 'function'
        ? options.closeInlineCssSurface
        : () => { };
    const commitOpenSources = typeof options.commitOpenSources === 'function'
        ? options.commitOpenSources
        : () => { };
    const closeAutocomplete = typeof options.closeAutocomplete === 'function'
        ? options.closeAutocomplete
        : () => { };
    const setStatus = typeof options.setStatus === 'function'
        ? options.setStatus
        : () => { };
    const tryApplyPendingExternalSource = typeof options.tryApplyPendingExternalSource === 'function'
        ? options.tryApplyPendingExternalSource
        : () => { };
    let currentTheme = 'auto';
    let currentAppearance = {
        paper: 'white',
        density: 'comfortable',
        scale: 100,
    };
    let chromeRevealTimer = null;
    function clampScale(value) {
        const numeric = Number(value);
        if (!Number.isFinite(numeric)) {
            return 100;
        }
        return Math.min(115, Math.max(90, Math.round(numeric)));
    }
    function getCurrentTheme() {
        return currentTheme;
    }
    function getCurrentAppearance() {
        return { ...currentAppearance };
    }
    function setAppearance(nextAppearance = {}) {
        const paper = String(nextAppearance.paper || 'white');
        const density = String(nextAppearance.density || 'comfortable');
        const scale = clampScale(nextAppearance.scale);
        currentAppearance = {
            paper: ['white', 'cream', 'slate'].includes(paper) ? paper : 'white',
            density: ['comfortable', 'compact'].includes(density) ? density : 'comfortable',
            scale,
        };
    }
    function updateAppearance(patch = {}, persist = false) {
        setAppearance({
            ...currentAppearance,
            ...(patch && typeof patch === 'object' ? patch : {}),
        });
        applyAppearance(Boolean(persist));
    }
    function loadAppearance() {
        try {
            const raw = window.localStorage.getItem(appearanceStorageKey);
            if (!raw) {
                return;
            }
            setAppearance(JSON.parse(raw));
        }
        catch {
        }
    }
    function persistAppearance() {
        try {
            window.localStorage.setItem(appearanceStorageKey, JSON.stringify(currentAppearance));
        }
        catch {
        }
    }
    function applyAppearance(persist = false) {
        document.body.dataset.paper = currentAppearance.paper;
        document.body.dataset.density = currentAppearance.density;
        document.documentElement.style.setProperty('--editor-scale', String(currentAppearance.scale / 100));
        const paperSelect = document.getElementById('paper-select');
        const densitySelect = document.getElementById('density-select');
        const scaleSlider = document.getElementById('scale-slider');
        if (paperSelect) {
            paperSelect.value = currentAppearance.paper;
        }
        if (densitySelect) {
            densitySelect.value = currentAppearance.density;
        }
        if (scaleSlider) {
            scaleSlider.value = String(currentAppearance.scale);
        }
        if (persist) {
            persistAppearance();
        }
        publishViewState(getActiveScopedCss());
    }
    function applyTheme(theme, persist = false) {
        const allowed = new Set(['auto', 'light', 'dark']);
        currentTheme = allowed.has(theme) ? theme : 'auto';
        document.body.dataset.theme = currentTheme;
        const prefersDark = window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches;
        const resolved = currentTheme === 'auto' ? (prefersDark ? 'dark' : 'light') : currentTheme;
        document.body.dataset.resolvedTheme = resolved;
        const select = document.getElementById('theme-select');
        if (select) {
            select.value = currentTheme;
        }
        if (persist) {
            postMessage({ type: 'set-theme', theme: currentTheme });
        }
        publishViewState(getActiveScopedCss());
    }
    function setControlsOpen(isOpen) {
        const chrome = document.getElementById('ui-chrome');
        const toggle = document.getElementById('ui-chrome-toggle');
        if (chrome) {
            chrome.dataset.open = isOpen ? 'true' : 'false';
        }
        if (toggle) {
            toggle.setAttribute('aria-expanded', isOpen ? 'true' : 'false');
        }
        if (isOpen) {
            document.body.classList.add('show-chrome');
        }
    }
    function setHelpOpen(isOpen) {
        const chrome = document.getElementById('ui-chrome');
        const btn = document.getElementById('ui-chrome-help-btn');
        if (chrome) {
            chrome.dataset.help = isOpen ? 'true' : 'false';
        }
        if (btn) {
            btn.setAttribute('aria-expanded', isOpen ? 'true' : 'false');
            btn.setAttribute('aria-label', isOpen ? 'Hide Help' : 'Show Help');
        }
        if (isOpen) {
            document.body.classList.add('show-chrome');
        }
    }
    function revealChromeBriefly(durationMs = 1400) {
        document.body.classList.add('show-chrome');
        if (chromeRevealTimer) {
            clearTimeout(chromeRevealTimer);
        }
        chromeRevealTimer = setTimeout(() => {
            const chrome = document.getElementById('ui-chrome');
            if (chrome && (chrome.dataset.open === 'true' || chrome.dataset.help === 'true')) {
                return;
            }
            document.body.classList.remove('show-chrome');
        }, durationMs);
    }
    function toggleControls(forceOpen) {
        const chrome = document.getElementById('ui-chrome');
        const isOpen = chrome ? chrome.dataset.open === 'true' : false;
        const opening = typeof forceOpen === 'boolean' ? forceOpen : !isOpen;
        if (opening) {
            setHelpOpen(false);
        }
        setControlsOpen(opening);
    }
    function toggleHelp() {
        const chrome = document.getElementById('ui-chrome');
        const isOpen = chrome ? chrome.dataset.help === 'true' : false;
        if (!isOpen) {
            setControlsOpen(false);
        }
        setHelpOpen(!isOpen);
    }
    function isEditModeEnabled() {
        return isEditModeState(getDocViewState());
    }
    function loadEditModePreference() {
        try {
            const stored = window.localStorage.getItem(editModeStorageKey);
            if (stored === 'true')
                return true;
            if (stored === 'false')
                return false;
        }
        catch {
        }
        return true;
    }
    function persistEditModePreference(enabled) {
        try {
            window.localStorage.setItem(editModeStorageKey, enabled ? 'true' : 'false');
        }
        catch {
        }
    }
    function setEditMode(enabled) {
        const transitioned = enabled
            ? transitionDocViewState('SET_EDIT')
            : transitionDocViewState('SET_READ');
        const docViewState = getDocViewState();
        if (!transitioned && (docViewState === DOC_VIEW_STATES.BOOTSTRAPPING || docViewState === DOC_VIEW_STATES.LOAD_ERROR)) {
            transitionDocViewState(enabled ? 'START_EDIT' : 'START_READ');
        }
        const editEnabled = isEditModeState(getDocViewState());
        syncEditModeIndicators(editEnabled);
        persistEditModePreference(editEnabled);
    }
    function toggleEditMode() {
        const nextEnabled = !isEditModeEnabled();
        if (!nextEnabled) {
            closeInlineCssSurface();
            commitOpenSources();
            closeAutocomplete();
        }
        setEditMode(nextEnabled);
        setStatus(nextEnabled ? 'Edit mode on' : 'Edit mode off');
        tryApplyPendingExternalSource();
    }
    return {
        applyAppearance,
        applyTheme,
        getCurrentAppearance,
        getCurrentTheme,
        isEditModeEnabled,
        loadAppearance,
        loadEditModePreference,
        revealChromeBriefly,
        setAppearance,
        setControlsOpen,
        setEditMode,
        setHelpOpen,
        toggleControls,
        toggleEditMode,
        toggleHelp,
        updateAppearance,
    };
}
