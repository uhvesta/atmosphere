import ComposableArchitecture
import Foundation
import HumataDaemonClient

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
        package var picked: Picked?
        package var comment = ""
        package var drafts: IdentifiedArrayOf<Draft> = []
        package var loading = false
        package var sending = false
        /// Why the selected file couldn't be read, until it is.
        package var readFailure: String?
        @Presents package var alert: AlertState<Never>?

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
            var lines = text.components(separatedBy: "\n")
            if lines.last == "" { lines.removeLast() }
            self.lines = lines
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
        case read(FileContents)
        case failed(Doing, String)
        case fileSelected(String)
        case renderedToggled
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
        case alert(PresentationAction<Never>)
    }

    /// What was being done when something failed.
    package enum Doing: Equatable, Sendable {
        case listing
        case reading(path: String)
        case sending
    }

    @Dependency(\.daemon) var daemon
    @Dependency(\.uuid) var uuid

    private enum CancelID { case list }

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .binding, .alert:
                return .none

            case .refresh:
                state.loading = true
                state.readFailure = nil
                let session = state.sessionID
                let list: Effect<Action> = .run { send in
                    await send(.listed(try await daemon.files(session)))
                } catch: { error, send in
                    await send(.failed(.listing, error.localizedDescription))
                }
                return .merge(list.cancellable(id: CancelID.list, cancelInFlight: true), read(state))

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
                return read(state)

            case .read(let contents):
                guard contents.path == state.selected else { return .none }
                state.readFailure = nil
                state.show(contents)
                return .none

            case .failed(.reading(let path), let message):
                // Said where the file would show, with Try Again.
                if path == state.selected { state.readFailure = message }
                return .none

            case .failed(let doing, let message):
                state.loading = false
                state.sending = false
                state.alert = AlertState {
                    TextState(
                        doing == .sending
                            ? String(localized: "Couldn’t send the comments")
                            : String(localized: "Couldn’t list the chat’s files"))
                } message: {
                    TextState(message)
                }
                return .none

            case .fileSelected(let path):
                guard state.selected != path else { return .none }
                state.selected = path
                state.show(nil)
                state.readFailure = nil
                state.picked = nil
                return read(state)

            case .renderedToggled:
                state.rendered.toggle()
                state.picked = nil
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
                    await send(.failed(.sending, error.localizedDescription))
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
        return .run { send in
            await send(.read(try await daemon.readFile(session, path)))
        } catch: { error, send in
            await send(.failed(.reading(path: path), error.localizedDescription))
        }
        // Not cancelled by the next read: effects start out of the order
        // they're asked in, so an older read could cancel a newer one.
        // `.read` drops what's for a file no longer picked.
    }

    /// The lines picked, as the quote.
    private func quote(_ state: State) -> String {
        guard let picked = state.picked, let lines = state.lines,
            picked.startLine >= 1, picked.endLine <= lines.count
        else { return "" }
        return lines[(picked.startLine - 1)...(picked.endLine - 1)].joined(separator: "\n")
    }
}
