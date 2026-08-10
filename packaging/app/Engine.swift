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

    /// The exact characters of one block, for the reader to edit.
    ///
    /// Not the rendered HTML: by the time a paragraph is drawn, the difference between the
    /// text and the way it is set has already been spent.
    static func source(of block: String, in url: URL) throws -> String {
        try run(["source", url.path, "--block", block])
    }

    /// A field's text as the engine decorates it — marks styled in place, every byte kept.
    ///
    /// `dx render --field` reads no file: it is a pure function of the characters, called
    /// between keystrokes so the field stays dressed as what it says.
    static func decorate(_ text: String) throws -> String {
        try run(["render", "--field=" + text])
    }

    /// Execute one runnable block and let `dx run` write its output into the document.
    ///
    /// The only call in this file that runs code, and it happens because the reader asked
    /// for that one block by name. A failed block is still *written* — the failure lands in
    /// the document as an output block — so the caller decides what a thrown failure means
    /// by looking at whether the file changed.
    ///
    /// `--approve` is the click itself: the reader is looking at this block's code, on this
    /// page, and pressed `run` on it — that *is* the review the gate asks for, and it
    /// approves nothing else, because `--only` narrows the run to the block they pointed
    /// at. Without it every freshly typed block would be refused, since editing a block
    /// changes its fingerprint and a terminal is the last place a person writing on a page
    /// wants to be sent.
    static func execute(_ block: String, in url: URL) throws {
        try run(["run", url.path, "--only", block, "--approve"])
    }

    /// Replace one block's body, leaving every other block in the file byte-identical.
    static func set(_ block: String, in url: URL, to text: String) throws {
        try run(["set", url.path, block, "--text=" + text])
    }

    /// The canonical `::kind attrs` opening line of one block — what the editing surface
    /// shows in the header field above the body.
    static func header(of block: String, in url: URL) throws -> String {
        try run(["source", url.path, "--block", block, "--header"])
            .trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// Replace one block wholesale — header and body — and return the id the replacement
    /// answers to, which is where the reader's caret belongs afterwards. An empty header
    /// means the text is plain prose, read the way the file itself would read it.
    static func replace(_ block: String, in url: URL, header: String, body: String) throws
        -> String
    {
        try run(["set", url.path, block, "--header=" + header, "--text=" + body])
            .trimmingCharacters(in: .whitespacesAndNewlines)
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

    /// Tick or untick one box of a checklist, by its position counting from zero — the
    /// position the renderer writes on every mark, and the one `dx check` takes.
    static func check(_ block: String, in url: URL, item: Int) throws {
        try run(["check", url.path, block, "--item=\(item)"])
    }

    /// One board operation, through `dx board` — move or add a node, settle the whole board
    /// after a drag, take a node off, or draw and erase an edge between two named sides.
    /// Returns the id of a freshly added node (the block the reader's caret belongs in
    /// next), and nil for every other action.
    static func board(_ action: String, on board: String, in url: URL, spec: [String: Any])
        throws -> String?
    {
        func number(_ key: String) -> Int { (spec[key] as? NSNumber)?.intValue ?? 0 }
        func name(_ key: String) -> String { spec[key] as? String ?? "" }

        switch action {
        case "place", "add":
            var arguments = ["board", url.path, board]
            if action == "add" {
                arguments.append("--add")
            } else {
                arguments += ["--place", name("node")]
            }
            // `--x=` rather than `--x `, so a negative coordinate is a value and not a flag.
            arguments += ["--x=\(number("x"))", "--y=\(number("y"))"]
            if number("w") > 0 {
                arguments.append("--w=\(number("w"))")
            }
            if number("h") > 0 {
                arguments.append("--h=\(number("h"))")
            }
            let placed = try run(arguments).trimmingCharacters(in: .whitespacesAndNewlines)
            return action == "add" ? placed : nil
        case "arrange":
            // Several nodes' boxes in one call — the group a reader moved, or a whole board
            // an agent laid out. `dx board` settles what they landed on.
            try run(["board", url.path, board, "--arrange", name("arrangement")])
            return nil
        case "detach":
            try run(["board", url.path, board, "--detach", name("node")])
            return nil
        case "link", "unlink":
            var arguments = ["board", url.path, board, "--\(action)", name("from"), "--to", name("to")]
            // The sides the reader dragged between, when they chose any.
            if !name("fromSide").isEmpty {
                arguments += ["--from-side", name("fromSide")]
            }
            if !name("toSide").isEmpty {
                arguments += ["--to-side", name("toSide")]
            }
            try run(arguments)
            return nil
        default:
            throw Failure(message: "unknown board action `\(action)`")
        }
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
