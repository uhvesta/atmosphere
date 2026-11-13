import ComposableArchitecture
import Foundation
import HumataDaemonClient

/// A session's changes as a code review: a range of its history, by
/// repository and file, with comments on lines in threads, sent to the agent
/// as one review, and its answers and fixes shown in them. Files can be
/// staged, unstaged and reverted whole or by hunk, and marked viewed.
@Reducer
package struct ReviewFeature {
    /// Which changes to read, as picked.
    package enum RangeKind: Hashable, CaseIterable, Sendable {
        case session
        case unstaged
        case staged
        case branch
        case turn
    }

    /// Lines picked to comment on, in the file as it is in the copy.
    package struct Commenting: Hashable, Sendable {
        package var file: FileRef
        package var startLine: Int
        package var endLine: Int
        /// In rendered Markdown, the text picked.
        package var quote: String

        package init(file: FileRef, startLine: Int, endLine: Int, quote: String = "") {
            self.file = file
            self.startLine = startLine
            self.endLine = endLine
            self.quote = quote
        }
    }

    @ObservableState
    package struct State: Equatable {
        package var sessionID: String
        /// How many turns the session has had, to pick one by.
        package var turns: Int
        package var range: ChangeRange = .session
        /// The branch to compare with when the range is a branch.
        package var branch = "main"
        /// Unchanged lines shown around each change.
        package var context = 3
        package var repositories: [RepositoryChanges] = []
        package var selected: FileRef?
        /// Shows every line of the selected file, changed or not.
        package var wholeFile = false
        /// The selected file read whole, once it is.
        package var whole: FileDiff?
        /// Shows a Markdown file rendered, its blocks commented on.
        package var rendered = false
        package var threads: IdentifiedArrayOf<ReviewThread> = []
        package var showsResolved = false
        package var commenting: Commenting?
        package var comment = ""
        /// Replies being written, by thread.
        package var replies: [ReviewThread.ID: String] = [:]
        package var loading = false
        /// The changes may have moved since they were last read, or never
        /// were: they're read again when the review next appears.
        package var stale = true
        /// Numbers each load, so only the latest one's changes show: effects
        /// can start out of the order they were asked in.
        var request = 0
        package var sending = false
        /// Why the changes couldn't be read the last time, until they are.
        package var loadFailure: String?
        /// A stage, unstage or revert waiting on the daemon; another waits.
        package var editing = false
        @Presents package var alert: AlertState<Action.Alert>?

        package init(sessionID: String, turns: Int = 0) {
            self.sessionID = sessionID
            self.turns = turns
        }

        /// What's kept of a review looked away from while Humata runs: the
        /// file and range it showed, and the comment and replies not sent yet.
        /// The changes themselves are read again when it next appears.
        package struct Kept: Equatable, Sendable {
            var range: ChangeRange
            var selected: FileRef?
            var wholeFile: Bool
            var rendered: Bool
            var commenting: Commenting?
            var comment: String
            var replies: [ReviewThread.ID: String]
        }

        package var kept: Kept {
            get {
                Kept(
                    range: range, selected: selected, wholeFile: wholeFile, rendered: rendered,
                    commenting: commenting, comment: comment, replies: replies)
            }
            set {
                range = newValue.range
                // The branch's field shows the one compared with.
                if case .branch(let name) = newValue.range { branch = name }
                selected = newValue.selected
                wholeFile = newValue.wholeFile
                rendered = newValue.rendered
                commenting = newValue.commenting
                comment = newValue.comment
                replies = newValue.replies
            }
        }

        package var rangeKind: RangeKind {
            switch range {
            case .session: .session
            case .unstaged: .unstaged
            case .staged: .staged
            case .branch: .branch
            case .turn: .turn
            }
        }

        /// The files in the range, with their repositories.
        package var files: [FileRef] {
            repositories.flatMap { repository in
                repository.files.map { FileRef(repository: repository.name, path: $0.path) }
            }
        }

        /// The selected file as shown: whole, or its changes.
        package var file: FileDiff? {
            guard let selected else { return nil }
            if wholeFile || rendered, let whole, whole.path == selected.path { return whole }
            return diff(selected)
        }

        /// The picked lines as a quote for the message box, with the comment
        /// written so far. A diff's lines keep their marks: what was taken
        /// out shows too.
        func quote(_ picked: Commenting) -> String? {
            let path = picked.file.path
            if !picked.quote.isEmpty {
                return CodeQuote.text(
                    path: path, start: picked.startLine, end: picked.endLine, language: "", code: picked.quote,
                    comment: comment)
            }
            guard let file else { return nil }
            let lines = file.hunks.flatMap(\.lines)
            let range = picked.startLine...picked.endLine
            guard let first = lines.firstIndex(where: { $0.new.map(range.contains) ?? false }),
                let last = lines.lastIndex(where: { $0.new.map(range.contains) ?? false })
            else { return nil }
            let picks = lines[first...last]
            let marked = picks.contains { $0.kind != .context }
            let code = picks.map { line in
                guard marked else { return line.text }
                return (line.kind == .added ? "+" : line.kind == .removed ? "-" : " ") + line.text
            }
            return CodeQuote.text(
                path: path, start: picked.startLine, end: picked.endLine,
                language: marked ? "diff" : CodeQuote.language(of: path), code: code.joined(separator: "\n"),
                comment: comment)
        }

        package func diff(_ file: FileRef) -> FileDiff? {
            repositories.first { $0.name == file.repository }?.files.first { $0.path == file.path }
        }

        /// Lines can be commented on where the range ends at the files in the
        /// copy, which is where threads are kept.
        package var canComment: Bool {
            switch range {
            case .session, .unstaged, .branch: true
            case .staged, .turn: false
            }
        }

        /// The threads shown, on the selected file or every file.
        package func shownThreads(on file: FileRef?) -> [ReviewThread] {
            threads.filter { (file == nil || $0.file == file) && (showsResolved || !$0.resolved) }
        }

        /// Comments not sent yet, on threads not resolved.
        package var pending: Int {
            threads.filter { !$0.resolved }.map(\.pending).reduce(0, +)
        }

        package var isMarkdown: Bool {
            guard let path = selected?.path.lowercased() else { return false }
            return path.hasSuffix(".md") || path.hasSuffix(".markdown")
        }
    }

    package enum Action: BindableAction {
        package enum Alert: Equatable {
            case revert(ChangeEdit)
        }

        case binding(BindingAction<State>)
        /// The view appeared, or the session's turn ended; reads it all again.
        case refresh
        /// The review came on screen: read again only after a turn since.
        case appeared
        case loaded(request: Int, [RepositoryChanges], [ReviewThread])
        case loadFailed(request: Int, String)
        case wholeLoaded(FileDiff?)
        case failed(Doing, String)
        case rangePicked(RangeKind)
        case turnPicked(Int)
        case branchSubmitted
        /// Shows more unchanged lines around changes, or fewer.
        case contextChanged(by: Int)
        case fileSelected(FileRef)
        case wholeFileToggled
        case renderedToggled
        /// A line of the selected file, by its number in the copy; extending
        /// picks the lines from the one picked before to it.
        case lineTapped(Int, extending: Bool)
        /// A rendered Markdown block, by its lines.
        case blockTapped(startLine: Int, endLine: Int, quote: String)
        case commentCancelled
        case commentSaved
        /// The comment wasn't saved; it goes back on its lines unless
        /// another was started since.
        case commentFailed(Commenting, text: String, message: String)
        case threadSaved(ReviewThread)
        /// The reply wasn't sent; it goes back in its box unless another
        /// was started since.
        case replyFailed(ReviewThread.ID, text: String, message: String)
        case replyChanged(ReviewThread.ID, String)
        case replySent(ReviewThread.ID)
        case resolveToggled(ReviewThread.ID)
        case deleteTapped(ReviewComment.ID)
        case sendTapped
        case sent(Int)
        /// Shows what the agent changed in the turn it answered the thread.
        case showFixTapped(ReviewThread.ID)
        case viewedToggled(FileRef)
        case editTapped(ChangeEdit)
        case edited
        /// Puts the picked lines, with the comment written so far, in the
        /// message box as a quote, instead of keeping them for the review.
        case quoteTapped
        case alert(PresentationAction<Alert>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate: Equatable {
            /// Text for the message box.
            case quote(String)
        }
    }

    /// What was being done when something failed, for the alert's title.
    package enum Doing: Equatable, Sendable {
        case reading
        case commenting
        case replying
        case resolving
        case deleting
        case sending
        case markingViewed
        case editing(ChangeAction)

        var title: String {
            switch self {
            case .reading: String(localized: "Couldn’t read the changes")
            case .commenting: String(localized: "Couldn’t save the comment")
            case .replying: String(localized: "Couldn’t send the reply")
            case .resolving: String(localized: "Couldn’t change the thread")
            case .deleting: String(localized: "Couldn’t delete the comment")
            case .sending: String(localized: "Couldn’t send the review")
            case .markingViewed: String(localized: "Couldn’t mark the file")
            case .editing(.stage): String(localized: "Couldn’t stage the change")
            case .editing(.unstage): String(localized: "Couldn’t unstage the change")
            case .editing(.revert): String(localized: "Couldn’t revert the change")
            }
        }
    }

    @Dependency(\.daemon) var daemon

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .binding, .alert(.dismiss), .delegate:
                return .none

            case .quoteTapped:
                guard let picked = state.commenting, let quote = state.quote(picked) else { return .none }
                state.commenting = nil
                state.comment = ""
                return .send(.delegate(.quote(quote)))

            case .refresh:
                return load(&state)

            case .appeared:
                guard state.stale else { return .none }
                return load(&state)

            // A where clause holds for only the pattern before it, so each has one.
            case .loaded(let request, _, _) where request != state.request,
                .loadFailed(let request, _) where request != state.request:
                // For another range or context than the one picked now.
                return .none

            case .loadFailed(_, let message):
                state.stale = true
                state.loading = false
                state.loadFailure = message
                // With nothing listed the empty view says it; with a list, the
                // list is as it was last read.
                return state.files.isEmpty ? .none : .send(.failed(.reading, message))

            case .loaded(_, let repositories, let threads):
                state.loading = false
                state.loadFailure = nil
                state.repositories = repositories
                state.threads = IdentifiedArray(uniqueElements: threads)
                if state.selected.map({ selected in !state.files.contains(selected) }) ?? true {
                    state.selected = state.files.first
                    state.whole = nil
                }
                return loadWhole(state)

            case .wholeLoaded(let file):
                state.whole = file
                return .none

            case .failed(let doing, let message):
                state.loading = false
                state.sending = false
                if case .editing = doing { state.editing = false }
                state.alert = AlertState {
                    TextState(doing.title)
                } message: {
                    TextState(message)
                }
                return .none

            case .rangePicked(let kind):
                switch kind {
                case .session: state.range = .session
                case .unstaged: state.range = .unstaged
                case .staged: state.range = .staged
                case .branch: state.range = .branch(state.branch)
                case .turn: state.range = .turn(max(state.turns, 1))
                }
                state.commenting = nil
                return load(&state)

            case .turnPicked(let turn):
                state.range = .turn(turn)
                return load(&state)

            case .branchSubmitted:
                let branch = state.branch.trimmingCharacters(in: .whitespaces)
                guard !branch.isEmpty else { return .none }
                state.range = .branch(branch)
                return load(&state)

            case .contextChanged(let delta):
                let context = min(max(state.context + delta, 0), 100)
                guard context != state.context else { return .none }
                state.context = context
                return load(&state)

            case .fileSelected(let file):
                guard state.selected != file else { return .none }
                state.selected = file
                state.whole = nil
                state.commenting = nil
                if !state.isMarkdown { state.rendered = false }
                return loadWhole(state)

            case .wholeFileToggled:
                state.wholeFile.toggle()
                return loadWhole(state)

            case .renderedToggled:
                state.rendered.toggle()
                state.commenting = nil
                return loadWhole(state)

            case .lineTapped(let line, let extending):
                guard state.canComment, let file = state.selected else { return .none }
                if extending, let picked = state.commenting, picked.file == file, picked.quote.isEmpty {
                    state.commenting?.startLine = min(picked.startLine, line)
                    state.commenting?.endLine = max(picked.endLine, line)
                } else {
                    state.commenting = Commenting(file: file, startLine: line, endLine: line)
                }
                return .none

            case .blockTapped(let start, let end, let quote):
                guard state.canComment, let file = state.selected else { return .none }
                state.commenting = Commenting(file: file, startLine: start, endLine: end, quote: quote)
                return .none

            case .commentCancelled:
                state.commenting = nil
                state.comment = ""
                return .none

            case .commentSaved:
                let text = state.comment.trimmingCharacters(in: .whitespacesAndNewlines)
                guard let picked = state.commenting, !text.isEmpty else { return .none }
                state.commenting = nil
                state.comment = ""
                let session = state.sessionID
                let thread = NewThread(
                    file: picked.file, startLine: picked.startLine, endLine: picked.endLine, quote: picked.quote,
                    text: text)
                return .run { send in
                    await send(.threadSaved(try await daemon.createThread(session, thread)))
                } catch: { error, send in
                    await send(.commentFailed(picked, text: text, message: error.localizedDescription))
                }

            case .commentFailed(let picked, let text, let message):
                if state.commenting == nil, state.comment.isEmpty {
                    state.commenting = picked
                    state.comment = text
                }
                return .send(.failed(.commenting, message))

            case .threadSaved(let thread):
                state.threads[id: thread.id] = thread
                return .none

            case .replyChanged(let id, let text):
                state.replies[id] = text
                return .none

            case .replySent(let id):
                let text = (state.replies[id] ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
                guard !text.isEmpty else { return .none }
                state.replies[id] = nil
                return .run { send in
                    await send(.threadSaved(try await daemon.replyToThread(id, text)))
                } catch: { error, send in
                    await send(.replyFailed(id, text: text, message: error.localizedDescription))
                }

            case .replyFailed(let id, let text, let message):
                if state.replies[id] == nil { state.replies[id] = text }
                return .send(.failed(.replying, message))

            case .resolveToggled(let id):
                guard let thread = state.threads[id: id] else { return .none }
                let resolved = !thread.resolved
                return .run { send in
                    await send(.threadSaved(try await daemon.resolveThread(id, resolved)))
                } catch: { error, send in
                    await send(.failed(.resolving, error.localizedDescription))
                }

            case .deleteTapped(let comment):
                return .run { send in
                    try await daemon.deleteComment(comment)
                    await send(.refresh)
                } catch: { error, send in
                    await send(.failed(.deleting, error.localizedDescription))
                }

            case .sendTapped:
                guard state.pending > 0, !state.sending else { return .none }
                state.sending = true
                let session = state.sessionID
                return .run { send in
                    await send(.sent(try await daemon.sendReview(session)))
                } catch: { error, send in
                    await send(.failed(.sending, error.localizedDescription))
                }

            case .sent:
                state.sending = false
                return load(&state)

            case .showFixTapped(let id):
                guard let thread = state.threads[id: id],
                    let turn = thread.comments.last(where: { $0.author == .agent })?.turn
                else { return .none }
                state.range = .turn(turn)
                state.selected = thread.file
                state.wholeFile = false
                state.rendered = false
                state.commenting = nil
                return load(&state)

            case .viewedToggled(let file):
                guard let diff = state.diff(file), !diff.blob.isEmpty else { return .none }
                let session = state.sessionID
                let viewed = !diff.viewed
                if let repository = state.repositories.firstIndex(where: { $0.name == file.repository }),
                    let index = state.repositories[repository].files.firstIndex(where: { $0.path == file.path })
                {
                    state.repositories[repository].files[index].viewed = viewed
                }
                return .run { _ in
                    try await daemon.markViewed(session, file, diff.blob, viewed)
                } catch: { error, send in
                    await send(.failed(.markingViewed, error.localizedDescription))
                }

            case .editTapped(let edit):
                guard !state.editing else { return .none }
                if edit.action == .revert {
                    state.alert = Self.confirmRevert(edit)
                    return .none
                }
                return self.edit(edit, in: &state)

            case .alert(.presented(.revert(let edit))):
                guard !state.editing else { return .none }
                return self.edit(edit, in: &state)

            case .edited:
                state.editing = false
                return load(&state)
            }
        }
        .ifLet(\.$alert, action: \.alert)
    }

    private func load(_ state: inout State) -> Effect<Action> {
        state.loading = true
        // A change during this read marks it stale again.
        state.stale = false
        state.request += 1
        let (request, session) = (state.request, state.sessionID)
        let query = ChangesQuery(range: state.range, context: state.context)
        return .run { send in
            async let changes = daemon.changes(session, query)
            async let threads = daemon.threads(session)
            await send(.loaded(request: request, try await changes, try await threads))
        } catch: { error, send in
            await send(.loadFailed(request: request, error.localizedDescription))
        }
    }

    private func loadWhole(_ state: State) -> Effect<Action> {
        guard state.wholeFile || state.rendered, let file = state.selected, state.diff(file)?.status != .deleted
        else { return .none }
        let session = state.sessionID
        let query = ChangesQuery(range: state.range, context: state.context, file: file, wholeFile: true)
        return .run { send in
            let changes = try await daemon.changes(session, query)
            await send(.wholeLoaded(changes.flatMap(\.files).first))
        } catch: { error, send in
            await send(.failed(.reading, error.localizedDescription))
        }
    }

    private func edit(_ edit: ChangeEdit, in state: inout State) -> Effect<Action> {
        state.editing = true
        let session = state.sessionID
        return .run { send in
            try await daemon.editChanges(session, edit)
            await send(.edited)
        } catch: { error, send in
            await send(.failed(.editing(edit.action), error.localizedDescription))
        }
    }

    private static func confirmRevert(_ edit: ChangeEdit) -> AlertState<Action.Alert> {
        let what = edit.hunk == nil ? "every unstaged change to \(edit.file.path)" : "this change to \(edit.file.path)"
        return AlertState {
            TextState("Revert \(edit.file.path)?")
        } actions: {
            ButtonState(role: .destructive, action: .revert(edit)) { TextState("Revert") }
            ButtonState(role: .cancel) { TextState("Cancel") }
        } message: {
            TextState(
                "This throws away \(what) in the session's copy of the code; a file that's new there is removed. "
                    + "It can't be undone here, though forking from an earlier turn still has the code as it was.")
        }
    }
}
