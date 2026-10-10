// Spike S5: a stand-in for the GUI that is a real AppKit application.
//
// The first S5 run used a plain executable as the bundle's main program. It had no window
// and was not an AppKit app, so a system permission dialog may never have been offered for
// it. This program is the bundle's main executable instead. It opens a window, starts
// `s5-host` (which starts the nested `denon-avr-api-server` and checks that the server can
// reach the receiver), and shows what the helper prints, as it prints it. The chain is
// app -> s5-host -> server, so the application macOS holds responsible for the server's
// network use is this one, as it will be for the real GUI.
//
// Three runs lost the permission dialogs, because they were remembered and not recorded. The
// strip at the bottom of the window is for that: three buttons that write a time-stamped line
// ("a dialog appeared", "I clicked Allow", "I clicked Don't Allow"), and a box for the dialog's
// exact name. They write to `notes-<label>.txt`, a file of their own, so that they never
// compete with the helper for its report. The times are wall-clock times, like the ones the
// helper prints on its attempt lines.
//
// Built by tools/s5/build-bundles.sh. See docs/research/local-network-permission-server-macos.md.

import AppKit
import Foundation

/// A button that works on the first click even while another program's dialog has the focus.
final class FirstClickButton: NSButton {
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }
}

final class Controller: NSObject, NSApplicationDelegate {
    private var window: NSWindow!
    private var textView: NSTextView!
    private var noteField: NSTextField!
    private var child: Process?
    private let label: String
    private let helper: URL
    private let root: String
    private let started = Date()
    private static let clockFormat: DateFormatter = {
        let format = DateFormatter()
        format.dateFormat = "HH:mm:ss"
        return format
    }()

    override init() {
        let executable = Bundle.main.executableURL ?? URL(fileURLWithPath: CommandLine.arguments[0])
        helper = executable.deletingLastPathComponent().appendingPathComponent("s5-host")
        label = Controller.label(from: Bundle.main.bundleURL)
        root = ProcessInfo.processInfo.environment["S5_ROOT"] ?? "/tmp/s5"
        super.init()
    }

    private var notesPath: String { "\(root)/notes-\(label).txt" }

    /// `.../S5-same.app` is the case `same`. A bundle in a directory whose name ends in `-copy`
    /// (the install-flow test's copy) gets `-copy` added, as the helper does, so that its report,
    /// data directory and notes are not the original's, which has the same file name.
    static func label(from bundle: URL) -> String {
        let name = bundle.deletingPathExtension().lastPathComponent
        let label = name.hasPrefix("S5-") ? String(name.dropFirst(3)) : name
        let copied = bundle.deletingLastPathComponent().lastPathComponent.hasSuffix("-copy")
        return copied ? label + "-copy" : label
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        let barHeight: CGFloat = 76
        let frame = NSRect(x: 160, y: 160, width: 860, height: 560)
        window = NSWindow(
            contentRect: frame,
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.title = "S5 \(label)"
        let content = window.contentView!
        let width = content.bounds.width

        let scroll = NSScrollView(
            frame: NSRect(x: 0, y: barHeight, width: width, height: content.bounds.height - barHeight))
        scroll.autoresizingMask = [.width, .height]
        scroll.hasVerticalScroller = true
        textView = NSTextView(frame: scroll.bounds)
        textView.autoresizingMask = [.width]
        textView.isEditable = false
        textView.font = NSFont.monospacedSystemFont(ofSize: 12, weight: .regular)
        scroll.documentView = textView
        content.addSubview(scroll)

        content.addSubview(makeButton("A dialog appeared", x: 12, y: 42, width: 170, #selector(markAppeared)))
        content.addSubview(makeButton("I clicked Allow", x: 190, y: 42, width: 150, #selector(markAllow)))
        content.addSubview(makeButton("I clicked Don't Allow", x: 348, y: 42, width: 190, #selector(markDeny)))
        noteField = NSTextField(frame: NSRect(x: 12, y: 10, width: width - 24 - 96, height: 24))
        noteField.placeholderString = "The dialog's exact name, or anything surprising; press Return to save"
        noteField.autoresizingMask = [.width, .maxYMargin]
        noteField.target = self
        noteField.action = #selector(saveNote(_:))
        content.addSubview(noteField)
        let save = makeButton("Save note", x: width - 100, y: 8, width: 88, #selector(saveNote(_:)))
        save.autoresizingMask = [.minXMargin, .maxYMargin]
        content.addSubview(save)

        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        append("S5 \(label): starting the helper, which starts the nested server.\n")
        append("This bundle is at \(Bundle.main.bundleURL.path)\n")
        append("Leave this window open until it says FINISHED. If a permission dialog appears:\n")
        append("  1. press \"A dialog appeared\" here, 2. read the dialog's exact name, 3. click Allow\n")
        append("  in the dialog (never Don't Allow), 4. press \"I clicked Allow\" here, 5. only then\n")
        append("  click the box below, type the dialog's exact name and press Return.\n")
        append("Those lines go to \(notesPath).\n\n")
        note("run started (\(label))")
        run()
    }

    private func makeButton(_ title: String, x: CGFloat, y: CGFloat, width: CGFloat, _ action: Selector)
        -> NSButton
    {
        let button = FirstClickButton(frame: NSRect(x: x, y: y, width: width, height: 28))
        button.title = title
        button.bezelStyle = .rounded
        button.target = self
        button.action = action
        button.autoresizingMask = [.maxXMargin, .maxYMargin]
        return button
    }

    @objc private func markAppeared(_ sender: Any?) { note("DIALOG appeared") }
    @objc private func markAllow(_ sender: Any?) { note("clicked ALLOW") }
    @objc private func markDeny(_ sender: Any?) { note("clicked DON'T ALLOW") }

    @objc private func saveNote(_ sender: Any?) {
        let text = noteField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        note("TEXT: \(text)")
        noteField.stringValue = ""
    }

    /// One line in the window and in the notes file: wall-clock time, seconds since the window
    /// opened, and the text. A failed write is said in the window, not swallowed.
    private func note(_ text: String) {
        let now = Date()
        let seconds = Int(now.timeIntervalSince(started))
        let line = "\(Controller.clockFormat.string(from: now)) (+\(seconds) s) \(text)\n"
        append("[note] \(line)")
        do {
            try FileManager.default.createDirectory(atPath: root, withIntermediateDirectories: true)
            let url = URL(fileURLWithPath: notesPath)
            if let handle = try? FileHandle(forWritingTo: url) {
                defer { try? handle.close() }
                try handle.seekToEnd()
                try handle.write(contentsOf: Data(line.utf8))
            } else {
                try Data(line.utf8).write(to: url)
            }
        } catch {
            append("[note] COULD NOT WRITE \(notesPath): \(error)\n")
        }
    }

    private func run() {
        let process = Process()
        process.executableURL = helper
        process.standardInput = FileHandle.nullDevice
        let pipe = Pipe()
        process.standardOutput = pipe
        process.standardError = pipe
        pipe.fileHandleForReading.readabilityHandler = { [weak self] handle in
            let data = handle.availableData
            guard !data.isEmpty, let text = String(data: data, encoding: .utf8) else { return }
            DispatchQueue.main.async { self?.append(text) }
        }
        process.terminationHandler = { [weak self] finished in
            DispatchQueue.main.async {
                guard let self = self else { return }
                self.append(
                    "\nFINISHED (helper exit status \(finished.terminationStatus)). "
                        + "The report is \(self.root)/result-\(self.label).txt and your notes are in "
                        + "\(self.notesPath). Write the dialog's name in the box if you have not, "
                        + "then close this window.\n")
            }
        }
        do {
            try process.run()
            child = process
        } catch {
            append("Cannot start \(helper.path): \(error)\n")
        }
    }

    private func append(_ text: String) {
        let attributes: [NSAttributedString.Key: Any] = [.font: textView.font!]
        textView.textStorage?.append(NSAttributedString(string: text, attributes: attributes))
        textView.scrollToEndOfDocument(nil)
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }

    func applicationWillTerminate(_ notification: Notification) {
        // The helper's server sees its parent go and shuts down.
        if let child = child, child.isRunning { child.terminate() }
    }
}

let application = NSApplication.shared
application.setActivationPolicy(.regular)
let controller = Controller()
application.delegate = controller
application.run()
