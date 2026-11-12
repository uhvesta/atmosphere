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

        package init(projects: [Project] = [], projectID: String? = nil) {
            self.projects = projects
            self.projectID = projectID
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        /// The person pressed Return: search now.
        case submitted
        /// Searched: plain, or with `related`.
        case found(SearchResults, related: [String])
        /// Both searches are done.
        case finished
        case failed(String)
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

    private enum CancelID { case search }

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

            case .found(let results, let related):
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

            case .failed(let message):
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
        guard !text.isEmpty else {
            state.hits = []
            state.related = []
            state.isSearching = false
            state.failure = nil
            return .cancel(id: CancelID.search)
        }
        state.isSearching = true
        let query = SearchQuery(text: text, projectID: state.projectID, since: since(state.period))
        return .run { send in
            if let pause { try await clock.sleep(for: pause) }
            await send(.found(try await daemon.search(query), related: []))
            let related = await relatedWords.related(text)
            var widened = query
            widened.related = related
            if !related.isEmpty {
                await send(.found(try await daemon.search(widened), related: related))
            }
            await send(.finished)
        } catch: { error, send in
            guard !(error is CancellationError) else { return }
            await send(.failed(error.localizedDescription))
        }
        .cancellable(id: CancelID.search, cancelInFlight: true)
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
