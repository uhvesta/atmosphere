import ComposableArchitecture
import Foundation
import LifrAPI
import LifrCore
import LifrSessionClient

/// The sheet for an app that launched the CLI for the first time: what it
/// may do, in which Avenues, for how long.
@Reducer
package struct ApprovalFeature {
    package enum Expiry: Int, CaseIterable, Hashable, Sendable {
        case day = 1
        case week = 7
        case month = 30
        case quarter = 90

        package var title: String {
            switch self {
            case .day: "1 day"
            case .week: "7 days"
            case .month: "30 days"
            case .quarter: "90 days"
            }
        }
    }

    @ObservableState
    package struct State: Equatable {
        package var request: ApprovalRequest
        package var canWrite = false
        package var allAvenues = true
        package var avenueIDs: Set<UUID> = []
        package var expiry = Expiry.month
        package var confirmWrites = false
        /// Avenues agents may see; hidden ones aren't offered.
        package var avenues: [AvenueAccess]
        package var error: String?

        package init(request: ApprovalRequest, workspace: Workspace) {
            self.request = request
            // Every script an interpreter runs shares its identity.
            if request.isInterpreter { expiry = .day }
            avenues = workspace.avenues.values
                .filter { $0.archivedAt == nil && $0.agentAccess != .hidden }
                .sorted { ($0.sortOrder, $0.id.uuidString) < ($1.sortOrder, $1.id.uuidString) }
                .map { AvenueAccess(id: $0.id, name: $0.name, access: $0.agentAccess) }
        }

        /// Whether the answer gives any access: a chosen list can't be empty.
        package var canApprove: Bool { allAvenues || !avenueIDs.isEmpty }

        package func grant(now: Date) -> Grant {
            Grant(
                client: request.client, scopes: canWrite ? [.read, .write] : [.read],
                avenueIDs: allAvenues ? nil : avenueIDs,
                expiresAt: now + Double(expiry.rawValue) * 24 * 60 * 60, confirmWrites: canWrite && confirmWrites)
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case approveTapped
        case denyTapped
        case answered(Result<Void, any Error>)
    }

    @Dependency(\.session) private var session
    @Dependency(\.dismiss) private var dismiss
    @Dependency(\.date.now) private var now

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .approveTapped:
                guard state.canApprove else { return .none }
                state.error = nil
                return .run { [session, id = state.request.id, grant = state.grant(now: now)] send in
                    await send(.answered(Result { try await session.approve(id, grant) }))
                }
            case .denyTapped:
                return .run { [session, id = state.request.id] send in
                    await send(.answered(Result { try await session.deny(id) }))
                }
            case .answered(.success):
                return .run { [dismiss] _ in await dismiss() }
            case .answered(.failure(let error)):
                state.error = (error as? SessionError)?.description ?? error.localizedDescription
                return .none
            case .binding:
                return .none
            }
        }
    }
}
