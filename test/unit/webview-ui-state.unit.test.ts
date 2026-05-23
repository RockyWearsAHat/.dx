import test from 'node:test';
import assert from 'node:assert/strict';
import { createUiStateController } from '#runtime-media/webview-ui-state.js';

function createStorage() {
  const values = new Map();

  return {
    getItem(key) {
      return values.has(key) ? values.get(key) : null;
    },
    setItem(key, value) {
      values.set(key, String(value));
    },
  };
}

function installDomStubs() {
  const elements = new Map();
  const styleValues = new Map();
  const body = {
    dataset: {},
    classList: {
      add() {},
      remove() {},
    },
  };
  const documentElement = {
    style: {
      setProperty(name, value) {
        styleValues.set(name, value);
      },
    },
  };

  for (const id of ['paper-select', 'density-select', 'scale-slider', 'theme-select', 'ui-chrome', 'ui-chrome-toggle', 'ui-chrome-help-btn']) {
    elements.set(id, {
      dataset: {},
      value: '',
      attributes: {},
      setAttribute(name, value) {
        this.attributes[name] = String(value);
      },
    });
  }

  const document = {
    body,
    documentElement,
    getElementById(id) {
      return elements.get(id) || null;
    },
  };

  const window = {
    localStorage: createStorage(),
    matchMedia: () => ({ matches: true }),
  };

  return { document, window, elements, styleValues };
}

test('ui state controller owns appearance updates and edit mode toggles', () => {
  const originalDocument = globalThis.document;
  const originalWindow = globalThis.window;
  const originalHtmlSelectElement = globalThis.HTMLSelectElement;
  const originalHtmlInputElement = globalThis.HTMLInputElement;
  const originalHtmlElement = globalThis.HTMLElement;
  class HtmlElementStub {}
  class HtmlSelectElementStub extends HtmlElementStub {}
  class HtmlInputElementStub extends HtmlElementStub {}

  globalThis.HTMLElement = HtmlElementStub;
  globalThis.HTMLSelectElement = HtmlSelectElementStub;
  globalThis.HTMLInputElement = HtmlInputElementStub;

  const { document, window, styleValues } = installDomStubs();
  globalThis.document = document;
  globalThis.window = window;

  let docViewState = 'ready-read';
  let publishedCss = null;
  let statusMessage = null;
  const transitions = [];

  try {
    const controller = createUiStateController({
      DOC_VIEW_STATES: {
        BOOTSTRAPPING: 'boot',
        LOAD_ERROR: 'error',
      },
      getDocViewState: () => docViewState,
      isEditModeState: (state) => state === 'ready-edit',
      transitionDocViewState: (event) => {
        transitions.push(event);
        if (event === 'SET_EDIT') {
          docViewState = 'ready-edit';
          return true;
        }
        if (event === 'SET_READ') {
          docViewState = 'ready-read';
          return true;
        }
        return false;
      },
      publishViewState: (css) => {
        publishedCss = css;
      },
      getActiveScopedCss: () => '.demo { color: red; }',
      setStatus: (value) => {
        statusMessage = value;
      },
      syncEditModeIndicators: () => {},
      tryApplyPendingExternalSource: () => {},
      closeInlineCssSurface: () => {},
      commitOpenSources: () => {},
      closeAutocomplete: () => {},
    });

    controller.updateAppearance({ paper: 'cream', density: 'compact', scale: '120' }, true);

    assert.deepEqual(controller.getCurrentAppearance(), {
      paper: 'cream',
      density: 'compact',
      scale: 115,
    });
    assert.equal(document.body.dataset.paper, 'cream');
    assert.equal(document.body.dataset.density, 'compact');
    assert.equal(styleValues.get('--editor-scale'), '1.15');
    assert.equal(publishedCss, '.demo { color: red; }');
    assert.equal(window.localStorage.getItem('docdb.appearance.v1'), JSON.stringify({
      paper: 'cream',
      density: 'compact',
      scale: 115,
    }));

    controller.toggleEditMode();

    assert.equal(docViewState, 'ready-edit');
    assert.deepEqual(transitions, ['SET_EDIT']);
    assert.equal(statusMessage, 'Edit mode on');
    assert.equal(window.localStorage.getItem('docdb.edit-mode.v1'), 'true');
  } finally {
    globalThis.document = originalDocument;
    globalThis.window = originalWindow;
    globalThis.HTMLSelectElement = originalHtmlSelectElement;
    globalThis.HTMLInputElement = originalHtmlInputElement;
    globalThis.HTMLElement = originalHtmlElement;
  }
});