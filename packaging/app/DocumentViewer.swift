import AppKit
import WebKit

/// One window showing one document.
///
/// The window is deliberately almost nothing: a title bar, and the page filling the rest. The
/// page already carries the whole look — one paper tone, one ink, no second surface — so any
/// toolbar, sidebar, or tinted chrome the app added would be the second surface the format
/// spends its stylesheet refusing. What the app contributes is the paper's colour behind the
/// page while it loads, so a window never flashes white on its way to a dark document.
final class DocumentViewer: NSObject, NSWindowDelegate, WKNavigationDelegate {
    /// The document this window shows.
    let url: URL

    /// Called when the reader closes the window, so the app can let the viewer go.
    private let onClose: (DocumentViewer) -> Void

    private let window: NSWindow
    private let web: WKWebView

    /// The bridge from the page's editing script to the file on disk.
    private var editor: Editor!

    /// The paper tone the page itself uses, so the window matches it before the page loads.
    ///
    /// These are `doc-core`'s `--dx-bg` in each scheme. The page is still the authority; this
    /// is only what sits behind it for the instant before it draws, which is the difference
    /// between opening a document and opening a white flash. It is a dynamic colour, so a
    /// reader switching their Mac to dark takes the window with them and there is nothing to
    /// keep in step by hand.
    private static let paper = NSColor(name: nil) { appearance in
        appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua
            ? NSColor(red: 0.102, green: 0.098, blue: 0.090, alpha: 1)
            : NSColor(red: 0.992, green: 0.988, blue: 0.976, alpha: 1)
    }

    /// Build a window for `url`. Nothing is shown until ``show()``.
    init(url: URL, onClose: @escaping (DocumentViewer) -> Void) {
        self.url = url
        self.onClose = onClose

        let configuration = WKWebViewConfiguration()
        // A document is editable, so this window runs JavaScript — and the *document* still
        // cannot. `Editor` explains the arrangement in full: the page is loaded under
        // `script-src 'none'`, and the editing script runs in its own content world, which
        // the page can neither reach nor be mistaken for. Author markup would have to defeat
        // the render allow-list, then a content-security policy, then a world boundary.
        configuration.defaultWebpagePreferences.allowsContentJavaScript = true
        for script in Editor.userScripts() {
            configuration.userContentController.addUserScript(script)
        }
        web = WKWebView(frame: .zero, configuration: configuration)
        web.allowsMagnification = true

        window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 860, height: 1040),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false)
        window.title = url.lastPathComponent
        // The proxy icon and its path menu come free with this, and they are what tell a reader
        // the window is a file on their disk rather than a preview of one.
        window.representedURL = url
        window.isReleasedWhenClosed = false
        window.contentView = web
        window.setFrameAutosaveName("dx.document")
        // Nothing here is worth restoring. The window's whole content is re-rendered from the
        // file every time it opens, so AppKit reinstating a saved scroll position — measured on
        // whatever document was open last — would land a fresh document part-way down.
        window.isRestorable = false

        window.backgroundColor = Self.paper
        if #available(macOS 12.0, *) {
            web.underPageBackgroundColor = Self.paper
        }

        super.init()
        window.delegate = self
        web.navigationDelegate = self

        // The editing bridge needs nothing from this window. An edit answers with the
        // re-rendered document and the page swaps it in itself, so a window is loaded once —
        // when it opens, and when a reader asks for it again with ⌘R.
        editor = Editor(url: url)
        web.configuration.userContentController.addScriptMessageHandler(
            editor, contentWorld: Editor.world, name: Editor.channel)
    }

    /// Put the window on screen, then render into it.
    ///
    /// That order is not arbitrary. Loading first and activating the app during the load left
    /// the page scrolled to its end on 2 of 5 launches — the reader opened a document and got
    /// its last line. Ordering the window front first means WebKit lays the page out into a
    /// window that is already on screen at its final size, and it has landed at the top on
    /// every launch since.
    func show() {
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        load()
    }

    /// Offset this window from the last one opened, and say where the next should go.
    ///
    /// Every window is saved and restored under one name, so a second document would otherwise
    /// land exactly on top of the first and look like nothing happened. Passing `nil` leaves
    /// the window where it was saved, which is what the first one wants.
    @discardableResult
    func cascade(after previous: NSPoint?) -> NSPoint {
        window.cascadeTopLeft(from: previous ?? .zero)
    }

    /// Render the document again — what the reader gets from View ▸ Reload after editing it.
    @objc func reloadDocument(_ sender: Any?) {
        load()
    }

    /// Undo a pinch: back to the page at the size the stylesheet set it.
    @objc func actualSize(_ sender: Any?) {
        web.magnification = 1
    }

    /// Ask the engine for the page, and show whatever it says.
    ///
    /// A failure is shown *in the window*, because the alternative — an empty window, or no
    /// window at all — is the one outcome that tells the reader nothing. `dx` fails with a
    /// sentence saying what to run, and that sentence is the most useful thing on screen.
    private func load() {
        do {
            web.loadHTMLString(Editor.policed(try Engine.page(for: url)), baseURL: nil)
        } catch let failure as Engine.Failure {
            web.loadHTMLString(Self.notice(failure.message), baseURL: nil)
        } catch {
            web.loadHTMLString(Self.notice(error.localizedDescription), baseURL: nil)
        }
    }

    /// A page carrying one sentence: the app's own voice, not a rendered document.
    ///
    /// It borrows the system's `Canvas`/`CanvasText` colours rather than restating the
    /// palette, so it follows light and dark without keeping a second copy of `doc-core`'s
    /// values that could fall out of step with them.
    private static func notice(_ message: String) -> String {
        let escaped =
            message
            .replacingOccurrences(of: "&", with: "&amp;")
            .replacingOccurrences(of: "<", with: "&lt;")
            .replacingOccurrences(of: ">", with: "&gt;")
        return """
            <!doctype html><meta charset="utf-8"><meta name="color-scheme" content="light dark">
            <body style="margin:0;background:Canvas;color:CanvasText;\
            font:15px/1.6 ui-monospace,SFMono-Regular,Menlo,monospace">
            <pre style="margin:0;padding:56px 64px;white-space:pre-wrap">\(escaped)</pre>
            """
    }

    // MARK: - WKNavigationDelegate

    /// Hand the page to the shared editor.
    ///
    /// Here rather than in `load()` because there is nothing to attach to until the document
    /// has actually been laid out.
    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        webView.evaluateJavaScript(Editor.attach, in: nil, in: Editor.world) { result in
            // A page that cannot be edited is still a page worth reading, so this is a note
            // rather than anything the reader is stopped by.
            if case .failure(let error) = result {
                NSLog("dx-app: the editor did not attach: %@", "\(error)")
            }
        }
    }

    // MARK: - NSWindowDelegate

    /// Let the application release this viewer once its window is gone.
    func windowWillClose(_ notification: Notification) {
        onClose(self)
    }
}
