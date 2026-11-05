import ComposableArchitecture
import Foundation
import LifrAssistantClient

/// The person's instructions for the assistant: the text in use, an
/// editor for a new version, and every version saved, any of which, or
/// Lifr's default, can be used again.
@Reducer
package struct InstructionsFeature {
    @ObservableState
    package struct State: Equatable {
        package var guidance: Guidance
        /// What's in the editor.
        package var draft: String
        package var isLoaded: Bool
        package var isSaving = false
        package var error: GuidanceError?

        package init(guidance: Guidance = Guidance(), isLoaded: Bool = false) {
            self.guidance = guidance
            self.draft = guidance.text
            self.isLoaded = isLoaded
        }

        package var count: Int { draft.trimmingCharacters(in: .whitespacesAndNewlines).count }
        package var isTooLong: Bool { count > Guidance.limit }
        package var canSave: Bool {
            isLoaded && !isSaving && count > 0 && !isTooLong
                && draft.trimmingCharacters(in: .whitespacesAndNewlines) != guidance.text
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case task
        case loaded(Result<Guidance, GuidanceError>)
        case saveTapped
        case useTapped(UUID?)
        case saved(Result<Guidance, GuidanceError>)
    }

    @Dependency(\.guidance) private var client
    @Dependency(\.date.now) private var now
    @Dependency(\.uuid) private var uuid

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .binding(\.draft):
                state.error = nil
                return .none

            case .binding:
                return .none

            case .task:
                guard !state.isLoaded else { return .none }
                return .run { [client] send in
                    await send(.loaded(Result { () async throws(GuidanceError) in try await Self.run(client.load) }))
                }

            case .loaded(.success(let guidance)):
                state.guidance = guidance
                state.draft = guidance.text
                state.isLoaded = true
                state.error = nil
                return .none

            case .loaded(.failure(let error)):
                state.error = error
                return .none

            case .saveTapped:
                guard state.canSave else { return .none }
                var next = state.guidance
                do throws(GuidanceError) {
                    try next.save(state.draft, id: uuid(), at: now)
                } catch {
                    state.error = error
                    return .none
                }
                return save(next, in: &state)

            case .useTapped(let id):
                guard state.isLoaded, !state.isSaving, id != state.guidance.current else { return .none }
                var next = state.guidance
                next.use(id)
                return save(next, in: &state)

            case .saved(.success(let guidance)):
                state.isSaving = false
                state.guidance = guidance
                state.draft = guidance.text
                state.error = nil
                return .none

            case .saved(.failure(let error)):
                state.isSaving = false
                state.error = error
                return .none
            }
        }
    }

    private func save(_ guidance: Guidance, in state: inout State) -> Effect<Action> {
        state.isSaving = true
        return .run { [client] send in
            await send(
                .saved(
                    Result { () async throws(GuidanceError) in
                        try await Self.run { try await client.save(guidance) }
                        return guidance
                    }))
        }
    }

    /// Runs `body`, reporting any failure as a `GuidanceError`.
    private static func run<Value>(_ body: () async throws -> Value) async throws(GuidanceError) -> Value {
        do {
            return try await body()
        } catch let error as GuidanceError {
            throw error
        } catch {
            throw .storageFailed(error.localizedDescription)
        }
    }
}
