(() => {
  interface BootFallback {
    markBooted: () => void;
  }

  interface WindowWithDocBootFallback extends Window {
    __docBootFallback?: BootFallback;
  }

  let booted = false;
  let bootError = '';

  function updateBootError(message: string | number | boolean | null | undefined | object): void {
    const text = String(message || '').trim();
    if (!text) {
      return;
    }
    bootError = text;
  }

  const fallback = {
    markBooted() {
      booted = true;
    },
  };

  (window as WindowWithDocBootFallback).__docBootFallback = fallback;

  const moduleScript = document.getElementById('doc-main-module');
  if (moduleScript) {
    moduleScript.addEventListener('error', () => {
      updateBootError('Failed to load webview module script.');
    });
  }

  window.addEventListener('error', (event) => {
    const message = event && event.message ? event.message : '';
    if (message) {
      updateBootError(`Runtime error: ${message}`);
    }
  });

  window.addEventListener('unhandledrejection', (event) => {
    const reason = event && event.reason;
    if (reason instanceof Error) {
      updateBootError(`Unhandled rejection: ${reason.message}`);
      return;
    }
    if (typeof reason === 'string' && reason.trim()) {
      updateBootError(`Unhandled rejection: ${reason}`);
    }
  });

  setTimeout(() => {
    if (booted) {
      return;
    }

    const page = document.querySelector('.page');
    const loadingScreen = document.getElementById('loading-screen');
    let loadNote = document.getElementById('load-note');

    if (page instanceof HTMLElement) {
      page.dataset.ready = 'true';
      page.dataset.editMode = 'true';
      page.setAttribute('aria-busy', 'false');
    }

    if (loadingScreen) {
      loadingScreen.dataset.hidden = 'true';
      loadingScreen.style.display = 'none';
    }

    if (!loadNote && page instanceof HTMLElement) {
      loadNote = document.createElement('div');
      loadNote.id = 'load-note';
      loadNote.className = 'load-note error';
      page.appendChild(loadNote);
    }

    if (loadNote) {
      const detail = bootError ? ` ${bootError}` : '';
      loadNote.textContent = `Bootstrap timed out: fallback recovery activated.${detail}`;
      loadNote.classList.add('error');
    }

    console.error('[doc-webview] bootstrap fallback activated');
  }, 2500);
})();
