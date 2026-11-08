import ComposableArchitecture
import Foundation
import ModelsClient

/// The Models screen's voices: the one that reads aloud, for every
/// language and for each of the app's languages, and the voice models to
/// download. A voice model downloads only when the person taps.
@Reducer
public struct VoicesFeature {
    @ObservableState
    public struct State: Equatable {
        /// The languages, as BCP 47 tags, each of which can have a voice of
        /// its own. An app with one voice for every language has none.
        public var languages: [String]
        public var models: [VoiceModel] = VoiceCatalog.all
        public var installed: Set<ModelID> = []
        public var downloads: [ModelID: DownloadState] = [:]
        /// The voice for every language.
        public var standard: VoiceChoice = .system
        /// Each language's own voice; one missing follows `standard`.
        public var own: [String: VoiceChoice] = [:]
        @Presents public var remove: ConfirmationDialogState<Remove>?
        @Presents public var alert: AlertState<Never>?

        public init(languages: [String] = []) {
            self.languages = languages
        }
    }

    public enum Remove: Equatable, Sendable {
        case confirmed(ModelID)
    }

    public enum Action {
        case task
        case loaded(installed: Set<ModelID>, standard: VoiceChoice, own: [String: VoiceChoice])
        case downloadsChanged([ModelID: DownloadState])
        /// A voice for `language`, or for every language when nil.
        case picked(VoiceChoice?, language: String?)
        case downloadTapped(ModelID)
        case downloaded(ModelID, Result<InstalledModel, ModelDownloadError>)
        case pauseTapped(ModelID)
        case removeTapped(ModelID)
        case removed(ModelID, Result<Void, ModelStorageError>)
        case remove(PresentationAction<Remove>)
        case alert(PresentationAction<Never>)
    }

    @Dependency(\.models) private var library
    @Dependency(\.speech) private var speech

    public init() {}

    public var body: some ReducerOf<Self> {
        Reduce { state, action in
            switch action {
            case .task:
                return .run { [library, speech, languages = state.languages] send in
                    await send(Self.loaded(library, speech, languages: languages))
                    for await downloads in library.downloads() {
                        await send(.downloadsChanged(downloads))
                    }
                }

            case .loaded(let installed, let standard, let own):
                state.installed = installed
                state.standard = standard
                state.own = own
                return .none

            case .downloadsChanged(let downloads):
                let ids = Set(state.models.map(\.id))
                let finished = state.downloads.keys.contains { downloads[$0] == nil }
                state.downloads = downloads.filter { ids.contains($0.key) }
                return finished ? reload(state) : .none

            case .picked(let choice, let language):
                if let language {
                    state.own[language] = choice
                } else {
                    state.standard = choice ?? .system
                }
                return .run { _ in await speech.choose(choice, language) }

            case .downloadTapped(let id):
                guard let model = state.models.first(where: { $0.id == id }) else { return .none }
                return .run { [library] send in
                    do throws(ModelDownloadError) {
                        await send(.downloaded(id, .success(try await library.download(model.spec))))
                    } catch {
                        await send(.downloaded(id, .failure(error)))
                    }
                }

            case .downloaded(let id, .success):
                state.installed.insert(id)
                return .none

            case .downloaded(_, .failure):
                // The row shows why, from `downloads`.
                return .none

            case .pauseTapped(let id):
                return .run { [library] _ in await library.pause(id) }

            case .removeTapped(let id):
                guard let model = state.models.first(where: { $0.id == id }) else { return .none }
                state.remove = ConfirmationDialogState {
                    TextState("Remove \(model.spec.name)?")
                } actions: {
                    ButtonState(role: .destructive, action: .confirmed(id)) { TextState("Remove voice") }
                } message: {
                    TextState(
                        "Its files are deleted from this device. Text it read aloud goes back to the system voice.")
                }
                return .none

            case .remove(.presented(.confirmed(let id))):
                return .run { [library] send in
                    do throws(ModelStorageError) {
                        try await library.delete(id)
                        await speech.forget(id)
                        await send(.removed(id, .success(())))
                    } catch {
                        await send(.removed(id, .failure(error)))
                    }
                }

            case .removed(let id, .success):
                state.installed.remove(id)
                state.downloads[id] = nil
                return reload(state)

            case .removed(_, .failure(let error)):
                state.alert = AlertState {
                    TextState("Couldn't remove the voice")
                } actions: {
                    ButtonState(role: .cancel) { TextState("OK") }
                } message: {
                    TextState(error.description)
                }
                return reload(state)

            case .remove, .alert:
                return .none
            }
        }
        .ifLet(\.$remove, action: \.remove)
        .ifLet(\.$alert, action: \.alert)
    }

    private func reload(_ state: State) -> Effect<Action> {
        .run { [library, speech, languages = state.languages] send in
            await send(Self.loaded(library, speech, languages: languages))
        }
    }

    private static func loaded(_ library: ModelLibraryClient, _ speech: SpeechClient, languages: [String]) async
        -> Action
    {
        let voices = Set(VoiceCatalog.all.map(\.id))
        let installed = Set(await library.installed().map(\.id).filter(voices.contains))
        var own: [String: VoiceChoice] = [:]
        for language in languages {
            own[language] = speech.choice(language)
        }
        return .loaded(installed: installed, standard: speech.choice(nil) ?? .system, own: own)
    }
}
