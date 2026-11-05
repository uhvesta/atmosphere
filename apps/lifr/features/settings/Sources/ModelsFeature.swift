import ComposableArchitecture
import Foundation
import LifrModelsClient

/// The models on this device: which one each use runs on, downloading one
/// from the catalog or from a Hugging Face link, and removing one.
/// Downloads go on when the screen closes, and show here when it opens.
@Reducer
package struct ModelsFeature {
    @ObservableState
    package struct State: Equatable {
        /// The catalog, then models added from links, in the order added.
        package var models: [ModelSpec] = []
        package var installed: Set<ModelID> = []
        package var downloads: [ModelID: DownloadState] = [:]
        package var selection: [ModelUse: ModelID] = [:]
        /// The Hugging Face link being added, and its SHA-256 if given.
        package var link = ""
        package var sha256 = ""
        package var isAdding = false
        /// Why the link couldn't be added.
        package var addError: String?
        @Presents package var remove: ConfirmationDialogState<Remove>?
        @Presents package var alert: AlertState<Never>?

        package init() {}

        /// The models a use can run on: Apple's, and those downloaded.
        package var choices: [ModelSpec] {
            models.filter { $0.id == .apple || installed.contains($0.id) }
        }

        /// The models that need a download: all but Apple's.
        package var downloadable: [ModelSpec] { models.filter { $0.id != .apple } }

        package func selected(for use: ModelUse) -> ModelID { selection[use] ?? .apple }

        package var canAdd: Bool { !isAdding && !link.trimmingCharacters(in: .whitespaces).isEmpty }

        fileprivate mutating func add(_ spec: ModelSpec) {
            if let index = models.firstIndex(where: { $0.id == spec.id }) {
                models[index] = spec
            } else {
                models.append(spec)
            }
        }
    }

    package enum Remove: Equatable, Sendable {
        case confirmed(ModelID)
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case task
        case loaded(installed: [InstalledModel], unfinished: [ModelSpec], selection: [ModelUse: ModelID])
        case downloadsChanged([ModelID: DownloadState])
        case picked(ModelID, for: ModelUse)
        case downloadTapped(ModelID)
        case downloaded(ModelID, Result<InstalledModel, ModelDownloadError>)
        case pauseTapped(ModelID)
        case removeTapped(ModelID)
        case removed(ModelID, Result<Void, ModelStorageError>)
        case addTapped
        case resolved(Result<ModelSpec, ModelResolveError>)
        case remove(PresentationAction<Remove>)
        case alert(PresentationAction<Never>)
    }

    @Dependency(\.models) private var library

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .task:
                state.models = library.catalog()
                return .run { [library] send in
                    await send(Self.loaded(library))
                    for await downloads in library.downloads() {
                        await send(.downloadsChanged(downloads))
                    }
                }

            case .loaded(let installed, let unfinished, let selection):
                for spec in unfinished { state.add(spec) }
                for model in installed { state.add(model.spec) }
                state.installed = Set(installed.map(\.id))
                state.selection = selection
                return .none

            case .downloadsChanged(let downloads):
                // One that left finished, perhaps started before Lifr last
                // quit, with no one here waiting for it.
                let finished = state.downloads.keys.contains { downloads[$0] == nil }
                state.downloads = downloads
                return finished ? reload() : .none

            case .picked(let id, let use):
                state.selection[use] = id
                return .run { [library] _ in await library.select(id, use) }

            case .downloadTapped(let id):
                guard let spec = state.models.first(where: { $0.id == id }) else { return .none }
                return download(spec)

            case .downloaded(_, .success(let model)):
                state.add(model.spec)
                state.installed.insert(model.id)
                return .none

            case .downloaded(_, .failure):
                // The download's row shows why, from `downloads`.
                return .none

            case .pauseTapped(let id):
                return .run { [library] _ in await library.pause(id) }

            case .removeTapped(let id):
                guard let spec = state.models.first(where: { $0.id == id }) else { return .none }
                state.remove = ConfirmationDialogState {
                    TextState("Remove \(spec.name)?")
                } actions: {
                    ButtonState(role: .destructive, action: .confirmed(id)) { TextState("Remove model") }
                } message: {
                    TextState("Its files are deleted from this device. Anything that ran on it goes back to Apple's.")
                }
                return .none

            case .remove(.presented(.confirmed(let id))):
                return .run { [library] send in
                    do throws(ModelStorageError) {
                        try await library.delete(id)
                        await send(.removed(id, .success(())))
                    } catch {
                        await send(.removed(id, .failure(error)))
                    }
                }

            case .removed(let id, .success):
                state.installed.remove(id)
                state.downloads[id] = nil
                return reload()

            case .removed(_, .failure(let error)):
                state.alert = .couldNot("Couldn't remove the model", error)
                return reload()

            case .addTapped:
                guard state.canAdd else { return .none }
                state.isAdding = true
                state.addError = nil
                let link = state.link.trimmingCharacters(in: .whitespaces)
                let given = state.sha256.trimmingCharacters(in: .whitespaces)
                return .run { [library] send in
                    do throws(ModelResolveError) {
                        let spec = try await library.resolve(link, given.isEmpty ? nil : given)
                        await send(.resolved(.success(spec)))
                    } catch {
                        await send(.resolved(.failure(error)))
                    }
                }

            case .resolved(.success(let spec)):
                state.isAdding = false
                state.link = ""
                state.sha256 = ""
                state.add(spec)
                return state.installed.contains(spec.id) ? .none : download(spec)

            case .resolved(.failure(let error)):
                state.isAdding = false
                state.addError = error.description
                return .none

            case .binding(\.link), .binding(\.sha256):
                state.addError = nil
                return .none

            case .binding, .remove, .alert:
                return .none
            }
        }
        .ifLet(\.$remove, action: \.remove)
        .ifLet(\.$alert, action: \.alert)
    }

    private func reload() -> Effect<Action> {
        .run { [library] send in await send(Self.loaded(library)) }
    }

    private static func loaded(_ library: ModelLibraryClient) async -> Action {
        let installed = await library.installed()
        var selection: [ModelUse: ModelID] = [:]
        for use in ModelUse.allCases { selection[use] = library.selection(use) }
        return .loaded(installed: installed, unfinished: await library.unfinished(), selection: selection)
    }

    private func download(_ spec: ModelSpec) -> Effect<Action> {
        .run { [library] send in
            do throws(ModelDownloadError) {
                await send(.downloaded(spec.id, .success(try await library.download(spec))))
            } catch {
                await send(.downloaded(spec.id, .failure(error)))
            }
        }
    }
}

extension ModelUse {
    package var title: String {
        switch self {
        case .assistant: String(localized: "Assistant")
        case .skills: String(localized: "Writing skills")
        case .titles: String(localized: "Naming conversations")
        }
    }
}

extension AlertState where Action == Never {
    fileprivate static func couldNot(_ title: LocalizedStringResource, _ error: some CustomStringConvertible) -> Self {
        AlertState {
            TextState(String(localized: title))
        } actions: {
            ButtonState(role: .cancel) { TextState("OK") }
        } message: {
            TextState(error.description)
        }
    }
}
