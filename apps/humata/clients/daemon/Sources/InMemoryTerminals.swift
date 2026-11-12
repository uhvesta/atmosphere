import Foundation

// Terminals: each runs a pretend shell that echoes what's typed and, on
// Enter, runs `echo`, `pwd` and `exit`; anything else isn't found.
extension InMemoryDaemon {
    struct FakeTerminal {
        var terminal: Terminal
        /// Everything it printed, replayed to each new watcher.
        var output = Data()
        /// What was typed since the last Enter.
        var line = ""
        var watchers: [UUID: AsyncThrowingStream<TerminalOutput, any Error>.Continuation] = [:]
    }

    private static let prompt = "$ "
    private static let home = "/Users/you"

    private static func unknownTerminal(_ terminal: String) -> DaemonError {
        .refused(.unknownTerminal, message: "There is no terminal \(terminal); it was closed. Open another.")
    }

    func openTerminal(_ place: HostPlace) throws -> Terminal {
        try state.withValue { state in
            var host = place.host
            var folder = place.folder ?? Self.home
            if let id = place.sessionID {
                guard let session = state.sessions.first(where: { $0.id == id }) else { throw Self.unknown(id) }
                host = session.host
                folder = session.workingDirectory
            }
            let terminal = Terminal(
                id: "t\(state.nextTerminal)", host: host, sessionID: place.sessionID, folder: folder,
                shell: "/bin/zsh", opened: start.addingTimeInterval(TimeInterval(state.nextTerminal)))
            state.nextTerminal += 1
            state.terminals[terminal.id] = FakeTerminal(terminal: terminal, output: Data(Self.prompt.utf8))
            return terminal
        }
    }

    func terminals(on host: String?) -> [Terminal] {
        state.value.terminals.values.map(\.terminal).filter { $0.host == host }.sorted { $0.opened < $1.opened }
    }

    @discardableResult
    func terminal(_ id: String) throws -> Terminal {
        guard let terminal = state.value.terminals[id]?.terminal else { throw Self.unknownTerminal(id) }
        return terminal
    }

    func type(_ data: Data, in id: String) throws {
        try state.withValue { state in
            guard var terminal = state.terminals[id] else { throw Self.unknownTerminal(id) }
            guard terminal.terminal.exitCode == nil else { return }
            var printed = ""
            var exit: Int?
            for character in String(bytes: data, encoding: .utf8) ?? "" where exit == nil {
                switch character {
                case "\r", "\n":
                    printed += "\r\n"
                    let (said, code) = Self.run(terminal.line, in: terminal.terminal.folder)
                    printed += said
                    exit = code
                    terminal.line = ""
                    if exit == nil { printed += Self.prompt }
                case "\u{7f}":
                    if !terminal.line.isEmpty {
                        terminal.line.removeLast()
                        printed += "\u{8} \u{8}"
                    }
                default:
                    terminal.line.append(character)
                    printed.append(character)
                }
            }
            let output = TerminalOutput(data: Data(printed.utf8))
            terminal.output.append(output.data)
            for watcher in terminal.watchers.values {
                watcher.yield(output)
            }
            if let exit {
                terminal.terminal.exitCode = exit
                for watcher in terminal.watchers.values {
                    watcher.yield(TerminalOutput(data: Data(), exitCode: exit))
                    watcher.finish()
                }
                terminal.watchers = [:]
            }
            state.terminals[id] = terminal
        }
    }

    /// What the pretend shell prints for `line`, and its exit code if the
    /// line ended it.
    private static func run(_ line: String, in folder: String) -> (String, Int?) {
        let words = line.split(separator: " ").map(String.init)
        guard let command = words.first else { return ("", nil) }
        switch command {
        case "echo": return (words.dropFirst().joined(separator: " ") + "\r\n", nil)
        case "pwd": return (folder + "\r\n", nil)
        case "exit": return ("", words.dropFirst().first.flatMap { Int($0) } ?? 0)
        default: return ("zsh: command not found: \(command)\r\n", nil)
        }
    }

    func closeTerminal(_ id: String) throws {
        try state.withValue { state in
            guard let terminal = state.terminals.removeValue(forKey: id) else { throw Self.unknownTerminal(id) }
            for watcher in terminal.watchers.values {
                watcher.yield(TerminalOutput(data: Data(), exitCode: terminal.terminal.exitCode ?? -1))
                watcher.finish()
            }
        }
    }

    func attachTerminal(_ id: String) -> AsyncThrowingStream<TerminalOutput, any Error> {
        AsyncThrowingStream { continuation in
            let watcher = UUID()
            let found = state.withValue { state -> Bool in
                guard let terminal = state.terminals[id] else { return false }
                continuation.yield(TerminalOutput(data: terminal.output, replay: true))
                if let code = terminal.terminal.exitCode {
                    continuation.yield(TerminalOutput(data: Data(), exitCode: code))
                    continuation.finish()
                } else {
                    state.terminals[id]?.watchers[watcher] = continuation
                }
                return true
            }
            guard found else {
                continuation.finish(throwing: Self.unknownTerminal(id))
                return
            }
            continuation.onTermination = { _ in
                self.state.withValue { $0.terminals[id]?.watchers[watcher] = nil }
            }
        }
    }
}
