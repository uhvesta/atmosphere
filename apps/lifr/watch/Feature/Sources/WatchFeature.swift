import ComposableArchitecture
import Foundation
import LifrGlance
import LifrWatchLink

/// The Watch's Today: the iPhone's list, a status for a task picked from
/// its choices, and a new task said or written into the Inbox.
@Reducer
package struct WatchFeature {
    @ObservableState
    package struct State: Equatable {
        /// Today as the iPhone last sent it.
        package var glance: TodayGlance?
        /// The statuses picked here and sent, by task, until a Today the
        /// iPhone wrote after them arrives.
        package var sent: [UUID: Sent] = [:]
        /// The tasks added here and sent, until a Today the iPhone wrote
        /// after them arrives.
        package var added: [Sent.Task] = []
        /// The task whose statuses are showing.
        package var choosing: UUID?
        package var draft = ""
        @Presents package var alert: AlertState<Never>?

        package init(glance: TodayGlance? = nil) {
            self.glance = glance
        }

        /// The task whose statuses are showing, as Today lists it.
        package var chosen: TodayGlance.Row? {
            guard let choosing else { return nil }
            return glance?.days.lazy.flatMap(\.rows).first { $0.id == choosing }
        }
    }

    /// A change sent to the iPhone, and when.
    package struct Sent: Equatable, Sendable {
        package var status: WatchStatus
        package var at: Date

        package struct Task: Equatable, Sendable, Identifiable {
            package var id: UUID
            package var title: String
            package var at: Date
        }
    }

    package enum Action: BindableAction {
        case task
        case glanceReceived(TodayGlance)
        case rowTapped(UUID)
        /// A reminder's Change Status… on the Watch.
        case reminderStatusRequested(UUID)
        case statusPicked(WatchStatus)
        case choiceDismissed
        case addSubmitted
        case sendFailed(WatchRequest, reason: String)
        case binding(BindingAction<State>)
        case alert(PresentationAction<Never>)
    }

    @Dependency(\.watch) var watch
    @Dependency(\.date.now) var now
    @Dependency(\.uuid) var uuid

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .task:
                if state.glance == nil { state.glance = watch.saved() }
                return .run { [watch] send in
                    for await glance in watch.glances() {
                        await send(.glanceReceived(glance))
                    }
                }

            case .glanceReceived(let glance):
                state.glance = glance
                state.sent = state.sent.filter { $0.value.at > glance.asOf }
                state.added.removeAll { $0.at <= glance.asOf }
                return .none

            case .rowTapped(let id):
                state.choosing = id
                return .none

            case .reminderStatusRequested(let id):
                state.choosing = id
                return .none

            case .statusPicked(let status):
                guard let id = state.choosing else { return .none }
                state.choosing = nil
                state.sent[id] = Sent(status: status, at: now)
                return send(.setStatus(id, status))

            case .choiceDismissed:
                state.choosing = nil
                return .none

            case .addSubmitted:
                let title = state.draft.trimmingCharacters(in: .whitespacesAndNewlines)
                state.draft = ""
                guard !title.isEmpty else { return .none }
                state.added.append(Sent.Task(id: uuid(), title: title, at: now))
                return send(.add(title))

            case .sendFailed(let request, let reason):
                switch request {
                case .setStatus(let id, _): state.sent[id] = nil
                case .add(let title): state.added.removeAll { $0.title == title }
                case .snooze: break
                }
                state.alert = AlertState {
                    TextState("Couldn't send the change to the iPhone")
                } message: {
                    TextState(reason)
                }
                return .none

            case .binding:
                return .none

            case .alert:
                return .none
            }
        }
        .ifLet(\.$alert, action: \.alert)
    }

    private func send(_ request: WatchRequest) -> Effect<Action> {
        .run { [watch] send in
            do {
                try await watch.send(request)
            } catch {
                await send(.sendFailed(request, reason: String(describing: error)))
            }
        }
    }
}
