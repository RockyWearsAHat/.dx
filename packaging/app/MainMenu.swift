import AppKit

/// The menu bar, built in code because the app has no nib.
///
/// It is the shortest menu that still behaves like a Mac application: the keystrokes a reader
/// — and now a writer — will reach for without thinking (⌘Q, ⌘W, ⌘O, ⌘R, and the whole editing
/// set) and nothing invented on top. Every item's target is `nil`, so it travels the responder
/// chain — which is how View ▸ Reload reaches the window's own ``DocumentViewer`` and
/// File ▸ Open… reaches the ``AppDelegate``.
enum MainMenu {
    /// The application's whole menu bar.
    static func build() -> NSMenu {
        let bar = NSMenu()
        bar.addItem(submenu(named: "dx", items: applicationItems()))
        bar.addItem(submenu(named: "File", items: fileItems()))
        bar.addItem(submenu(named: "Edit", items: editItems()))
        bar.addItem(submenu(named: "View", items: viewItems()))
        bar.addItem(submenu(named: "Window", items: windowItems()))
        return bar
    }

    private static func applicationItems() -> [NSMenuItem] {
        [
            item("About dx", #selector(NSApplication.orderFrontStandardAboutPanel(_:)), ""),
            .separator(),
            item("Hide dx", #selector(NSApplication.hide(_:)), "h"),
            item("Quit dx", #selector(NSApplication.terminate(_:)), "q"),
        ]
    }

    private static func fileItems() -> [NSMenuItem] {
        [
            item("Open…", #selector(AppDelegate.openDocument(_:)), "o"),
            item("Close", #selector(NSWindow.performClose(_:)), "w"),
        ]
    }

    /// The full editing set, because the viewer became an editor. A keystroke only reaches
    /// the `WKWebView` through a menu key equivalent, so a missing Paste item is a ⌘V that
    /// does nothing at all — this menu held Copy and Select All from the read-only days.
    /// Undo/Redo are named by string: `NSResponder` has no typed selector for them, and the
    /// web view (and any field editor) answers both on the responder chain.
    private static func editItems() -> [NSMenuItem] {
        [
            item("Undo", Selector(("undo:")), "z"),
            item("Redo", Selector(("redo:")), "Z"),
            .separator(),
            item("Cut", #selector(NSText.cut(_:)), "x"),
            item("Copy", #selector(NSText.copy(_:)), "c"),
            item("Paste", #selector(NSText.paste(_:)), "v"),
            item("Select All", #selector(NSText.selectAll(_:)), "a"),
        ]
    }

    /// Reload is here because a document on disk changes while a window is open — someone
    /// edits it in an editor, or an agent writes to it — and a viewer that cannot ask again is
    /// a viewer showing yesterday.
    private static func viewItems() -> [NSMenuItem] {
        [
            item("Reload", #selector(DocumentViewer.reloadDocument(_:)), "r"),
            item("Actual Size", #selector(DocumentViewer.actualSize(_:)), "0"),
        ]
    }

    private static func windowItems() -> [NSMenuItem] {
        [
            item("Minimize", #selector(NSWindow.performMiniaturize(_:)), "m"),
            item("Zoom", #selector(NSWindow.performZoom(_:)), ""),
        ]
    }

    /// A top-level menu holding `items`.
    private static func submenu(named title: String, items: [NSMenuItem]) -> NSMenuItem {
        let holder = NSMenuItem()
        let menu = NSMenu(title: title)
        for entry in items {
            menu.addItem(entry)
        }
        holder.submenu = menu
        return holder
    }

    /// One menu item, aimed at the responder chain rather than a fixed object.
    private static func item(_ title: String, _ action: Selector, _ key: String) -> NSMenuItem {
        NSMenuItem(title: title, action: action, keyEquivalent: key)
    }
}
