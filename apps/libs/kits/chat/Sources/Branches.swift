import Foundation

/// A conversation's messages make a tree: each follows one message, and
/// writing after going back starts a branch beside the one there. What's
/// shown is the path from the first message to where the conversation is
/// now; the rest is kept, to go forward to again.
extension Conversation {
    /// One of the person's messages, with its replies: a point the
    /// conversation can go back or forward to.
    public struct Checkpoint: Identifiable, Hashable, Sendable {
        public var message: Message
        /// The last reply to it, or the message itself before any: where
        /// the conversation is once it goes here.
        public var end: UUID
        /// The changes its replies saved that can still be undone, as the
        /// app counts them.
        public var changes: Int
        /// Whether it's shown, rather than after where the conversation is
        /// or on another branch.
        public var isShown: Bool
        /// Whether it's the last one shown.
        public var isCurrent: Bool

        public var id: UUID { message.id }
    }

    /// Adds `message` after the last one shown, starting a branch when
    /// others came after that one before.
    public mutating func append(_ message: Message) {
        keepParents()
        if let last = messages.last {
            parents[message.id] = last.id
            latest[last.id] = message.id
        }
        messages.append(message)
    }

    /// Shows the path to the message `id`, keeping what came after it.
    public mutating func go(to id: UUID) {
        keepParents()
        let every = messages + hidden
        let byID = Dictionary(every.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
        var path: [Message] = []
        var next: UUID? = id
        while let current = next, let message = byID[current] {
            path.insert(message, at: 0)
            next = parents[current]
        }
        guard !path.isEmpty else { return }
        for (before, after) in zip(path, path.dropFirst()) { latest[before.id] = after.id }
        let shown = Set(path.map(\.id))
        hidden = every.filter { !shown.contains($0.id) }
        messages = path
    }

    /// Shows the path to the message `id` follows, to write another in
    /// its place: `id` and what came after it are kept, to go to again.
    public mutating func go(before id: UUID) {
        if let parent = parent(of: id) {
            go(to: parent)
            return
        }
        guard (messages + hidden).contains(where: { $0.id == id }) else { return }
        // The first message: nothing is shown before it.
        keepParents()
        hidden = messages + hidden
        messages = []
    }

    /// Every point to go to, on every branch, oldest first, with the
    /// changes each reply saved that can still be undone.
    public func checkpoints(changes: (UUID) -> Int) -> [Checkpoint] {
        let shown = Set(messages.map(\.id))
        let current = messages.last { $0.role == .person }?.id
        return oldestFirst(messages + hidden).filter { $0.role == .person }.map { message in
            let replies = self.replies(to: message.id)
            return Checkpoint(
                message: message, end: replies.last ?? message.id,
                changes: replies.reduce(0) { $0 + changes($1) },
                isShown: shown.contains(message.id), isCurrent: message.id == current)
        }
    }

    /// The end of the point before the last one shown, to go back to.
    public var back: UUID? {
        let asked = messages.filter { $0.role == .person }
        guard asked.count > 1 else { return nil }
        let before = asked[asked.count - 2].id
        return replies(to: before).last ?? before
    }

    /// The end of the point after the last message shown, the way it last
    /// went, to go forward to.
    public var forward: UUID? {
        guard let last = messages.last, let next = following(last.id) else { return nil }
        return replies(to: next).last ?? next
    }

    /// The end of the latest branch from where the conversation is now.
    public var newest: UUID? {
        guard var end = messages.last?.id, forward != nil else { return nil }
        while let next = following(end) { end = next }
        return end
    }

    /// The messages shown now that going to `id` would hide, and that
    /// `changed` says saved changes still to undo.
    public func changedReplies(hiddenGoingTo id: UUID, changed: (UUID) -> Bool) -> [UUID] {
        hiddenGoing(changed: changed) { $0.go(to: id) }
    }

    /// The messages shown now that writing the message `id` again would
    /// hide, and that `changed` says saved changes still to undo.
    public func changedReplies(hiddenGoingBefore id: UUID, changed: (UUID) -> Bool) -> [UUID] {
        hiddenGoing(changed: changed) { $0.go(before: id) }
    }

    private func hiddenGoing(changed: (UUID) -> Bool, _ going: (inout Self) -> Void) -> [UUID] {
        var there = self
        going(&there)
        let shown = Set(there.messages.map(\.id))
        return messages.map(\.id).filter { !shown.contains($0) && changed($0) }
    }

    /// The messages written after the same one as `id`, on each branch,
    /// oldest first: `id` and its alternatives.
    public func branches(at id: UUID) -> [Message] {
        let parent = parent(of: id)
        return oldestFirst((messages + hidden).filter { self.parent(of: $0.id) == parent })
    }

    /// Where the conversation is once it goes back to the message `id`:
    /// after the replies to it, for one of the person's.
    public func end(of id: UUID) -> UUID {
        messages.first { $0.id == id }?.role == .person ? replies(to: id).last ?? id : id
    }

    /// The end of the latest path through `id`, to show that branch.
    public func newest(through id: UUID) -> UUID {
        var end = id
        while let next = following(end) { end = next }
        return end
    }

    /// A new conversation called `name` of the messages up to `id`, on this
    /// one's branch to it, with an empty context: what this one's replies
    /// changed stays with it.
    public func fork(at id: UUID, as forkID: UUID, now: Date, name: String) -> Self? {
        var copy = self
        copy.go(to: id)
        guard copy.messages.last?.id == id else { return nil }
        var fork = Self(id: forkID, createdAt: now)
        fork.messages = copy.messages
        let kept = Set(copy.messages.map(\.id))
        fork.summaries = summaries.filter { kept.contains($0.key) }
        fork.isSynced = isSynced
        fork.name = name
        return fork
    }

    /// The message `id` follows, or nil for the first.
    private func parent(of id: UUID) -> UUID? {
        if let parent = parents[id] { return parent }
        guard let index = messages.firstIndex(where: { $0.id == id }), index > 0 else { return nil }
        return messages[index - 1].id
    }

    /// The message last shown after `id`, or the newest after it.
    private func following(_ id: UUID) -> UUID? {
        if let next = latest[id] { return next }
        return oldestFirst((messages + hidden).filter { parent(of: $0.id) == id }).last?.id
    }

    /// `messages` by when they were written, keeping their order when
    /// that's the same.
    private func oldestFirst(_ messages: [Message]) -> [Message] {
        messages.enumerated().sorted { ($0.element.at, $0.offset) < ($1.element.at, $1.offset) }.map(\.element)
    }

    /// The assistant's replies after the person's message `id`, the way
    /// the conversation last went.
    private func replies(to id: UUID) -> [UUID] {
        var replies: [UUID] = []
        var end = id
        let roles = Dictionary((messages + hidden).map { ($0.id, $0.role) }, uniquingKeysWith: { first, _ in first })
        while let next = following(end), roles[next] == .assistant {
            replies.append(next)
            end = next
        }
        return replies
    }

    /// Writes down what each shown message follows, so it's kept once some
    /// are hidden.
    private mutating func keepParents() {
        for (before, after) in zip(messages, messages.dropFirst()) where parents[after.id] == nil {
            parents[after.id] = before.id
        }
    }
}
