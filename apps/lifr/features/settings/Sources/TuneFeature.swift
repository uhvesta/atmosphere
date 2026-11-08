import ComposableArchitecture
import Foundation
import LifrModelsClient

/// Compares the ways a downloaded model can run on this device: each format
/// it's downloaded in, on each engine and backend, with the engine's
/// settings as the person sets them, side by side. They pick a setup for
/// chats and one for quick tasks, which is what those uses run. Nothing
/// runs until they tap Tune for this device, and nothing downloads until
/// they tap a format.
@Reducer
package struct TuneFeature {
    @ObservableState
    package struct State: Equatable {
        /// The model, with every format it comes in.
        package var model: ModelSpec
        package var candidates: [TuneCandidate] = []
        /// The latest result of each candidate measured here.
        package var results: [TuneCandidate: TuneResult] = [:]
        /// Why a candidate in the last run has no result.
        package var failures: [TuneCandidate: String] = [:]
        /// The candidate running now.
        package var running: TuneCandidate?
        package var isRunning = false
        package var setups: [TunePurpose: TunedSetup] = [:]
        package var otherFormats: [ModelVariant] = []
        package var downloading: ModelVariant?
        package var downloadError: String?
        /// The index of the candidate whose settings are open.
        package var editing: Int?

        package init(model: ModelSpec) {
            self.model = model
        }

        /// Adds each of `more` that isn't listed yet, after those listed.
        fileprivate mutating func list(_ more: [TuneCandidate]) {
            for candidate in more where !candidates.contains(candidate) {
                candidates.append(candidate)
            }
        }

        fileprivate mutating func record(_ outcome: TuneOutcome, of candidate: TuneCandidate) {
            switch outcome {
            case .measured(let result):
                results[candidate] = result
                failures[candidate] = nil
            case .failed(let failure):
                failures[candidate] = failure.description
            case .skipped:
                break
            }
        }
    }

    package enum Action {
        case task
        /// The candidates for each format downloaded, what was measured
        /// before, latest first, what's picked, and the formats to offer.
        case loaded(
            candidates: [TuneCandidate], results: [TuneResult], setups: [TunePurpose: TunedSetup],
            otherFormats: [ModelVariant])
        case tuneTapped
        /// One candidate is done, before qualities are known.
        case compared(TuneCandidate, TuneOutcome)
        /// The run is done: every candidate's outcome, with qualities.
        case finished([TuneOutcome])
        case stopTapped
        case editTapped(Int?)
        case optionsChanged(EngineOptions, at: Int)
        case duplicateTapped(Int)
        case removeTapped(Int)
        case setupPicked(TunedSetup?, for: TunePurpose)
        case downloadTapped(ModelVariant)
        case downloaded(Result<InstalledModel, ModelDownloadError>)
    }

    private enum CancelID { case run }

    @Dependency(\.models) private var library

    package init() {}

    package var body: some ReducerOf<Self> {
        Reduce { state, action in
            switch action {
            case .task:
                return load(state.model.id)

            case .loaded(let candidates, let results, let setups, let otherFormats):
                state.list(candidates)
                // Setups measured before with other settings come back too,
                // each with its latest result.
                for result in results.reversed() { state.results[result.key.candidate] = result }
                state.list(results.map(\.key.candidate))
                state.setups = setups
                state.otherFormats = otherFormats
                return .none

            case .tuneTapped:
                guard !state.isRunning, !state.candidates.isEmpty else { return .none }
                state.isRunning = true
                state.failures = [:]
                state.running = state.candidates.first
                let candidates = state.candidates
                return .run { [library] send in
                    let outcomes = await library.compare(candidates) { candidate, outcome in
                        await send(.compared(candidate, outcome))
                    }
                    await send(.finished(outcomes))
                }
                .cancellable(id: CancelID.run, cancelInFlight: true)

            case .compared(let candidate, let outcome):
                state.record(outcome, of: candidate)
                let next = state.candidates.firstIndex(of: candidate).map { $0 + 1 }
                state.running = next.flatMap { $0 < state.candidates.count ? state.candidates[$0] : nil }
                return .none

            case .finished(let outcomes):
                for (candidate, outcome) in zip(state.candidates, outcomes) {
                    state.record(outcome, of: candidate)
                }
                state.isRunning = false
                state.running = nil
                return .none

            case .stopTapped:
                state.isRunning = false
                state.running = nil
                return .cancel(id: CancelID.run)

            case .editTapped(let index):
                state.editing = index.flatMap { state.candidates.indices.contains($0) ? $0 : nil }
                return .none

            case .optionsChanged(let options, let index):
                guard !state.isRunning, state.candidates.indices.contains(index) else { return .none }
                state.candidates[index].options = options
                return .none

            case .duplicateTapped(let index):
                guard !state.isRunning, state.candidates.indices.contains(index) else { return .none }
                state.candidates.insert(state.candidates[index], at: index + 1)
                state.editing = index + 1
                return .none

            case .removeTapped(let index):
                guard !state.isRunning, state.candidates.count > 1, state.candidates.indices.contains(index) else {
                    return .none
                }
                state.candidates.remove(at: index)
                state.editing = nil
                return .none

            case .setupPicked(let setup, let purpose):
                state.setups[purpose] = setup
                let id = state.model.id
                return .run { [library] _ in await library.pickSetup(setup, purpose, id) }

            case .downloadTapped(let variant):
                guard state.downloading == nil, state.otherFormats.contains(variant) else { return .none }
                state.downloading = variant
                state.downloadError = nil
                let spec = state.model.running(variant)
                return .run { [library] send in
                    do throws(ModelDownloadError) {
                        await send(.downloaded(.success(try await library.download(spec))))
                    } catch {
                        await send(.downloaded(.failure(error)))
                    }
                }

            case .downloaded(.success):
                state.downloading = nil
                return load(state.model.id)

            case .downloaded(.failure(let error)):
                state.downloading = nil
                state.downloadError = error.description
                return .none
            }
        }
    }

    private func load(_ id: ModelID) -> Effect<Action> {
        .run { [library] send in
            await send(
                .loaded(
                    candidates: library.tuneCandidates(id), results: library.tuneResults(id),
                    setups: library.setups(id), otherFormats: library.otherFormats(id)))
        }
    }
}
