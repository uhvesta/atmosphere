import ComposableArchitecture
import Foundation
import HumataDaemonClient

/// Something to fill a repository row with: a saved repository, or a
/// folder that completes the path typed.
package enum RepositorySuggestion: Hashable, Sendable, Identifiable {
    case saved(SavedRepository)
    /// An absolute path.
    case folder(String)

    package var id: String {
        switch self {
        case .saved(let repository): "saved:\(repository.id)"
        case .folder(let path): "folder:\(path)"
        }
    }

    /// What the row's field is filled with.
    package var location: String {
        switch self {
        case .saved(let repository): repository.location
        case .folder(let path): path
        }
    }

    /// What it's called: a saved repository's name, or a folder's own.
    package var name: String {
        switch self {
        case .saved(let repository): repository.name
        case .folder(let path): (path as NSString).lastPathComponent
        }
    }
}

/// Finding what to fill a repository row with. Pure and off the main
/// thread: the reducer runs these in its effects.
package enum RepositorySearch {
    /// How many suggestions show under a row.
    package static let limit = 8

    /// What to suggest for `query`: nothing for a URL, which is taken as
    /// typed; the folders that complete a path from `/` or `~/`; and
    /// otherwise the saved repositories that match, best first.
    package static func suggestions(
        for query: String, in saved: [SavedRepository], home: String, limit: Int = Self.limit
    ) -> [RepositorySuggestion] {
        let query = query.trimmingCharacters(in: .whitespacesAndNewlines)
        if Repository.isURL(query) { return [] }
        if query.hasPrefix("/") || query.hasPrefix("~/") || query == "~" {
            return folders(completing: query, home: home, limit: limit).map(RepositorySuggestion.folder)
        }
        return ranked(query, in: saved, limit: limit).map(RepositorySuggestion.saved)
    }

    /// The saved repositories that match `query` by name or location, best
    /// first: a name that starts with it, then a word of either that does,
    /// then either that holds it, then a name with its letters in order.
    /// Ties keep the order given, which is last used first. Case is
    /// ignored; with nothing typed, the first `limit` as given.
    package static func ranked(
        _ query: String, in saved: [SavedRepository], limit: Int = Self.limit
    ) -> [SavedRepository] {
        let query = query.lowercased()
        guard !query.isEmpty else { return Array(saved.prefix(limit)) }
        let scored = saved.enumerated().compactMap { index, repository -> (rank: Int, index: Int)? in
            let name = rank(of: repository.name.lowercased(), for: query, inOrder: true)
            let location = rank(of: repository.location.lowercased(), for: query, inOrder: false).map { max($0, 1) }
            guard let best = [name, location].compactMap({ $0 }).min() else { return nil }
            return (best, index)
        }
        return scored.sorted { $0.rank != $1.rank ? $0.rank < $1.rank : $0.index < $1.index }
            .prefix(limit)
            .map { saved[$0.index] }
    }

    /// 0 for a start, 1 for a word's start, 2 for anywhere, 3 for the
    /// letters in order when `inOrder`; nil for no match.
    private static func rank(of text: String, for query: String, inOrder: Bool) -> Int? {
        if text.hasPrefix(query) { return 0 }
        let words = text.split { "-_:./ @".contains($0) }
        if words.contains(where: { $0.hasPrefix(query) }) { return 1 }
        if text.contains(query) { return 2 }
        guard inOrder else { return nil }
        var rest = query[...]
        for character in text where rest.first == character { rest = rest.dropFirst() }
        return rest.isEmpty ? 3 : nil
    }

    /// The folders that complete `path`, from `/` or `~/` for `home`: in
    /// the folder it names up to its last `/`, those whose names start
    /// with what follows, by name. Hidden folders only once a `.` is
    /// typed. Absolute paths, never ending in `/`.
    package static func folders(completing path: String, home: String, limit: Int = Self.limit) -> [String] {
        var expanded = path
        if expanded == "~" || expanded.hasPrefix("~/") { expanded = home + expanded.dropFirst() }
        guard expanded.hasPrefix("/"), let slash = expanded.lastIndex(of: "/") else { return [] }
        let folder = String(expanded[...slash])
        let typed = expanded[expanded.index(after: slash)...].lowercased()
        let manager = FileManager.default
        guard let names = try? manager.contentsOfDirectory(atPath: folder) else { return [] }
        let candidates =
            names
            .filter { name in
                (typed.hasPrefix(".") || !name.hasPrefix(".")) && name.lowercased().hasPrefix(typed)
            }
            .sorted { $0.localizedStandardCompare($1) == .orderedAscending }
            .lazy
            .map { folder + $0 }
            .filter { candidate in
                var isFolder: ObjCBool = false
                return manager.fileExists(atPath: candidate, isDirectory: &isFolder) && isFolder.boolValue
            }
        return Array(candidates.prefix(limit))
    }
}

/// The list under a project's repository field while it's being typed in:
/// the saved repositories that match, recent first, or the folders that
/// complete a path. Arrows move, Return or Tab picks, Escape closes it
/// until the text changes. Matching and reading folders run off the main
/// thread.
@Reducer
package struct RepositorySearchFeature {
    @ObservableState
    package struct State: Equatable {
        /// Nil until first read.
        package var saved: [SavedRepository]?
        package var loading = false
        /// Why the saved repositories couldn't be read, and what to do.
        package var failure: String?
        /// The row whose list shows; nil while none does.
        package var row: RepositoryRow.ID?
        /// The text the suggestions are for.
        var query = ""
        package var suggestions: [RepositorySuggestion] = []
        package var selected = 0
        /// The row and text the list was closed or picked on, so it stays
        /// shut until that row's text changes.
        var closedOn: Closed?

        struct Closed: Equatable {
            var row: RepositoryRow.ID
            var text: String
        }

        package init() {}

        /// The suggestion Return would pick.
        package var highlighted: RepositorySuggestion? {
            suggestions.indices.contains(selected) ? suggestions[selected] : nil
        }
    }

    package enum Action {
        /// The row's text changed, or it took the focus with this text.
        case typed(row: RepositoryRow.ID, String)
        /// The row lost the focus.
        case left(row: RepositoryRow.ID)
        case loaded([SavedRepository])
        case loadFailed(String)
        case suggested(query: String, [RepositorySuggestion])
        case moved(by: Int)
        /// Return or Tab.
        case submitted
        case picked(RepositorySuggestion)
        /// Escape.
        case closed
        case delegate(Delegate)

        @CasePathable
        package enum Delegate: Equatable {
            /// Fill `row` with `suggestion`'s location.
            case picked(row: RepositoryRow.ID, RepositorySuggestion)
        }
    }

    @Dependency(\.daemon) var daemon

    private enum CancelID { case match }

    package init() {}

    package var body: some ReducerOf<Self> {
        Reduce { state, action in
            switch action {
            case .typed(let row, let text):
                if let closed = state.closedOn, closed.row != row || closed.text != text { state.closedOn = nil }
                guard state.closedOn == nil else {
                    hide(&state)
                    return .cancel(id: CancelID.match)
                }
                if state.row != row { state.suggestions = [] }
                state.row = row
                state.query = text
                var effects = [match(state)]
                if state.saved == nil, !state.loading {
                    state.loading = true
                    state.failure = nil
                    effects.append(
                        .run { send in
                            await send(.loaded(try await daemon.savedRepositories()))
                        } catch: { error, send in
                            await send(.loadFailed(error.localizedDescription))
                        })
                }
                return .merge(effects)

            case .left(let row):
                guard state.row == row else { return .none }
                hide(&state)
                return .cancel(id: CancelID.match)

            case .loaded(let saved):
                state.loading = false
                state.saved = saved
                return state.row == nil ? .none : match(state)

            case .loadFailed(let message):
                state.loading = false
                state.failure = String(
                    localized: """
                        Couldn't read the saved repositories: \(message) Check that Humata's daemon is running; \
                        a folder or URL typed in full still works.
                        """)
                return .none

            case .suggested(let query, let suggestions):
                guard state.row != nil, query == state.query else { return .none }
                if suggestions != state.suggestions { state.selected = 0 }
                state.suggestions = suggestions
                return .none

            case .moved(let by):
                guard !state.suggestions.isEmpty else { return .none }
                state.selected = min(max(state.selected + by, 0), state.suggestions.count - 1)
                return .none

            case .submitted:
                guard let suggestion = state.highlighted else { return .none }
                return .send(.picked(suggestion))

            case .picked(let suggestion):
                guard let row = state.row else { return .none }
                state.closedOn = State.Closed(row: row, text: suggestion.location)
                hide(&state)
                return .merge(.cancel(id: CancelID.match), .send(.delegate(.picked(row: row, suggestion))))

            case .closed:
                guard let row = state.row else { return .none }
                state.closedOn = State.Closed(row: row, text: state.query)
                hide(&state)
                return .cancel(id: CancelID.match)

            case .delegate:
                return .none
            }
        }
    }

    private func hide(_ state: inout State) {
        state.row = nil
        state.query = ""
        state.suggestions = []
        state.selected = 0
    }

    /// Matches off the main thread; a newer query cancels an older one.
    private func match(_ state: State) -> Effect<Action> {
        let (query, saved) = (state.query, state.saved ?? [])
        return .run { send in
            let home = FileManager.default.homeDirectoryForCurrentUser.path(percentEncoded: false)
            let suggestions = RepositorySearch.suggestions(
                for: query, in: saved, home: home.hasSuffix("/") ? String(home.dropLast()) : home)
            await send(.suggested(query: query, suggestions))
        }
        .cancellable(id: CancelID.match, cancelInFlight: true)
    }
}
