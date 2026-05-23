declare function acquireVsCodeApi(): {
  postMessage: (message: object) => void;
  getState?: () => object | null;
  setState?: (state: object) => void;
};

declare module './webview-fsm.mjs' {
  export const DOC_SAVE_STATES: Record<string, string>;
  export const DOC_SAVE_TRANSITIONS: Record<string, Record<string, string>>;
  export const DOC_VIEW_STATES: Record<string, string>;
  export const DOC_VIEW_TRANSITIONS: Record<string, Record<string, string>>;
}

declare module './webview-block-renderer.js' {
  export * from './webview-block-renderer';
}

declare module './webview-doc-model.js' {
  export * from './webview-doc-model';
}

declare module './webview-ui-state.js' {
  export * from './webview-ui-state';
}
