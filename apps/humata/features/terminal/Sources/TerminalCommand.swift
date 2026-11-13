import ComposableArchitecture
import Foundation
import HumataDaemonClient

/// What a terminal's surface runs: a line for `/bin/sh -c`, as Ghostty runs
/// a surface's command, and what to add to its environment.
package struct TerminalCommand: Equatable, Sendable {
    package var line: String
    package var environment: [String: String]

    package init(line: String, environment: [String: String] = [:]) {
        self.line = line
        self.environment = environment
    }

    /// `humata terminal attach` for `terminal`, run from `executable`, which
    /// pipes the host's terminal through humatad on `socket`; nil leaves the
    /// command to find humatad where it does by default.
    package static func attach(_ terminal: Terminal, executable: String, socket: String?) -> TerminalCommand {
        var words = [executable, "terminal", "attach", terminal.id]
        if let host = terminal.host { words += ["--host", host] }
        return TerminalCommand(
            line: words.map(shellQuoted).joined(separator: " "),
            environment: socket.map { ["HUMATA_SOCKET": $0] } ?? [:])
    }

    /// `word` as one word of a shell's command line, whatever it holds.
    private static func shellQuoted(_ word: String) -> String {
        "'" + word.replacingOccurrences(of: "'", with: "'\\''") + "'"
    }
}

/// Which command draws a terminal. The app runs the `humata` inside it;
/// tests and previews run something harmless, and a flow can print a fixed
/// transcript.
package struct TerminalCommandClient: Sendable {
    package var command: @Sendable (_ terminal: Terminal) -> TerminalCommand

    package init(command: @escaping @Sendable (_ terminal: Terminal) -> TerminalCommand) {
        self.command = command
    }
}

extension TerminalCommandClient: DependencyKey {
    /// The `humata` in the app's Helpers, talking to the humatad the app
    /// itself talks to: `$HUMATA_SOCKET` when the app was given one, else
    /// where both look by default.
    package static var liveValue: TerminalCommandClient {
        let executable = Bundle.main.bundleURL.appending(path: "Contents/Helpers/humata").path
        let socket = ProcessInfo.processInfo.environment["HUMATA_SOCKET"].flatMap { $0.isEmpty ? nil : $0 }
        return TerminalCommandClient { .attach($0, executable: executable, socket: socket) }
    }

    package static var testValue: TerminalCommandClient {
        TerminalCommandClient { _ in TerminalCommand(line: "/bin/cat") }
    }

    package static var previewValue: TerminalCommandClient { testValue }
}

extension DependencyValues {
    package var terminalCommand: TerminalCommandClient {
        get { self[TerminalCommandClient.self] }
        set { self[TerminalCommandClient.self] = newValue }
    }
}
