(() => {
  let booted = false;
  let bootError = '';

  function updateBootError(message) {
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

  window.__docBootFallback = fallback;

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

    if (page) {
      page.dataset.ready = 'true';
      page.dataset.editMode = 'true';
      page.setAttribute('aria-busy', 'false');
    }

    if (loadingScreen) {
      loadingScreen.dataset.hidden = 'true';
      loadingScreen.style.display = 'none';
    }

    if (!loadNote && page) {
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
