import AppKit

// The whole application in five lines, because there is no nib to do it: an application
// object, the menu bar, and a delegate that decides — from what macOS hands this launch —
// whether it is a document to read or the install to run. See `AppDelegate`.
//
// The delegate is a top-level binding on purpose: NSApplication holds its delegate weakly, and
// a local would be released before the first event arrived.
let application = NSApplication.shared
let delegate = AppDelegate()
application.setActivationPolicy(.regular)
application.mainMenu = MainMenu.build()
application.delegate = delegate
application.run()
