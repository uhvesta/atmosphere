import ComposableArchitecture
import Foundation
import HumataDaemonClient
import HumataReview

/// A chat's folder: its files, the selected one read, Markdown rendered,
/// and comments on what was picked in them, kept as drafts until they're
/// sent to the chat as one message.
@Reducer
package struct DocsFeature {
    /// What was picked to comment on: lines of a file, and in rendered
    /// Markdown or a selection, the text itself.
    package struct Picked: Hashable, Sendable {
        package var path: String
        package var startLine: Int
        package var endLine: Int
        /// The text commented on, which can be cut down to the words meant.
        package var quote: String

        package init(path: String, startLine: Int, endLine: Int, quote: String = "") {
            self.path = path
            self.startLine = startLine
            self.endLine = endLine
            self.quote = quote
        }

        package func covers(_ line: Int, in path: String) -> Bool {
            self.path == path && (startLine...endLine).contains(line)
        }
    }

    /// A comment not sent yet.
    package struct Draft: Hashable, Sendable, Identifiable {
        package var id: UUID
        package var picked: Picked
        package var text: String

        package init(id: UUID, picked: Picked, text: String) {
            self.id = id
            self.picked = picked
            self.text = text
        }
    }

    /// The versions of the selected file its turns left, one of them shown,
    /// on its own or compared with the file now.
    package struct Versions: Equatable, Sendable {
        package var path: String
        /// Oldest first; nil until they're listed.
        package var list: [FileVersion]?
        /// The version shown, by turn; nil shows the file now.
        package var selected: Int?
        /// The version shown, once it's read.
        package var lines: [String]?
        /// Shows the version and the file now as one rendered diff.
        package var comparing = false
        package var comparison: [ComparedBlock]?
        package var restoring = false

        package init(path: String) {
            self.path = path
        }
    }

    package enum Alert: Equatable, Sendable {
        case restore(path: String, turn: Int)
    }

    @ObservableState
    package struct State: Equatable {
        package var sessionID: String
        package var files: [ChatFile] = []
        /// The folder has more files than are listed.
        package var truncated = false
        package var selected: String?
        /// The selected file, once it's read.
        /// The selected file as read; set through `show`, which splits it
        /// into lines once.
        package private(set) var contents: FileContents?
        /// The selected file's lines, once it's read and is text.
        package private(set) var lines: [String]?
        /// Shows Markdown as it reads; otherwise its source.
        package var rendered = true
        /// Shows the file wide, beside the chat and its composer.
        package var focused = false
        /// The selected file's lines, from 1, that changed when it last
        /// read again; cleared when another file is picked.
        package var changed: Set<Int> = []
        /// The selected file's versions, while they're shown.
        package var versions: Versions?
        package var picked: Picked?
        package var comment = ""
        package var drafts: IdentifiedArrayOf<Draft> = []
        package var loading = false
        package var sending = false
        @Presents package var alert: AlertState<Alert>?

        package init(sessionID: String) {
            self.sessionID = sessionID
        }

        package var isMarkdown: Bool {
            files.first { $0.path == selected }?.isMarkdown ?? false
        }

        package mutating func show(_ contents: FileContents?) {
            self.contents = contents
            guard let contents, let text = contents.text else {
                lines = nil
                return
            }
            lines = DocsFeature.lines(text)
        }

        /// The drafts as the one message they're sent as: each quote, where
        /// it is, then the comment, by file and line.
        package var message: String {
            let sorted = drafts.sorted { ($0.picked.path, $0.picked.startLine) < ($1.picked.path, $1.picked.startLine) }
            let comments = sorted.map { draft in
                let picked = draft.picked
                let lines =
                    picked.startLine == picked.endLine
                    ? "line \(picked.startLine)" : "lines \(picked.startLine)–\(picked.endLine)"
                var part = "\(picked.path), \(lines):\n"
                if !picked.quote.isEmpty {
                    part +=
                        picked.quote.split(separator: "\n", omittingEmptySubsequences: false)
                        .map { "> \($0)" }.joined(separator: "\n") + "\n"
                }
                return part + draft.text
            }
            let intro =
                drafts.count == 1
                ? "A comment on the files in this chat's folder:"
                : "Comments on the files in this chat's folder; answer each:"
            return ([intro] + comments).joined(separator: "\n\n")
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        /// The view appeared, or the chat's turn ended; lists and reads again.
        case refresh
        case listed(ChatFiles)
        /// A file read, with the lines that changed since it was read last.
        case read(FileContents, changed: Set<Int>)
        case failed(String)
        case fileSelected(String)
        case renderedToggled
        case focusToggled
        /// Shows the selected file's versions, or stops.
        case versionsToggled
        case versionsListed(path: String, [FileVersion])
        case versionsFailed(String)
        /// A version to show, by turn; nil for the file now.
        case versionPicked(Int?)
        case versionRead(path: String, turn: Int, lines: [String])
        case compareToggled
        case compared(path: String, turn: Int, [ComparedBlock])
        /// Asks before the shown version is written over the file.
        case restoreTapped
        case restored(FileContents, changed: Set<Int>)
        /// A line of the source, by number; extending picks the lines from
        /// the one picked before to it.
        case lineTapped(Int, extending: Bool)
        /// A rendered Markdown block, by its lines.
        case blockTapped(startLine: Int, endLine: Int, quote: String)
        /// Text selected in the source, with the lines it's on.
        case textSelected(startLine: Int, endLine: Int, quote: String)
        case commentCancelled
        case commentSaved
        case draftRemoved(Draft.ID)
        case sendTapped
        case sent
        case alert(PresentationAction<Alert>)
    }

    @Dependency(\.daemon) var daemon
    @Dependency(\.uuid) var uuid

    private enum CancelID {
        case list
        case versions
        case version
        case compare
    }

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .binding, .alert(.dismiss):
                return .none

            case .refresh:
                state.loading = true
                let session = state.sessionID
                let list: Effect<Action> = .run { send in
                    await send(.listed(try await daemon.files(session)))
                } catch: { error, send in
                    await send(.failed(error.localizedDescription))
                }
                return .merge(
                    list.cancellable(id: CancelID.list, cancelInFlight: true), read(state), listVersions(state))

            case .listed(let files):
                state.loading = false
                state.files = files.files
                state.truncated = files.truncated
                guard state.selected.map({ selected in !files.files.contains { $0.path == selected } }) ?? true else {
                    return .none
                }
                // The first Markdown file, as the chat's documents are most
                // likely those.
                state.selected = (files.files.first(where: \.isMarkdown) ?? files.files.first)?.path
                state.show(nil)
                state.picked = nil
                state.changed = []
                if state.versions != nil { state.versions = state.selected.map(Versions.init) }
                return .merge(read(state), listVersions(state))

            case .read(let contents, let changed):
                guard contents.path == state.selected, contents != state.contents else { return .none }
                // Read again unchanged, the file keeps its marks.
                state.changed = state.contents == nil ? [] : changed
                state.show(contents)
                return compare(state)

            case .failed(let message):
                state.loading = false
                state.sending = false
                state.alert = AlertState {
                    TextState("Something went wrong")
                } message: {
                    TextState(message)
                }
                return .none

            case .fileSelected(let path):
                guard state.selected != path else { return .none }
                state.selected = path
                state.show(nil)
                state.picked = nil
                state.changed = []
                if state.versions != nil { state.versions = Versions(path: path) }
                return .merge(read(state), listVersions(state))

            case .renderedToggled:
                state.rendered.toggle()
                state.picked = nil
                return .none

            case .focusToggled:
                state.focused.toggle()
                return .none

            case .versionsToggled:
                guard state.versions == nil, let path = state.selected else {
                    state.versions = nil
                    return .merge(.cancel(id: CancelID.versions), .cancel(id: CancelID.version))
                }
                state.versions = Versions(path: path)
                state.picked = nil
                return listVersions(state)

            case .versionsListed(let path, let list):
                guard state.versions?.path == path else { return .none }
                state.versions?.list = list
                return .none

            case .versionsFailed(let message):
                if state.versions?.list == nil { state.versions = nil }
                state.versions?.restoring = false
                state.alert = AlertState {
                    TextState("Couldn’t read or restore the file’s versions")
                } message: {
                    TextState(
                        "\(message)\n\nThe file as it is now is unchanged. Check that Humata’s daemon is running, "
                            + "then try again.")
                }
                return .none

            case .versionPicked(let turn):
                guard let versions = state.versions, versions.selected != turn else { return .none }
                state.versions?.selected = turn
                state.versions?.lines = nil
                state.versions?.comparison = nil
                state.picked = nil
                guard let turn else { return .cancel(id: CancelID.version) }
                let session = state.sessionID
                let path = versions.path
                return .run { send in
                    let contents = try await daemon.readFileVersion(session, path, turn)
                    await send(.versionRead(path: path, turn: turn, lines: Self.lines(contents.text ?? "")))
                } catch: { error, send in
                    await send(.versionsFailed(error.localizedDescription))
                }
                .cancellable(id: CancelID.version, cancelInFlight: true)

            case .versionRead(let path, let turn, let lines):
                guard state.versions?.path == path, state.versions?.selected == turn else { return .none }
                state.versions?.lines = lines
                return compare(state)

            case .compareToggled:
                guard state.versions != nil else { return .none }
                state.versions?.comparing.toggle()
                state.versions?.comparison = nil
                return compare(state)

            case .compared(let path, let turn, let blocks):
                guard state.versions?.path == path, state.versions?.selected == turn,
                    state.versions?.comparing == true
                else { return .none }
                state.versions?.comparison = blocks
                return .none

            case .restoreTapped:
                guard let versions = state.versions, let turn = versions.selected, !versions.restoring else {
                    return .none
                }
                state.alert = AlertState {
                    TextState("Restore this version?")
                } actions: {
                    ButtonState(role: .destructive, action: .restore(path: versions.path, turn: turn)) {
                        TextState("Restore")
                    }
                    ButtonState(role: .cancel) {
                        TextState("Cancel")
                    }
                } message: {
                    TextState(
                        "\(versions.path) is written as \(Self.name(turn)) left it, over the file as it is now. "
                            + "It’s a change of its own, which the next turn’s snapshot keeps; the file as it is "
                            + "now stays in the list only if a turn snapshotted it.")
                }
                return .none

            case .alert(.presented(.restore(let path, let turn))):
                guard state.versions?.path == path else { return .none }
                state.versions?.restoring = true
                let session = state.sessionID
                let old = state.contents?.path == path ? state.lines : nil
                return .run { send in
                    let contents = try await daemon.restoreFileVersion(session, path, turn)
                    let changed = old.map {
                        MarkdownComparison.changedLines(from: $0, to: Self.lines(contents.text ?? ""))
                    }
                    await send(.restored(contents, changed: changed ?? []))
                } catch: { error, send in
                    await send(.versionsFailed(error.localizedDescription))
                }

            case .restored(let contents, let changed):
                state.versions?.restoring = false
                state.versions?.selected = nil
                state.versions?.lines = nil
                state.versions?.comparing = false
                state.versions?.comparison = nil
                guard contents.path == state.selected else { return .none }
                state.changed = changed
                state.show(contents)
                return .none

            case .lineTapped(let line, let extending):
                guard let path = state.selected else { return .none }
                if extending, let picked = state.picked, picked.path == path {
                    state.picked?.startLine = min(picked.startLine, line)
                    state.picked?.endLine = max(picked.endLine, line)
                } else {
                    state.picked = Picked(path: path, startLine: line, endLine: line)
                }
                let quote = quote(state)
                state.picked?.quote = quote
                return .none

            case .blockTapped(let start, let end, let quote), .textSelected(let start, let end, let quote):
                guard let path = state.selected else { return .none }
                state.picked = Picked(path: path, startLine: start, endLine: end, quote: quote)
                return .none

            case .commentCancelled:
                state.picked = nil
                state.comment = ""
                return .none

            case .commentSaved:
                let text = state.comment.trimmingCharacters(in: .whitespacesAndNewlines)
                guard var picked = state.picked, !text.isEmpty else { return .none }
                picked.quote = picked.quote.trimmingCharacters(in: .whitespacesAndNewlines)
                let draft = Draft(id: uuid(), picked: picked, text: text)
                state.drafts.append(draft)
                state.picked = nil
                state.comment = ""
                return .none

            case .draftRemoved(let id):
                state.drafts.remove(id: id)
                return .none

            case .sendTapped:
                guard !state.drafts.isEmpty, !state.sending else { return .none }
                state.sending = true
                let session = state.sessionID
                let message = state.message
                return .run { send in
                    try await daemon.send(session, message, .enqueue)
                    await send(.sent)
                } catch: { error, send in
                    await send(.failed(error.localizedDescription))
                }

            case .sent:
                // Kept until now, so a failed send loses nothing.
                state.sending = false
                state.drafts = []
                return .none
            }
        }
        .ifLet(\.$alert, action: \.alert)
    }

    private func read(_ state: State) -> Effect<Action> {
        guard let path = state.selected else { return .none }
        let session = state.sessionID
        let old = state.contents?.path == path ? state.lines : nil
        return .run { send in
            let contents = try await daemon.readFile(session, path)
            // Off the main thread: a long file's diff takes a moment.
            let changed = old.map { MarkdownComparison.changedLines(from: $0, to: Self.lines(contents.text ?? "")) }
            await send(.read(contents, changed: changed ?? []))
        } catch: { error, send in
            await send(.failed(error.localizedDescription))
        }
        // Not cancelled by the next read: effects start out of the order
        // they're asked in, so an older read could cancel a newer one.
        // `.read` drops what's for a file no longer picked.
    }

    /// The selected file's versions listed, while they're shown.
    private func listVersions(_ state: State) -> Effect<Action> {
        guard let path = state.versions?.path else { return .none }
        let session = state.sessionID
        return .run { send in
            await send(.versionsListed(path: path, try await daemon.fileVersions(session, path)))
        } catch: { error, send in
            await send(.versionsFailed(error.localizedDescription))
        }
        .cancellable(id: CancelID.versions, cancelInFlight: true)
    }

    /// The version shown and the file now compared, when that's asked for
    /// and both are read.
    private func compare(_ state: State) -> Effect<Action> {
        guard let versions = state.versions, versions.comparing, let turn = versions.selected,
            let old = versions.lines, let new = state.lines, state.contents?.path == versions.path
        else { return .none }
        let path = versions.path
        return .run { send in
            await send(.compared(path: path, turn: turn, MarkdownComparison.compare(old, with: new)))
        }
        .cancellable(id: CancelID.compare, cancelInFlight: true)
    }

    /// `text` split into lines, without the empty one after a last newline.
    static func lines(_ text: String) -> [String] {
        var lines = text.components(separatedBy: "\n")
        if lines.last == "" { lines.removeLast() }
        return lines
    }

    /// A version, by its turn, as the person reads it.
    static func name(_ turn: Int) -> String {
        turn == 0 ? "the start of the chat" : "turn \(turn)"
    }

    /// The lines picked, as the quote.
    private func quote(_ state: State) -> String {
        guard let picked = state.picked, let lines = state.lines,
            picked.startLine >= 1, picked.endLine <= lines.count
        else { return "" }
        return lines[(picked.startLine - 1)...(picked.endLine - 1)].joined(separator: "\n")
    }
}
