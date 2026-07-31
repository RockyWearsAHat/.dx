import Foundation

/// The `dx` binary this application carries, and the only way the app produces a view.
///
/// The rule the whole project rests on is that there is **one engine**: if the editor, the
/// command line, and this window could render differently, they will. So the app renders
/// nothing itself — it asks `dx render` for the page and shows what comes back. The binary is
/// the one *inside this bundle*, never one found on `PATH`, so the window and the engine that
/// drew it are the same build and cannot drift apart.
enum Engine {
    /// What went wrong asking the engine for something.
    ///
    /// `message` is meant to be shown to a person as-is: `dx` fails with a sentence saying what
    /// to do about it, and rewording that here would only lose the instruction.
    struct Failure: Error {
        /// The sentence to show.
        let message: String
    }

    /// The `dx` binary beside this application's own executable.
    ///
    /// `Contents/MacOS/dx`, reached from the running executable rather than by name, so a
    /// bundle copied anywhere still finds its own copy.
    static var binary: URL {
        let executable = Bundle.main.executableURL ?? Bundle.main.bundleURL
        return executable.deletingLastPathComponent().appendingPathComponent("dx")
    }

    /// The document at `url` as a self-contained HTML page.
    ///
    /// Reading never executes and never writes: `dx render` parses, resolves the pointer
    /// through the store, and prints a page. It runs no code block and touches no file.
    ///
    /// - Throws: ``Failure`` carrying what `dx` said — a missing document, an unreadable
    ///   pointer, a parse error — so the window can show that sentence instead of nothing.
    static func page(for url: URL) throws -> String {
        try run(["render", url.path, "--theme", "auto"])
    }

    /// The document's blocks, without the page around them.
    ///
    /// What an edit hands back: the window replaces this one element instead of loading a
    /// page again, so the sheet a reader is writing on never blinks and never scrolls away
    /// from them.
    static func sheet(for url: URL) throws -> String {
        try run(["render", url.path, "--theme", "auto", "--fragment"])
    }

    /// One block as the page would draw it, with `body` in it — saving nothing.
    ///
    /// This is what keeps a document rendered while it is written on: the reader types `.dx`
    /// source, and the block beside the field shows what that source says. `dx render` is a
    /// read, so nothing here writes to the file the reader has not finished with.
    static func draw(_ block: String, in url: URL, as body: String) throws -> String {
        // `--body=` rather than `--body `, so a line starting with `-` is a line and not a
        // flag. The same reason `set` passes its text that way.
        try run(["render", url.path, "--theme", "auto", "--block", block, "--body=" + body])
    }

    /// The exact characters of one block, for the reader to edit.
    ///
    /// Not the rendered HTML: by the time a paragraph is drawn, the difference between the
    /// text and the way it is set has already been spent.
    static func source(of block: String, in url: URL) throws -> String {
        try run(["source", url.path, "--block", block])
    }

    /// Replace one block's body, leaving every other block in the file byte-identical.
    static func set(_ block: String, in url: URL, to text: String) throws {
        try run(["set", url.path, block, "--text=" + text])
    }

    /// Add a paragraph after `block` and return the id it was given.
    static func insert(after block: String, in url: URL) throws -> String {
        try run(["insert", url.path, "--after", block]).trimmingCharacters(
            in: .whitespacesAndNewlines)
    }

    /// Take one block out of the document.
    static func remove(_ block: String, in url: URL) throws {
        try run(["remove", url.path, block])
    }

    /// Run the bundled `dx` with `arguments` and return everything it printed.
    ///
    /// - Throws: ``Failure`` when the binary is missing, cannot be started, or exits non-zero.
    @discardableResult
    static func run(_ arguments: [String]) throws -> String {
        guard FileManager.default.isExecutableFile(atPath: binary.path) else {
            throw Failure(
                message:
                    "This copy of DX.app has no dx binary at \(binary.path). "
                    + "Rebuild it with packaging/build-app.sh.")
        }

        let process = Process()
        process.executableURL = binary
        process.arguments = arguments
        let output = Pipe()
        let errors = Pipe()
        process.standardOutput = output
        process.standardError = errors
        process.standardInput = FileHandle.nullDevice

        // Both pipes are drained on their own queues. Reading one to the end first deadlocks
        // as soon as the other fills its buffer, and a long parse error is easily that large.
        var printed = Data()
        var complaint = Data()
        let draining = DispatchGroup()
        draining.enter()
        DispatchQueue.global(qos: .userInitiated).async {
            printed = output.fileHandleForReading.readDataToEndOfFile()
            draining.leave()
        }
        draining.enter()
        DispatchQueue.global(qos: .userInitiated).async {
            complaint = errors.fileHandleForReading.readDataToEndOfFile()
            draining.leave()
        }

        do {
            try process.run()
        } catch {
            throw Failure(message: "Could not run \(binary.path): \(error.localizedDescription)")
        }
        process.waitUntilExit()
        draining.wait()

        let text = String(decoding: printed, as: UTF8.self)
        guard process.terminationStatus == 0 else {
            let said = String(decoding: complaint, as: UTF8.self).trimmingCharacters(
                in: .whitespacesAndNewlines)
            throw Failure(message: said.isEmpty ? "dx \(arguments[0]) failed." : said)
        }
        return text
    }
}
