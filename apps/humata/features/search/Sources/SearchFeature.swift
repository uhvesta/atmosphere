import ComposableArchitecture
import Foundation
import HumataDaemonClient

/// Searching what every session said and did: messages, replies, commands
/// and their output, on every host. It searches as the person types, then
/// again with related words, which find what was said in other words.
/// Opening a hit opens its session at that event.
@Reducer
package struct SearchFeature {
    /// How far back to search, unless the words name a time themselves.
    package enum Period: Hashable, Sendable, CaseIterable {
        case anyTime
        case today
        case pastWeek
        case pastMonth
    }

    @ObservableState
    package struct State: Equatable {
        package var text = ""
        package var period = Period.anyTime
        /// Only this project's sessions; nil for all.
        package var projectID: String?
        /// The projects to pick from.
        package var projects: [Project]
        package var hits: [SearchHit] = []
        /// The words searched, without a time the text named.
        package var words = ""
        /// The time searched, when the text named one.
        package var namedSince: Date?
        package var namedUntil: Date?
        /// The related words searched as well.
        package var related: [String] = []
        package var isSearching = false
        /// Whether the text was searched since it last changed.
        package var searched = false
        package var failure: String?
        /// The text the last search was for, so the field writing back the
        /// same text doesn't search it again.
        var queried = ""
        /// Numbers each search, so only the latest one's results show:
        /// effects can start out of the order they were asked in.
        var request = 0

        package init(projects: [Project] = [], projectID: String? = nil) {
            self.projects = projects
            self.projectID = projectID
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        /// The person pressed Return: search now.
        case submitted
        /// Typing paused for the search numbered `request`.
        case due(request: Int)
        /// Searched: plain, or with `related`.
        case found(request: Int, SearchResults, related: [String])
        /// Both searches are done.
        case finished(request: Int)
        case failed(request: Int, String)
        case hitTapped(SearchHit)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate: Equatable {
            /// Opens the session at `event`, or at its end if nil.
            case openSession(id: String, event: String?)
            /// Opens the project's board at the item.
            case openItem(projectID: String, itemID: String)
        }
    }

    @Dependency(\.daemon) var daemon
    @Dependency(\.relatedWords) var relatedWords
    @Dependency(\.continuousClock) var clock
    @Dependency(\.date) var date
    @Dependency(\.calendar) var calendar

    /// How long typing pauses before it's searched.
    static let pause = Duration.milliseconds(300)

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .binding(\.text):
                guard state.text.trimmingCharacters(in: .whitespacesAndNewlines) != state.queried else {
                    return .none
                }
                return search(&state, after: Self.pause)

            case .binding(\.period), .binding(\.projectID), .submitted:
                return search(&state, after: nil)

            case .binding:
                return .none

            case .due(let request):
                guard request == state.request else { return .none }
                return run(state)

            // A where clause holds for only the pattern before it, so each has one.
            case .found(let request, _, _) where request != state.request,
                .finished(let request) where request != state.request,
                .failed(let request, _) where request != state.request:
                // For text, a period or a project changed since.
                return .none

            case .found(_, let results, let related):
                state.hits = results.hits
                state.words = results.words
                state.namedSince = results.since
                state.namedUntil = results.until
                state.related = related
                state.searched = true
                state.failure = nil
                return .none

            case .finished:
                state.isSearching = false
                return .none

            case .failed(_, let message):
                state.isSearching = false
                state.searched = true
                state.failure = message
                return .none

            case .hitTapped(let hit):
                if let session = hit.sessionID {
                    return .send(.delegate(.openSession(id: session, event: hit.eventID)))
                }
                guard let project = hit.projectID, let item = hit.itemID else { return .none }
                return .send(.delegate(.openItem(projectID: project, itemID: item)))

            case .delegate:
                return .none
            }
        }
    }

    /// Searches the text, plainly and then with related words, once typing
    /// pauses for `pause`.
    private func search(_ state: inout State, after pause: Duration?) -> Effect<Action> {
        let text = state.text.trimmingCharacters(in: .whitespacesAndNewlines)
        state.queried = text
        state.searched = false
        state.request += 1
        guard !text.isEmpty else {
            state.hits = []
            state.related = []
            state.isSearching = false
            state.failure = nil
            return .none
        }
        state.isSearching = true
        guard let pause else { return run(state) }
        let request = state.request
        return .run { send in
            try await clock.sleep(for: pause)
            await send(.due(request: request))
        }
    }

    /// Searches the text plainly, then with related words.
    private func run(_ state: State) -> Effect<Action> {
        let (request, text) = (state.request, state.queried)
        let query = SearchQuery(text: text, projectID: state.projectID, since: since(state.period))
        return .run { send in
            await send(.found(request: request, try await daemon.search(query), related: []))
            let related = await relatedWords.related(text)
            var widened = query
            widened.related = related
            if !related.isEmpty {
                await send(.found(request: request, try await daemon.search(widened), related: related))
            }
            await send(.finished(request: request))
        } catch: { error, send in
            await send(.failed(request: request, error.localizedDescription))
        }
    }

    private func since(_ period: Period) -> Date? {
        let today = calendar.startOfDay(for: date.now)
        return switch period {
        case .anyTime: nil
        case .today: today
        case .pastWeek: calendar.date(byAdding: .day, value: -6, to: today)
        case .pastMonth: calendar.date(byAdding: .month, value: -1, to: today)
        }
    }
}
