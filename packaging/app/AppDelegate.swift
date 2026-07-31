import AppKit
import UniformTypeIdentifiers

/// The application, and which of its two roles a launch is.
///
/// `DX.app` is both the viewer for `.dx` documents and the thing a person double-clicks to
/// install `dx` on this Mac. Those are not modes a reader chooses and not two applications:
/// macOS hands an app the documents it was asked to open, so **a launch carrying a document is
/// a read, and a launch carrying nothing is the install** — which is exactly what
/// double-clicking `DX.app` has always done. One bundle, because a viewer that shipped
/// separately from the engine it renders through is a second thing to install and a second
/// thing to go out of date.
///
/// Reading never executes and never writes: opening a document runs `dx render` and nothing
/// else. The install only ever happens on a launch that opened no document at all.
final class AppDelegate: NSObject, NSApplicationDelegate {
    /// One per open window, held so the windows and their delegates stay alive.
    private var viewers: [DocumentViewer] = []

    /// Where the next window goes, so a second document does not land on the first.
    private var nextWindowCorner: NSPoint?

    // MARK: - The viewer

    /// Documents from Finder, `open`, a drag onto the icon, or a second double-click.
    func application(_ application: NSApplication, open urls: [URL]) {
        for url in urls {
            show(url)
        }
    }

    /// File ▸ Open… — the way to a document when the app is already running.
    @objc func openDocument(_ sender: Any?) {
        let panel = NSOpenPanel()
        if let document = UTType(filenameExtension: "dx") {
            panel.allowedContentTypes = [document]
        }
        panel.allowsMultipleSelection = true
        panel.begin { response in
            guard response == .OK else { return }
            for url in panel.urls {
                self.show(url)
            }
        }
    }

    /// Show `url`, or bring its window forward if it is already open.
    private func show(_ url: URL) {
        let standardized = url.standardizedFileURL
        if let open = viewers.first(where: { $0.url == standardized }) {
            open.show()
            return
        }
        let viewer = DocumentViewer(url: standardized) { [weak self] closed in
            self?.viewers.removeAll { $0 === closed }
        }
        viewers.append(viewer)
        nextWindowCorner = viewer.cascade(after: nextWindowCorner)
        viewer.show()
    }

    // MARK: - The install

    /// A launch that opened no document is the install.
    ///
    /// `application(_:open:)` is delivered during launch, before this, so an empty window list
    /// here means macOS handed this launch nothing to read.
    func applicationDidFinishLaunching(_ notification: Notification) {
        guard viewers.isEmpty else { return }
        install()
    }

    /// There is no such thing as an empty `.dx` window.
    ///
    /// Without this, AppKit offers an untitled document at launch and whenever the Dock icon is
    /// clicked with nothing open — and a window with no document to show is the blank window
    /// this viewer exists to avoid. A reader with nothing open uses File ▸ Open….
    func applicationShouldOpenUntitledFile(_ application: NSApplication) -> Bool {
        false
    }

    /// The viewer is one window per document, so the last one closing ends the app.
    func applicationShouldTerminateAfterLastWindowClosed(_ application: NSApplication) -> Bool {
        true
    }

    /// Run `dx setup` off the main thread, then say what happened and quit.
    private func install() {
        DispatchQueue.global(qos: .userInitiated).async {
            let outcome = Result { try Installer.install() }
            DispatchQueue.main.async {
                self.report(outcome)
                NSApp.terminate(nil)
            }
        }
    }

    /// The one dialog the install shows.
    private func report(_ outcome: Result<Installer.Report, Error>) {
        let alert = NSAlert()
        alert.messageText = "dx"
        switch outcome {
        case .success(let report):
            alert.informativeText = """
                dx is installed on this Mac, and .dx documents now open here.

                The rendering service is running at \(report.serving ?? "127.0.0.1"), and it \
                starts with your session from now on.

                Open Terminal and run 'dx help' to use it, or 'dx browser' for the one step \
                each browser reserves for you.
                """
            alert.addButton(withTitle: "Done")
            alert.addButton(withTitle: "Show report")
            if alert.runModal() == .alertSecondButtonReturn {
                NSWorkspace.shared.open(Installer.log)
            }
        case .failure(let error):
            let said = (error as? Engine.Failure)?.message ?? error.localizedDescription
            alert.alertStyle = .critical
            alert.informativeText = "dx could not finish installing.\n\n\(said)"
            alert.addButton(withTitle: "OK")
            alert.runModal()
        }
    }
}
