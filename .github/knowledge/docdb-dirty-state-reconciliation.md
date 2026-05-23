# DOC DB Dirty-State Reconciliation

- VS Code dirty tracking is version/save based, not strict text-equality based.
- The webview model may lag while block source/header editor surfaces are open.
- Reconcile must not emit clean while editing surfaces are active, otherwise host can incorrectly restore stub text and interfere with save flow.
- Reconcile should run immediately after close/commit wrappers (`closeBlockSrc`, `commitOpenSources`, `commitOpenSourcesForHistory`) to clear dirty state when edits resolve to saved snapshot.
- Runtime extension uses built artifacts (`build/runtime/...` and `build/docdb-webview.bundle.min.js`), so source fixes require rebuild/install before manual verification.