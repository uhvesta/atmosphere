import Foundation

// A file's versions: the session's folder as each turn left it, from the
// writes in its copy; turn 0 is before the first turn.
extension InMemoryDaemon {
    func fileVersions(_ session: String, _ path: String) throws -> [FileVersion] {
        var versions: [FileVersion] = []
        var last: String?
        for (turn, folder) in try folderByTurn(session).enumerated() {
            guard let text = folder.files[path], text != last else { continue }
            versions.append(FileVersion(turn: turn, time: folder.time))
            last = text
        }
        return versions
    }

    func readFileVersion(_ session: String, _ path: String, _ turn: Int) throws -> FileContents {
        let turns = try folderByTurn(session)
        guard turns.indices.contains(turn) else {
            throw DaemonError.refused(
                .invalidRequest,
                message: "Session \(session) has had \(turns.count - 1) turns, so it has no turn \(turn).")
        }
        guard let text = turns[turn].files[path] else {
            throw DaemonError.refused(.invalidRequest, message: "\(path) wasn't there after turn \(turn).")
        }
        return FileContents(path: path, text: text)
    }

    func restoreFileVersion(_ session: String, _ path: String, _ turn: Int) throws -> FileContents {
        let version = try readFileVersion(session, path, turn)
        state.withValue { state in
            guard let copy = state.copyOf[session] else { return }
            // After every event so far: the next turn's checkpoint keeps it.
            let last = state.histories[session]?.last?.id ?? ""
            state.copies[copy, default: []].append(
                Write(event: Self.restoredAfter + last, path: path, contents: version.text))
        }
        return version
    }

    /// Marks a restore's write, by the event it came after.
    private static let restoredAfter = "restored-after:"

    /// The session's folder after each turn, by path from it, with when the
    /// turn ended.
    private func folderByTurn(_ session: String) throws -> [(time: Date, files: [String: String])] {
        try known(session)
        return state.withValue { state in
            let history = state.histories[session] ?? []
            let writes = state.copyOf[session].flatMap { state.copies[$0] } ?? []
            let directory = state.sessions.first { $0.id == session }?.workingDirectory ?? ""
            let prefix = directory.hasSuffix("/") ? directory : directory + "/"
            var turnOf: [String: Int] = [:]
            var times: [Date] = [history.first?.time ?? Date(timeIntervalSince1970: 0)]
            for event in history {
                if case .turnStarted = event.kind { times.append(event.time) }
                times[times.count - 1] = event.time
                turnOf[event.id] = times.count - 1
            }
            func snapshotTurn(_ write: Write) -> Int {
                if let turn = turnOf[write.event] { return turn }
                guard write.event.hasPrefix(Self.restoredAfter) else { return times.count }
                // Written between turns: the next one's snapshot has it.
                return (turnOf[String(write.event.dropFirst(Self.restoredAfter.count))] ?? -1) + 1
            }
            return times.indices.map { turn in
                let written = Self.contents(after: writes.filter { snapshotTurn($0) <= turn })
                let files = written.reduce(into: [String: String]()) { files, file in
                    if !file.key.hasPrefix("/") {
                        files[file.key] = file.value
                    } else if file.key.hasPrefix(prefix) {
                        files[String(file.key.dropFirst(prefix.count))] = file.value
                    }
                }
                return (times[turn], files)
            }
        }
    }
}
