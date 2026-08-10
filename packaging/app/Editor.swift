import Foundation
import WebKit

/// The bridge between the page a reader is typing on and the `dx` that owns the file.
///
/// The editing itself — the caret, the keyboard, what Return means, when a block keeps
/// drawing itself — is `editor/surface/edit.js`, shared with the VS Code webview so the two
/// surfaces cannot drift. This class is the half that can only exist on a Mac: it carries
/// messages from that script to the bundled binary and hands back what `dx` said.
///
/// Nothing here loads a page. An edit answers with the re-rendered document and the script
/// swaps that one element in, so the window a reader is writing in is never replaced: no
/// flash, no lost scroll position, no reattaching the editor to a page that just appeared.
///
/// # Why the page can be scripted without the document being able to script
/// A `.dx` can contain author markup (`::html`, `::svg`), and it was written by whoever wrote
/// the repository it came from. Enabling JavaScript for the page would ordinarily be handing
/// that author a machine. Two independent things stop it:
///
/// 1. **The page is served under `script-src 'none'`.** `dx render` never emits a `<script>`
///    — author markup goes through an allow-list with no place for one — and the policy says
///    so in the one place a reader's machine would otherwise take the author's word for it.
///    If sanitization ever failed, the browser still refuses to run what got through.
/// 2. **The editor is not in the page.** It runs in its own `WKContentWorld`, which the page
///    cannot reach and the page's policy does not govern. Author markup cannot call it,
///    cannot see its message handler, and cannot pretend to be it.
///
/// Those two together are why this is safe in a way that "we sanitize carefully" is not:
/// the document would have to defeat the allow-list *and* a content-security policy *and*
/// a world boundary, and the third has no known way through at all.
final class Editor: NSObject, WKScriptMessageHandlerWithReply {
    /// The world the editing script lives in — never the page's.
    static let world = WKContentWorld.world(name: "dx-editor")

    /// Name the script posts to.
    static let channel = "dx"

    /// The document being edited.
    private let url: URL

    /// - Parameter url: the document this editor writes to.
    init(url: URL) {
        self.url = url
    }

    /// The page's own content: the editing script and the styles that make a field look like
    /// the block it replaced.
    ///
    /// Read from the bundle rather than compiled in, for the same reason the browser
    /// extension is: one source, `editor/surface`, copied into each thing that ships it.
    static func userScripts() -> [WKUserScript] {
        var scripts: [WKUserScript] = []
        if let css = resource("edit", "css") {
            scripts.append(
                WKUserScript(
                    source: injectStyle(css),
                    injectionTime: .atDocumentEnd,
                    forMainFrameOnly: true,
                    in: world))
        }
        if let js = resource("edit", "js") {
            scripts.append(
                WKUserScript(
                    source: js, injectionTime: .atDocumentEnd, forMainFrameOnly: true, in: world))
        }
        return scripts
    }

    /// Attach the shared editor to a freshly loaded page.
    ///
    /// `host` is the contract `edit.js` documents; each call becomes one message to this
    /// class and, on the other side of it, one `dx` command.
    static let attach = """
        window.dxEditor && window.dxEditor.attach({
          source: (id) => window.webkit.messageHandlers.\(channel).postMessage(
            { op: 'source', id }),
          parts: (id) => window.webkit.messageHandlers.\(channel).postMessage(
            { op: 'parts', id }),
          decorate: (text) => window.webkit.messageHandlers.\(channel).postMessage(
            { op: 'decorate', text }),
          commit: (id, text, then) => window.webkit.messageHandlers.\(channel).postMessage(
            { op: 'commit', id, text, then }),
          replace: (id, header, body, then) => window.webkit.messageHandlers.\(channel).postMessage(
            { op: 'replace', id, header, body, then }),
          remove: (id) => window.webkit.messageHandlers.\(channel).postMessage(
            { op: 'remove', id }),
          run: (id) => window.webkit.messageHandlers.\(channel).postMessage(
            { op: 'run', id }),
          check: (id, item) => window.webkit.messageHandlers.\(channel).postMessage(
            { op: 'check', id, item }),
          board: (id, action, spec) => window.webkit.messageHandlers.\(channel).postMessage(
            { op: 'board', id, action, spec })
        });
        """

    /// A stylesheet, as a statement that adds it to the page it is evaluated in.
    private static func injectStyle(_ css: String) -> String {
        let literal = css
            .replacingOccurrences(of: "\\", with: "\\\\")
            .replacingOccurrences(of: "`", with: "\\`")
            .replacingOccurrences(of: "$", with: "\\$")
        return """
            (function () {
              const style = document.createElement('style');
              style.textContent = `\(literal)`;
              document.head.appendChild(style);
            })();
            """
    }

    /// One file from `Contents/Resources/surface`, which `build-app.sh` copies out of
    /// `editor/surface`.
    private static func resource(_ name: String, _ extension: String) -> String? {
        guard
            let url = Bundle.main.url(
                forResource: name, withExtension: `extension`, subdirectory: "surface")
        else { return nil }
        return try? String(contentsOf: url, encoding: .utf8)
    }

    /// Put a content-security policy on a rendered page.
    ///
    /// `dx render` produces a document, not a security context — the context is whoever
    /// displays it, which here is this window. `script-src 'none'` is the whole point;
    /// the rest keeps a document from reaching the network at all, which a page made of
    /// inlined bytes never needs to do.
    static func policed(_ page: String) -> String {
        let policy =
            "<meta http-equiv=\"Content-Security-Policy\" content=\""
            + "default-src 'none'; script-src 'none'; style-src 'unsafe-inline'; "
            + "img-src data:; font-src data:\">"
        guard let head = page.range(of: "<head>") else {
            return policy + page
        }
        return page.replacingCharacters(in: head, with: "<head>" + policy)
    }

    // MARK: - WKScriptMessageHandlerWithReply

    /// Answer one call from the editing script.
    ///
    /// Every failure is returned to the script as an error rather than thrown away, because
    /// the script shows it on the page — a save that quietly did not happen is the worst
    /// outcome an editor has.
    func userContentController(
        _ controller: WKUserContentController,
        didReceive message: WKScriptMessage,
        replyHandler: @escaping (Any?, String?) -> Void
    ) {
        guard let call = message.body as? [String: Any], let op = call["op"] as? String else {
            replyHandler(nil, "malformed editor message")
            return
        }

        do {
            switch op {
            case "source":
                replyHandler(try Engine.source(of: try id(call), in: url), nil)
            case "parts":
                let block = try id(call)
                replyHandler(
                    [
                        "header": try Engine.header(of: block, in: url),
                        "body": try Engine.source(of: block, in: url),
                    ], nil)
            case "decorate":
                replyHandler(try Engine.decorate(call["text"] as? String ?? ""), nil)
            case "run":
                // Off the main queue, because the run takes as long as the block's code
                // does, and a window that cannot repaint while code runs is a hang.
                let block = try id(call)
                let before = try? Data(contentsOf: url)
                DispatchQueue.global(qos: .userInitiated).async {
                    var failure: String?
                    do {
                        try Engine.execute(block, in: self.url)
                    } catch let refused as Engine.Failure {
                        failure = refused.message
                    } catch {
                        failure = error.localizedDescription
                    }
                    DispatchQueue.main.async {
                        // A failed *block* was still written into the document, and the page
                        // showing that failure is the report. Only a run that changed
                        // nothing — a missing sandbox, a refused file — has just a sentence.
                        if let failure, (try? Data(contentsOf: self.url)) == before {
                            replyHandler(nil, failure)
                            return
                        }
                        do {
                            replyHandler(try self.sheet(focusing: nil), nil)
                        } catch let refused as Engine.Failure {
                            replyHandler(nil, refused.message)
                        } catch {
                            replyHandler(nil, error.localizedDescription)
                        }
                    }
                }
                return
            case "commit":
                let block = try id(call)
                try Engine.set(block, in: url, to: call["text"] as? String ?? "")
                // Return: the sentence just written is saved and the next one is started, in
                // this one call. The reader lands in the new paragraph, on the page they were
                // already looking at — nothing is loaded again.
                var focus: String?
                if call["then"] as? String == "insert" {
                    focus = try Engine.insert(after: block, in: url)
                }
                replyHandler(try sheet(focusing: focus), nil)
            case "replace":
                // A retype: the reader rewrote the tag line over the block. The engine
                // answers with the id the replacement took, which is where the caret goes —
                // and Return asks for the next paragraph after it, in this same call.
                let replaced = try Engine.replace(
                    try id(call), in: url,
                    header: call["header"] as? String ?? "",
                    body: call["body"] as? String ?? "")
                var focus: String?
                if call["then"] as? String == "insert" {
                    focus = try Engine.insert(after: replaced, in: url)
                }
                replyHandler(try sheet(focusing: focus), nil)
            case "remove":
                try Engine.remove(try id(call), in: url)
                replyHandler(try sheet(focusing: nil), nil)
            case "check":
                // A box on a checklist, clicked — `dx check`, so the marker the format keeps
                // the checked state in is spelled by the one thing that knows how to spell it.
                let item = (call["item"] as? NSNumber)?.intValue ?? 0
                try Engine.check(try id(call), in: url, item: item)
                replyHandler(try sheet(focusing: nil), nil)
            case "board":
                // A drag, a link, or a new node on the canvas — `dx board`, which is the
                // same operation an agent performs, so the two cannot write different lines.
                let focus = try Engine.board(
                    call["action"] as? String ?? "",
                    on: try id(call), in: url,
                    spec: call["spec"] as? [String: Any] ?? [:])
                replyHandler(try sheet(focusing: focus), nil)
            default:
                replyHandler(nil, "unknown editor operation `\(op)`")
            }
        } catch let failure as Engine.Failure {
            replyHandler(nil, failure.message)
        } catch {
            replyHandler(nil, error.localizedDescription)
        }
    }

    /// The document after a change, in the shape `edit.js` puts back on the page.
    ///
    /// `focus` is the block to open once it is there — the paragraph Return just created.
    private func sheet(focusing focus: String?) throws -> [String: Any] {
        var answer: [String: Any] = ["document": try Engine.sheet(for: url)]
        if let focus, !focus.isEmpty {
            answer["focus"] = focus
        }
        return answer
    }

    /// The block id a call names, or a failure saying it did not name one.
    private func id(_ call: [String: Any]) throws -> String {
        guard let id = call["id"] as? String, !id.isEmpty else {
            throw Engine.Failure(message: "that block has no id to edit")
        }
        return id
    }
}
