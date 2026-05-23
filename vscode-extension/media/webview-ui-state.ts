type DocViewStateMap = Record<string, string>;

interface UiAppearance {
  paper: 'white' | 'cream' | 'slate';
  density: 'comfortable' | 'compact';
  scale: number;
}

interface UiStateControllerOptions {
  appearanceStorageKey?: string;
  editModeStorageKey?: string;
  DOC_VIEW_STATES?: DocViewStateMap;
  transitionDocViewState?: (event: string) => boolean;
  getDocViewState?: () => string;
  isEditModeState?: (state: string) => boolean;
  syncEditModeIndicators?: (enabled: boolean) => void;
  postMessage?: (payload: Record<string, string | number | boolean | null | undefined | object>) => void;
  publishViewState?: (effectiveCss: string) => void;
  getActiveScopedCss?: () => string;
  closeInlineCssSurface?: (restoreFocus?: boolean) => void;
  commitOpenSources?: (exceptIndex?: number) => void;
  closeAutocomplete?: () => void;
  setStatus?: (message: string) => void;
  tryApplyPendingExternalSource?: () => void;
}

interface UiStateController {
  applyAppearance: (persist?: boolean) => void;
  applyTheme: (theme: string, persist?: boolean) => void;
  getCurrentAppearance: () => UiAppearance;
  getCurrentTheme: () => string;
  isEditModeEnabled: () => boolean;
  loadAppearance: () => void;
  loadEditModePreference: () => boolean;
  revealChromeBriefly: (durationMs?: number) => void;
  setAppearance: (nextAppearance?: Partial<UiAppearance>) => void;
  setControlsOpen: (isOpen: boolean) => void;
  setEditMode: (enabled: boolean) => void;
  setHelpOpen: (isOpen: boolean) => void;
  toggleControls: (forceOpen?: boolean) => void;
  toggleEditMode: () => void;
  toggleHelp: () => void;
  updateAppearance: (patch?: Partial<UiAppearance>, persist?: boolean) => void;
}

export function createUiStateController(options: UiStateControllerOptions = {}): UiStateController {
  const appearanceStorageKey = String(options.appearanceStorageKey || 'docdb.appearance.v1');
  const editModeStorageKey = String(options.editModeStorageKey || 'docdb.edit-mode.v1');
  const DOC_VIEW_STATES = options.DOC_VIEW_STATES || {};
  const BOOTSTRAPPING_STATE = DOC_VIEW_STATES.BOOTSTRAPPING ?? 'BOOTSTRAPPING';
  const LOAD_ERROR_STATE = DOC_VIEW_STATES.LOAD_ERROR ?? 'LOAD_ERROR';

  const transitionDocViewState = typeof options.transitionDocViewState === 'function'
    ? options.transitionDocViewState
    : () => false;
  const getDocViewState = typeof options.getDocViewState === 'function'
    ? options.getDocViewState
    : () => BOOTSTRAPPING_STATE;
  const isEditModeState = typeof options.isEditModeState === 'function'
    ? options.isEditModeState
    : () => false;
  const syncEditModeIndicators = typeof options.syncEditModeIndicators === 'function'
    ? options.syncEditModeIndicators
    : () => {};
  const postMessage = typeof options.postMessage === 'function'
    ? options.postMessage
    : () => {};
  const publishViewState = typeof options.publishViewState === 'function'
    ? options.publishViewState
    : () => {};
  const getActiveScopedCss = typeof options.getActiveScopedCss === 'function'
    ? options.getActiveScopedCss
    : () => '';
  const closeInlineCssSurface = typeof options.closeInlineCssSurface === 'function'
    ? options.closeInlineCssSurface
    : () => {};
  const commitOpenSources = typeof options.commitOpenSources === 'function'
    ? options.commitOpenSources
    : () => {};
  const closeAutocomplete = typeof options.closeAutocomplete === 'function'
    ? options.closeAutocomplete
    : () => {};
  const setStatus = typeof options.setStatus === 'function'
    ? options.setStatus
    : () => {};
  const tryApplyPendingExternalSource = typeof options.tryApplyPendingExternalSource === 'function'
    ? options.tryApplyPendingExternalSource
    : () => {};

  let currentTheme = 'auto';
  let currentAppearance: UiAppearance = {
    paper: 'white',
    density: 'comfortable',
    scale: 100,
  };
  let chromeRevealTimer: ReturnType<typeof setTimeout> | null = null;

  function getSelect(id: string): HTMLSelectElement | null {
    const node = document.getElementById(id);
    return node instanceof HTMLSelectElement ? node : null;
  }

  function getInput(id: string): HTMLInputElement | null {
    const node = document.getElementById(id);
    return node instanceof HTMLInputElement ? node : null;
  }

  function clampScale(value: string | number | boolean | null | undefined | object): number {
    const numeric = Number(value);

    if (!Number.isFinite(numeric)) {
      return 100;
    }

    return Math.min(115, Math.max(90, Math.round(numeric)));
  }

  function getCurrentTheme(): string {
    return currentTheme;
  }

  function getCurrentAppearance(): UiAppearance {
    return { ...currentAppearance };
  }

  function setAppearance(nextAppearance: Partial<UiAppearance> = {}): void {
    const paperRaw = String(nextAppearance.paper || 'white');
    const densityRaw = String(nextAppearance.density || 'comfortable');
    const scale = clampScale(nextAppearance.scale);

    currentAppearance = {
      paper: ['white', 'cream', 'slate'].includes(paperRaw) ? paperRaw as UiAppearance['paper'] : 'white',
      density: ['comfortable', 'compact'].includes(densityRaw) ? densityRaw as UiAppearance['density'] : 'comfortable',
      scale,
    };
  }

  function updateAppearance(patch: Partial<UiAppearance> = {}, persist = false): void {
    setAppearance({
      ...currentAppearance,
      ...(patch && typeof patch === 'object' ? patch : {}),
    });
    applyAppearance(Boolean(persist));
  }

  function loadAppearance(): void {
    try {
      const raw = window.localStorage.getItem(appearanceStorageKey);

      if (!raw) {
        return;
      }

      setAppearance(JSON.parse(raw) as Partial<UiAppearance>);
    } catch {
    }
  }

  function persistAppearance(): void {
    try {
      window.localStorage.setItem(appearanceStorageKey, JSON.stringify(currentAppearance));
    } catch {
    }
  }

  function applyAppearance(persist = false): void {
    document.body.dataset.paper = currentAppearance.paper;
    document.body.dataset.density = currentAppearance.density;
    document.documentElement.style.setProperty('--editor-scale', String(currentAppearance.scale / 100));

    const paperSelect = getSelect('paper-select');
    const densitySelect = getSelect('density-select');
    const scaleSlider = getInput('scale-slider');

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

  function applyTheme(theme: string, persist = false): void {
    const allowed = new Set(['auto', 'light', 'dark']);
    currentTheme = allowed.has(theme) ? theme : 'auto';
    document.body.dataset.theme = currentTheme;

    const prefersDark = window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches;
    const resolved = currentTheme === 'auto' ? (prefersDark ? 'dark' : 'light') : currentTheme;
    document.body.dataset.resolvedTheme = resolved;

    const select = getSelect('theme-select');
    if (select) {
      select.value = currentTheme;
    }

    if (persist) {
      postMessage({ type: 'set-theme', theme: currentTheme });
    }

    publishViewState(getActiveScopedCss());
  }

  function setControlsOpen(isOpen: boolean): void {
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

  function setHelpOpen(isOpen: boolean): void {
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

  function revealChromeBriefly(durationMs = 1400): void {
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

  function toggleControls(forceOpen?: boolean): void {
    const chrome = document.getElementById('ui-chrome');
    const isOpen = chrome ? chrome.dataset.open === 'true' : false;
    const opening = typeof forceOpen === 'boolean' ? forceOpen : !isOpen;

    if (opening) {
      setHelpOpen(false);
    }

    setControlsOpen(opening);
  }

  function toggleHelp(): void {
    const chrome = document.getElementById('ui-chrome');
    const isOpen = chrome ? chrome.dataset.help === 'true' : false;

    if (!isOpen) {
      setControlsOpen(false);
    }

    setHelpOpen(!isOpen);
  }

  function isEditModeEnabled(): boolean {
    return isEditModeState(getDocViewState());
  }

  function loadEditModePreference(): boolean {
    try {
      const stored = window.localStorage.getItem(editModeStorageKey);
      if (stored === 'true') return true;
      if (stored === 'false') return false;
    } catch {
    }
    return true;
  }

  function persistEditModePreference(enabled: boolean): void {
    try {
      window.localStorage.setItem(editModeStorageKey, enabled ? 'true' : 'false');
    } catch {
    }
  }

  function setEditMode(enabled: boolean): void {
    const transitioned = enabled
      ? transitionDocViewState('SET_EDIT')
      : transitionDocViewState('SET_READ');

    const docViewState = getDocViewState();
    if (!transitioned && (docViewState === BOOTSTRAPPING_STATE || docViewState === LOAD_ERROR_STATE)) {
      transitionDocViewState(enabled ? 'START_EDIT' : 'START_READ');
    }

    const editEnabled = isEditModeState(getDocViewState());
    syncEditModeIndicators(editEnabled);
    persistEditModePreference(editEnabled);
  }

  function toggleEditMode(): void {
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
