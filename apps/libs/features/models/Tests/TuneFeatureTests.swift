import ComposableArchitecture
import Foundation
import ModelsClient
import ModelsFeatures
import XCTest

@MainActor
final class TuneFeatureTests: XCTestCase {
    private let gemma = ModelCatalog.gemma4E2B

    private var gguf: ModelVariant {
        get throws { try XCTUnwrap(gemma.variants.first { $0.runtime == .llama }) }
    }

    private func tune(
        _ library: InMemoryModelLibrary, dependencies: (inout DependencyValues) -> Void = { _ in }
    ) async -> TestStoreOf<TuneFeature> {
        let store = TestStore(initialState: TuneFeature.State(model: gemma)) {
            TuneFeature()
        } withDependencies: {
            $0.models = library.client
            dependencies(&$0)
        }
        store.exhaustivity = .off
        await store.send(.task)
        await store.receive(\.loaded)
        return store
    }

    func testTuningOpensFromModelsForADownloadedModelOnly() async {
        let library = InMemoryModelLibrary(installed: [gemma])
        let store = TestStore(initialState: ModelsFeature.State()) {
            ModelsFeature()
        } withDependencies: {
            $0.models = library.client
        }
        store.exhaustivity = .off
        await store.send(.task)
        await store.receive(\.loaded)

        await store.send(.tuneTapped(ModelCatalog.qwen35Tiny.id))
        XCTAssertNil(store.state.tune)
        await store.send(.tuneTapped(gemma.id))

        XCTAssertEqual(store.state.tune?.model.variants.map(\.runtime), [.liteRT, .llama])
        await store.skipInFlightEffects()
    }

    func testOpeningListsEachDownloadedFormatOnEachBackendAndRunsNothing() async throws {
        let library = InMemoryModelLibrary(installed: [gemma])
        let store = await tune(library)

        XCTAssertEqual(store.state.candidates.map(\.runtime), [.liteRT, .liteRT])
        XCTAssertEqual(store.state.candidates.map(\.options.backend), [.gpu, .cpu])
        XCTAssertEqual(store.state.results, [:])
        XCTAssertFalse(store.state.isRunning)
        XCTAssertEqual(store.state.otherFormats, [try gguf])
        XCTAssertEqual(library.client.tuneResults(gemma.id), [])
    }

    func testTuningMeasuresEachSetupSideBySideAndScoresItsAnswers() async {
        let library = InMemoryModelLibrary(installed: [gemma])
        let store = await tune(library)

        await store.send(.tuneTapped)
        XCTAssertTrue(store.state.isRunning)
        XCTAssertEqual(store.state.running, store.state.candidates.first)
        await store.receive(\.compared)
        await store.receive(\.compared)
        await store.receive(\.finished)

        XCTAssertFalse(store.state.isRunning)
        let gpu = store.state.results[store.state.candidates[0]]
        let cpu = store.state.results[store.state.candidates[1]]
        XCTAssertEqual(gpu?.measurement.backend, .gpu)
        XCTAssertEqual(cpu?.measurement.backend, .cpu)
        XCTAssertGreaterThan(gpu?.measurement.decodeTokensPerSecond ?? 0, cpu?.measurement.decodeTokensPerSecond ?? 0)
        // The CPU runs at full precision, so it's the reference.
        XCTAssertEqual(cpu?.quality?.isReference, true)
        XCTAssertLessThan(gpu?.quality?.agreement ?? 1, 1)
        XCTAssertEqual(library.client.tuneResults(gemma.id).count, 2)
    }

    func testWhatWasMeasuredBeforeShowsWhenItOpensAgain() async {
        let library = InMemoryModelLibrary(installed: [gemma])
        var tuned = library.client.tuneCandidates(gemma.id)[0]
        tuned.options.cpuThreads = 2
        _ = await library.client.compare([tuned]) { _, _ in }

        let store = await tune(library)

        XCTAssertEqual(store.state.candidates.count, 3)
        XCTAssertEqual(store.state.candidates.last, tuned)
        XCTAssertNotNil(store.state.results[tuned])
    }

    func testAFailingEngineIsReportedForItsSetupsAndTheOthersAreMeasured() async throws {
        let library = InMemoryModelLibrary(installed: [gemma])
        library.failComparing(on: .llama, with: .unavailable(.notEnoughMemory(needed: 4, available: 2)))
        _ = try await library.client.download(gemma.running(gguf))
        let store = await tune(library)
        XCTAssertEqual(store.state.candidates.map(\.runtime), [.liteRT, .liteRT, .llama, .llama])

        await store.send(.tuneTapped)
        await store.receive(\.finished)

        let llama = store.state.candidates.filter { $0.runtime == .llama }
        let liteRT = store.state.candidates.filter { $0.runtime == .liteRT }
        XCTAssertTrue(llama.allSatisfy { store.state.failures[$0] != nil && store.state.results[$0] == nil })
        XCTAssertTrue(liteRT.allSatisfy { store.state.results[$0] != nil && store.state.failures[$0] == nil })
    }

    func testAnotherFormatDownloadsOnlyOnTapAndIsThenCompared() async throws {
        let library = InMemoryModelLibrary(installed: [gemma])
        let store = await tune(library)

        await store.send(.downloadTapped(try gguf))
        XCTAssertEqual(store.state.downloading, try gguf)
        await store.receive(\.downloaded)
        await store.receive(\.loaded)

        XCTAssertNil(store.state.downloading)
        XCTAssertEqual(store.state.otherFormats, [])
        XCTAssertEqual(store.state.candidates.map(\.runtime), [.liteRT, .liteRT, .llama, .llama])
    }

    func testADownloadThatFailsSaysWhy() async throws {
        let library = InMemoryModelLibrary(installed: [gemma])
        library.failNextDownload(of: gemma.id, with: .notEnoughSpace(needed: 4, available: 1))
        let store = await tune(library)

        await store.send(.downloadTapped(try gguf))
        await store.receive(\.downloaded)

        XCTAssertNil(store.state.downloading)
        XCTAssertNotNil(store.state.downloadError)
        XCTAssertEqual(store.state.otherFormats, [try gguf])
    }

    func testChangedSettingsAreANewSetupWithNoResultUntilMeasured() async {
        let library = InMemoryModelLibrary(installed: [gemma])
        let store = await tune(library)
        await store.send(.tuneTapped)
        await store.receive(\.finished)

        var options = store.state.candidates[0].options
        options.cpuThreads = 4
        await store.send(.duplicateTapped(0))
        XCTAssertEqual(store.state.editing, 1)
        await store.send(.optionsChanged(options, at: 1))

        XCTAssertEqual(store.state.candidates.count, 3)
        XCTAssertNotNil(store.state.results[store.state.candidates[0]])
        XCTAssertNil(store.state.results[store.state.candidates[1]])

        await store.send(.removeTapped(1))
        XCTAssertEqual(store.state.candidates.count, 2)
    }

    func testTheLastSetupCantBeRemoved() async throws {
        let library = InMemoryModelLibrary(installed: [gemma])
        let store = await tune(library)
        await store.send(.removeTapped(0))

        await store.send(.removeTapped(0))

        XCTAssertEqual(store.state.candidates.count, 1)
    }

    func testTheSetupPickedForEachPurposeIsKept() async {
        let library = InMemoryModelLibrary(installed: [gemma])
        let store = await tune(library)
        let cpu = store.state.candidates[1].setup

        await store.send(.setupPicked(cpu, for: .quick))
        await store.finish()

        XCTAssertEqual(library.client.setups(gemma.id), [.quick: cpu])
        await store.send(.setupPicked(nil, for: .quick))
        await store.finish()
        XCTAssertEqual(library.client.setups(gemma.id), [:])
    }

    func testStoppingKeepsWhatWasMeasuredAndEndsTheRun() async {
        let library = InMemoryModelLibrary(installed: [gemma])
        let measured = TuneResult(
            key: TuneKey(
                device: "iPhone18,1", system: "27.0",
                candidate: library.client.tuneCandidates(gemma.id)[0]),
            measurement: ModelMeasurement(
                backend: .gpu, runtime: .liteRT, quantization: "", window: gemma.contextTokens, device: "iPhone18,1",
                system: "27.0", prefillTokensPerSecond: 400, decodeTokensPerSecond: 24, timeToFirstToken: 0.6,
                peakMemoryBytes: 1, cpuSecondsPerToken: 0, cpuJoulesPerToken: nil, gpuSecondsPerToken: nil,
                measuredAt: Date(timeIntervalSince1970: 0)))
        let store = await tune(library) {
            $0.models.compare = { candidates, report in
                await report(candidates[0], .measured(measured))
                try? await Task.sleep(for: .seconds(60))
                return candidates.map { _ in .skipped }
            }
        }

        await store.send(.tuneTapped)
        await store.receive(\.compared)
        XCTAssertEqual(store.state.running, store.state.candidates[1])
        await store.send(.stopTapped)

        XCTAssertFalse(store.state.isRunning)
        XCTAssertNil(store.state.running)
        XCTAssertEqual(store.state.results[store.state.candidates[0]], measured)
        await store.finish()
    }
}
