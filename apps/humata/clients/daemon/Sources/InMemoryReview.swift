import Foundation

// Review: the copy's changes over a range, staging, and threads of comments
// sent to the agent as one message. Every copy starts empty, so each file is
// new in it; staging and reverting a hunk work on its whole file.
extension InMemoryDaemon {
    struct Thread {
        var session: String
        var thread: ReviewThread
    }

    struct Review {
        var id: Int
        var session: String
        var threads: [String]
        var message: String
    }

    /// The files after `writes`, by path.
    static func contents(after writes: [Write]) -> [String: String] {
        var files: [String: String] = [:]
        for write in writes {
            files[write.path] = write.contents
        }
        return files
    }

    func changes(_ session: String, _ query: ChangesQuery) throws -> [RepositoryChanges] {
        try known(session)
        return try state.withValue { state in
            let (before, after) = try Self.ends(of: query.range, session: session, in: state)
            let viewed = state.viewed[session] ?? [:]
            var paths = Set(before.keys).union(after.keys).sorted()
            if let file = query.file {
                guard file.repository == Self.repository else { throw Self.unknownRepository(file.repository) }
                guard before[file.path] != nil || after[file.path] != nil else {
                    throw DaemonError.refused(
                        .invalidRequest, message: "There is no \(file.path) in the session's copy of the code.")
                }
                paths = [file.path]
            }
            let files = paths.compactMap { path -> FileDiff? in
                let old = before[path]
                let new = after[path]
                guard old != new || query.wholeFile else { return nil }
                let status: FileStatus =
                    switch (old, new) {
                    case (nil, _): .added
                    case (_, nil): .deleted
                    case (let old, let new) where old == new: .unchanged
                    default: .modified
                    }
                let context = query.wholeFile ? Int.max : query.context
                let blob = new.map(Self.blob) ?? ""
                return FileDiff(
                    path: path,
                    status: status,
                    blob: blob,
                    hunks: LineDiff.hunks(Self.lines(old), Self.lines(new), context: context),
                    viewed: !blob.isEmpty && viewed[FileRef(repository: Self.repository, path: path)] == blob
                )
            }
            return [RepositoryChanges(name: Self.repository, patch: Self.patch(files), files: files)]
        }
    }

    func editChanges(_ session: String, _ edit: ChangeEdit) throws {
        try known(session)
        try state.withValue { state in
            guard edit.file.repository == Self.repository else { throw Self.unknownRepository(edit.file.repository) }
            guard let copy = state.copyOf[session] else { throw Self.noCopy(session) }
            let path = edit.file.path
            let working = Self.contents(after: state.copies[copy] ?? [])[path]
            let index = Self.index(of: copy, in: state)[path]
            let changed =
                switch edit.action {
                case .stage, .revert: working != index
                case .unstage: index != nil
                }
            guard changed else {
                throw DaemonError.refused(
                    .changed, message: "\(path) has no such change any more; read the changes again.")
            }
            switch edit.action {
            case .stage: state.staged[copy, default: [:]][path] = .some(working)
            case .unstage: state.staged[copy]?[path] = nil
            case .revert:
                let event = state.histories[session]?.last?.id ?? ""
                state.copies[copy, default: []].append(Write(event: event, path: path, contents: index))
            }
        }
    }

    func markViewed(_ session: String, _ file: FileRef, _ blob: String, _ viewed: Bool) throws {
        try known(session)
        state.withValue { state in
            state.viewed[session, default: [:]][file] = viewed ? blob : nil
        }
    }

    func createThread(_ session: String, _ new: NewThread) throws -> ReviewThread {
        try known(session)
        return try state.withValue { state in
            guard new.file.repository == Self.repository else { throw Self.unknownRepository(new.file.repository) }
            guard let copy = state.copyOf[session] else { throw Self.noCopy(session) }
            let lines = Self.lines(Self.contents(after: state.copies[copy] ?? [])[new.file.path])
            guard new.startLine >= 1, new.startLine <= new.endLine, new.endLine <= lines.count else {
                throw DaemonError.refused(
                    .invalidRequest,
                    message: "\(new.file.path) has \(lines.count) lines, so lines \(new.startLine)-\(new.endLine) "
                        + "aren't in it; read its changes again and comment on a line shown.")
            }
            let thread = ReviewThread(
                id: "\(state.nextThread)",
                file: new.file,
                startLine: new.startLine,
                endLine: new.endLine,
                code: Array(lines[(new.startLine - 1)..<new.endLine]),
                quote: new.quote,
                comments: [comment(new.text, in: &state)]
            )
            state.nextThread += 1
            state.threads.append(Thread(session: session, thread: thread))
            return thread
        }
    }

    func replyToThread(_ id: String, _ text: String) throws -> ReviewThread {
        let session = try state.withValue { state in
            guard let index = state.threads.firstIndex(where: { $0.thread.id == id }) else {
                throw Self.unknownThread(id)
            }
            state.threads[index].thread.comments.append(comment(text, in: &state))
            return state.threads[index].session
        }
        return try shown(id, in: session)
    }

    func resolveThread(_ id: String, _ resolved: Bool) throws -> ReviewThread {
        let session = try state.withValue { state in
            guard let index = state.threads.firstIndex(where: { $0.thread.id == id }) else {
                throw Self.unknownThread(id)
            }
            state.threads[index].thread.resolved = resolved
            return state.threads[index].session
        }
        return try shown(id, in: session)
    }

    func deleteComment(_ id: String) throws {
        try state.withValue { state in
            guard
                let index = state.threads.firstIndex(where: { thread in
                    thread.thread.comments.contains { $0.id == id && $0.author == .you }
                }),
                let comment = state.threads[index].thread.comments.firstIndex(where: { $0.id == id })
            else {
                throw DaemonError.refused(
                    .invalidRequest, message: "There is no comment \(id) of yours; list the threads to find it.")
            }
            guard !state.threads[index].thread.comments[comment].sent else {
                throw DaemonError.refused(
                    .invalidRequest, message: "Comment \(id) was sent to the agent already, so it stays.")
            }
            state.threads[index].thread.comments.remove(at: comment)
            if state.threads[index].thread.comments.isEmpty { state.threads.remove(at: index) }
        }
    }

    /// The session's threads where their lines are now, with the agent's
    /// answers.
    func threads(_ session: String) throws -> [ReviewThread] {
        try known(session)
        return state.withValue { state in
            let files = Self.contents(after: state.copyOf[session].flatMap { state.copies[$0] } ?? [])
            let reviews = state.reviews.filter { $0.session == session }
            let replies = Self.replies(in: state.histories[session] ?? [], to: reviews)
            for index in state.threads.indices where state.threads[index].session == session {
                Self.anchor(&state.threads[index].thread, in: Self.lines(files[state.threads[index].thread.file.path]))
            }
            return state.threads.filter { $0.session == session }.map { stored in
                var thread = stored.thread
                thread.comments += replies[thread.id] ?? []
                thread.comments.sort { $0.time < $1.time }
                return thread
            }
        }
    }

    /// Sends every comment not sent yet, on threads not resolved, as one
    /// message.
    func sendReview(_ session: String) throws -> Int {
        try known(session)
        _ = try threads(session)
        let (message, count) = try state.withValue { state -> (String, Int) in
            var message = Self.reviewIntroduction
            var count = 0
            var threads: [String] = []
            for index in state.threads.indices where state.threads[index].session == session {
                let thread = state.threads[index].thread
                let pending = thread.comments.filter { $0.author == .you && !$0.sent }
                guard !thread.resolved, !pending.isEmpty else { continue }
                let lines =
                    thread.startLine == thread.endLine
                    ? "line \(thread.startLine)" : "lines \(thread.startLine)-\(thread.endLine)"
                message += "\n[thread \(thread.id)] \(thread.file.repository)/\(thread.file.path), \(lines):\n```\n"
                message += thread.code.map { $0 + "\n" }.joined() + "```\n"
                if !thread.quote.isEmpty { message += "On the text \"\(thread.quote)\":\n" }
                message += pending.map { $0.text.trimmingCharacters(in: .whitespacesAndNewlines) + "\n" }.joined()
                for comment in state.threads[index].thread.comments.indices {
                    state.threads[index].thread.comments[comment].sent = true
                }
                count += pending.count
                threads.append(thread.id)
            }
            guard count > 0 else {
                throw DaemonError.refused(
                    .invalidRequest, message: "There are no comments to send; leave one on a line first.")
            }
            let review = Review(id: state.reviews.count + 1, session: session, threads: threads, message: message)
            state.reviews.append(review)
            return (message, count)
        }
        try send(session, message, .enqueue)
        return count
    }
}

extension InMemoryDaemon {
    /// The one repository an in-memory copy holds.
    fileprivate static let repository = "code"

    fileprivate static let reviewIntroduction =
        "Here is my review of your changes, as comments on lines of the code. Make the changes they ask for, and "
        + "answer each thread on its own: start the answer with the thread's tag on a line of its own, like this:"
        + "\n\n[thread 1]\nWhat you did about it, or why not.\n"

    fileprivate static func unknownRepository(_ name: String) -> DaemonError {
        .refused(.invalidRequest, message: "The session has no repository \(name); its only one is \(repository).")
    }

    fileprivate static func noCopy(_ session: String) -> DaemonError {
        .refused(.invalidRequest, message: "Session \(session) has no copy of the code; start it in a project.")
    }

    fileprivate static func unknownThread(_ thread: String) -> DaemonError {
        .refused(.unknownThread, message: "There is no review thread \(thread); list the threads to find its id.")
    }

    fileprivate func comment(_ text: String, in state: inout State) -> ReviewComment {
        defer { state.nextComment += 1 }
        return ReviewComment(
            id: "c\(state.nextComment)", author: .you, text: text, sent: false,
            time: start + Double(state.nextEvent) + Double(state.nextComment) / 1000)
    }

    fileprivate func shown(_ id: String, in session: String) throws -> ReviewThread {
        guard let thread = try threads(session).first(where: { $0.id == id }) else { throw Self.unknownThread(id) }
        return thread
    }

    /// The files at the range's two ends.
    fileprivate static func ends(
        of range: ChangeRange, session: String, in state: State
    ) throws -> (before: [String: String], after: [String: String]) {
        guard let copy = state.copyOf[session] else { return ([:], [:]) }
        let writes = state.copies[copy] ?? []
        let working = contents(after: writes)
        switch range {
        case .session, .branch: return ([:], working)
        case .unstaged: return (index(of: copy, in: state), working)
        case .staged: return ([:], index(of: copy, in: state))
        case .turn(let turn):
            let history = state.histories[session] ?? []
            let turns = history.reduce(into: (seen: 0, of: [String: Int]())) { counted, event in
                if case .turnStarted = event.kind { counted.seen += 1 }
                counted.of[event.id] = counted.seen
            }
            guard turn >= 1, turn <= turns.seen else {
                throw DaemonError.refused(
                    .invalidRequest,
                    message: "Session \(session) has had \(turns.seen) turns, so it has no turn \(turn).")
            }
            let upTo = { (last: Int) in
                contents(after: writes.filter { (turns.of[$0.event] ?? turns.seen) <= last })
            }
            return (upTo(turn - 1), upTo(turn))
        }
    }

    fileprivate static func index(of copy: String, in state: State) -> [String: String] {
        var files: [String: String] = [:]
        for (path, contents) in state.staged[copy] ?? [:] {
            files[path] = contents
        }
        return files
    }

    fileprivate static func lines(_ contents: String?) -> [String] {
        guard let contents, !contents.isEmpty else { return [] }
        var lines = contents.components(separatedBy: "\n")
        if lines.last == "" { lines.removeLast() }
        return lines
    }

    /// Names the contents; the same contents get the same name every run.
    fileprivate static func blob(_ contents: String) -> String {
        var hash: UInt64 = 0xcbf2_9ce4_8422_2325
        for byte in contents.utf8 {
            hash = (hash ^ UInt64(byte)) &* 0x100_0000_01b3
        }
        return String(hash, radix: 16)
    }

    fileprivate static func patch(_ files: [FileDiff]) -> String {
        files.map { file in
            let from = file.status == .added ? "/dev/null" : "a/\(file.path)"
            let to = file.status == .deleted ? "/dev/null" : "b/\(file.path)"
            let hunks = file.hunks.map { hunk in
                let lines = hunk.lines.map { line in
                    let mark =
                        switch line.kind {
                        case .context: " "
                        case .added: "+"
                        case .removed: "-"
                        }
                    return mark + line.text
                }
                return (["@@ -\(hunk.oldStart),\(hunk.oldCount) +\(hunk.newStart),\(hunk.newCount) @@"] + lines)
                    .joined(separator: "\n")
            }
            return (["--- \(from)", "+++ \(to)"] + hunks).joined(separator: "\n")
        }.joined(separator: "\n")
    }

    /// Moves the thread to where its lines are now, or marks it outdated.
    fileprivate static func anchor(_ thread: inout ReviewThread, in lines: [String]) {
        let code = thread.code
        let count = code.count
        guard count > 0 else { return }
        let at = { (start: Int) in
            start >= 1 && start - 1 + count <= lines.count
                && Array(lines[(start - 1)..<(start - 1 + count)]) == code
        }
        if at(thread.startLine) {
            thread.outdated = false
        } else if let start = (1...max(lines.count, 1)).first(where: at) {
            thread.startLine = start
            thread.endLine = start + count - 1
            thread.outdated = false
        } else {
            thread.outdated = true
        }
    }

    /// The agent's answers, by thread: in the turn after each review, the
    /// text under a `[thread N]` tag.
    fileprivate static func replies(in history: [SessionEvent], to reviews: [Review]) -> [String: [ReviewComment]] {
        var replies: [String: [ReviewComment]] = [:]
        var pending = reviews[...]
        var answering: (review: Review, turn: Int?)?
        var turns = 0
        for event in history {
            switch event.kind {
            case .userMessage(let text, _):
                if let review = pending.first, review.message == text {
                    pending.removeFirst()
                    answering = (review, nil)
                }
            case .turnStarted:
                turns += 1
                if answering?.turn == nil { answering?.turn = turns }
            case .assistantMessage(let text):
                guard let current = answering, let turn = current.turn else { continue }
                for (thread, answer) in sections(text) where current.review.threads.contains(thread) {
                    replies[thread, default: []].append(
                        ReviewComment(
                            id: "agent-\(current.review.id)-\(thread)", author: .agent, text: answer, sent: true,
                            time: event.time, turn: turn))
                }
            case .turnEnded:
                if answering?.turn != nil { answering = nil }
            default: break
            }
        }
        return replies
    }

    /// The text under each `[thread N]` line.
    fileprivate static func sections(_ text: String) -> [(String, String)] {
        var sections: [(String, [String])] = []
        for line in text.components(separatedBy: "\n") {
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            if trimmed.hasPrefix("[thread "), let close = trimmed.firstIndex(of: "]") {
                let id = trimmed[trimmed.index(trimmed.startIndex, offsetBy: 8)..<close]
                let rest = trimmed[trimmed.index(after: close)...].trimmingCharacters(in: .whitespaces)
                sections.append((String(id), rest.isEmpty ? [] : [rest]))
            } else if !sections.isEmpty {
                sections[sections.count - 1].1.append(line)
            }
        }
        return sections.map { id, lines in
            (id, lines.joined(separator: "\n").trimmingCharacters(in: .whitespacesAndNewlines))
        }.filter { !$0.1.isEmpty }
    }
}

/// A line diff by longest common subsequence, in hunks with `context`
/// unchanged lines around each change.
enum LineDiff {
    static func hunks(_ old: [String], _ new: [String], context: Int) -> [Hunk] {
        let lines = diff(old, new)
        let changed = lines.indices.filter { lines[$0].kind != .context }
        guard !changed.isEmpty || (context == Int.max && !lines.isEmpty) else { return [] }
        var ranges: [ClosedRange<Int>] = []
        if context == Int.max {
            ranges = [0...(lines.count - 1)]
        } else {
            for index in changed {
                let range = max(0, index - context)...min(lines.count - 1, index + context)
                if let last = ranges.last, range.lowerBound <= last.upperBound + 1 {
                    ranges[ranges.count - 1] = last.lowerBound...max(last.upperBound, range.upperBound)
                } else {
                    ranges.append(range)
                }
            }
        }
        return ranges.map { range in
            let slice = Array(lines[range])
            let oldLines = slice.filter { $0.kind != .added }
            let newLines = slice.filter { $0.kind != .removed }
            let oldBefore = lines[..<range.lowerBound].filter { $0.kind != .added }.count
            let newBefore = lines[..<range.lowerBound].filter { $0.kind != .removed }.count
            return Hunk(
                oldStart: oldLines.isEmpty ? oldBefore : oldBefore + 1, oldCount: oldLines.count,
                newStart: newLines.isEmpty ? newBefore : newBefore + 1, newCount: newLines.count,
                lines: slice)
        }
    }

    private static func diff(_ old: [String], _ new: [String]) -> [DiffLine] {
        // lengths[o][n]: the longest common subsequence of old[o...] and new[n...].
        var lengths = Array(repeating: Array(repeating: 0, count: new.count + 1), count: old.count + 1)
        for oldAt in stride(from: old.count - 1, through: 0, by: -1) {
            for newAt in stride(from: new.count - 1, through: 0, by: -1) {
                lengths[oldAt][newAt] =
                    old[oldAt] == new[newAt]
                    ? lengths[oldAt + 1][newAt + 1] + 1
                    : max(lengths[oldAt + 1][newAt], lengths[oldAt][newAt + 1])
            }
        }
        var lines: [DiffLine] = []
        var (oldAt, newAt) = (0, 0)
        while oldAt < old.count || newAt < new.count {
            if oldAt < old.count, newAt < new.count, old[oldAt] == new[newAt] {
                lines.append(DiffLine(kind: .context, old: oldAt + 1, new: newAt + 1, text: old[oldAt]))
                oldAt += 1
                newAt += 1
            } else if newAt < new.count, oldAt == old.count || lengths[oldAt][newAt + 1] >= lengths[oldAt + 1][newAt] {
                lines.append(DiffLine(kind: .added, old: nil, new: newAt + 1, text: new[newAt]))
                newAt += 1
            } else {
                lines.append(DiffLine(kind: .removed, old: oldAt + 1, new: nil, text: old[oldAt]))
                oldAt += 1
            }
        }
        // Removed lines read better before the lines that replace them.
        var ordered: [DiffLine] = []
        var added: [DiffLine] = []
        for line in lines {
            switch line.kind {
            case .added: added.append(line)
            case .removed: ordered.append(line)
            case .context:
                ordered += added
                added = []
                ordered.append(line)
            }
        }
        return ordered + added
    }
}
