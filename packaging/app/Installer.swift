import Foundation

/// The install: `dx setup`, run once, from the application a person just double-clicked.
///
/// This is the role `DX.app` has always had, and it is unchanged — the app is still the only
/// thing anyone has to open to make this Mac understand `.dx`. What is new is that opening a
/// *document* is a different launch (see ``AppDelegate``), so the two never collide.
enum Installer {
    /// What one install produced, once it worked.
    struct Report {
        /// Where the rendering service ended up listening, when it answered in time.
        let serving: String?
        /// The file the full report was written to.
        let log: URL
    }

    /// Where the full output goes.
    ///
    /// A dialog is the wrong place to read forty lines and the right place to learn whether it
    /// worked, so the dialog says the one sentence and this holds the rest.
    static var log: URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Logs/dx/setup.log")
    }

    /// Run `dx setup` and record what it said.
    ///
    /// - Throws: ``Engine/Failure`` when `dx` could not be started or refused to install, with
    ///   the sentence it printed.
    static func install() throws -> Report {
        let text: String
        do {
            text = try Engine.run(["setup"])
        } catch let failure as Engine.Failure {
            write(failure.message)
            throw failure
        }
        write(text)
        return Report(serving: serving(in: text), log: log)
    }

    /// The address `dx setup` reported the rendering service at, if it reported one.
    private static func serving(in report: String) -> String? {
        for line in report.split(separator: "\n") where line.contains("serving") {
            if let start = line.range(of: "http://") {
                return String(line[start.lowerBound...]).trimmingCharacters(in: .whitespaces)
            }
        }
        return nil
    }

    /// Write the full report where the reader can open it, ignoring a failure to do so.
    ///
    /// A log that cannot be written is not a reason to fail an install that worked — the
    /// dialog still says what happened, which is the part that matters.
    private static func write(_ text: String) {
        let directory = log.deletingLastPathComponent()
        try? FileManager.default.createDirectory(
            at: directory, withIntermediateDirectories: true)
        try? text.write(to: log, atomically: true, encoding: .utf8)
    }
}
