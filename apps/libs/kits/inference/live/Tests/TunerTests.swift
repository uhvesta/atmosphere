import Foundation
import InferenceKit
import XCTest

@testable import InferenceKitLive

final class TunerTests: XCTestCase {
    private let device = DeviceInfo(model: "iPhone18,1", system: "27.0")
    private let host = ModelHost(load: { _, _ throws(ModelRuntimeError) in throw .failed("no") })

    /// Gemma 4 E2B as LiteRT-LM and as GGUF, both downloaded.
    private var installed: [InstalledModel] {
        let entry = ModelCatalog.gemma4E2B
        return entry.variants.map {
            InstalledModel(spec: entry.running($0), directory: URL(filePath: "/"), installedAt: Date())
        }
    }

    private func candidate(
        _ runtime: RuntimeKind, _ backend: ModelBackend, kvCache: EngineOptions.KVCache? = nil
    ) -> TuneCandidate {
        TuneCandidate(
            model: ModelCatalog.gemma4E2B.id, runtime: runtime, quantization: runtime == .llama ? "Q4_K_M" : "",
            options: EngineOptions(backend: backend, window: 8_192, kvCache: kvCache))
    }

    /// A bench whose engines read `decode` tokens a second, by engine and
    /// backend, and answer each prompt with `answers`, by engine and
    /// backend; one that's in `failing` fails.
    private func bench(
        failing: Set<String> = [],
        answers: @escaping @Sendable (InstalledModel, ModelBackend) -> String = { _, _ in
            "the same answer"
        }, measured: Locked<[String]> = Locked([]),
        measure: (@Sendable (InstalledModel, ModelBackend) async throws(ModelRuntimeError) -> Void)? = nil
    ) -> Bench {
        Bench(
            measure: { model, backend throws(ModelRuntimeError) in
                let name = "\(model.spec.runtime) \(backend)"
                measured.withValue { $0.append(name) }
                if failing.contains(name) { throw .unavailable(.notEnoughMemory(needed: 9, available: 1)) }
                try await measure?(model, backend)
                return EngineSpeed(
                    prefillTokens: 256, decodeTokens: 128,
                    prefillTokensPerSecond: model.spec.runtime == .liteRT ? 900 : 600,
                    decodeTokensPerSecond: backend == .gpu ? 40 : 15, timeToFirstToken: backend == .gpu ? 0.3 : 0.8)
            },
            load: { model, backend throws(ModelRuntimeError) in
                FakeEngine(
                    chat: FakeChat([String](repeating: answers(model, backend), count: TunePrompts.standard.count)))
            }, device: device, footprint: { 0 }, cpuSeconds: { 0 }, energy: { nil },
            now: { Date(timeIntervalSince1970: 0) }, interval: .seconds(3_600))
    }

    func testEachCandidateIsMeasuredOnItsOwnEngineAndBackend() async throws {
        let candidates = [candidate(.liteRT, .gpu), candidate(.llama, .gpu), candidate(.llama, .cpu)]
        let measured = Locked<[String]>([])

        let outcomes = await bench(measured: measured).tune(candidates, of: installed, host: host)

        XCTAssertEqual(measured.value, ["liteRT gpu", "llama gpu", "llama cpu"])
        let results = outcomes.compactMap(\.result)
        XCTAssertEqual(results.map(\.measurement.runtime), [.liteRT, .llama, .llama])
        XCTAssertEqual(results.map(\.measurement.backend), [.gpu, .gpu, .cpu])
        XCTAssertEqual(results.map(\.measurement.prefillTokensPerSecond), [900, 600, 600])
        XCTAssertEqual(results.map(\.measurement.decodeTokensPerSecond), [40, 40, 15])
        XCTAssertEqual(results.map(\.key), candidates.map { $0.key(device: "iPhone18,1", system: "27.0") })
    }

    func testAnswersAreScoredAgainstTheMostPreciseCandidates() async throws {
        let candidates = [candidate(.llama, .gpu, kvCache: .q4), candidate(.llama, .cpu, kvCache: .f16)]
        let bench = bench(answers: { _, backend in
            backend == .cpu ? "ice is less dense than water" : "ice is less dense than stone"
        })

        let outcomes = await bench.tune(candidates, of: installed, host: host)

        let gpu = try XCTUnwrap(outcomes.first?.result?.quality)
        let cpu = try XCTUnwrap(outcomes.last?.result?.quality)
        XCTAssertEqual(gpu.agreement, 5.0 / 6, accuracy: 1e-9)
        XCTAssertFalse(gpu.isReference)
        XCTAssertEqual(gpu.reference, candidates[1])
        XCTAssertEqual(cpu.agreement, 1)
        XCTAssertTrue(cpu.isReference)
    }

    func testALongAnswerIsCutAndScoredOnItsFirstWords() async throws {
        let long = (1...100).map { "word\($0)" }.joined(separator: " ")
        let candidates = [candidate(.llama, .cpu), candidate(.llama, .gpu)]
        let bench = bench(answers: { _, backend in
            backend == .cpu ? long : (1...TunePrompts.answerWords).map { "word\($0)" }.joined(separator: " ")
        })

        let outcomes = await bench.tune(candidates, of: installed, host: host)

        XCTAssertEqual(outcomes.last?.result?.quality?.agreement, 1)
    }

    func testAFailingCandidateIsReportedAndTheOthersGoOn() async throws {
        let candidates = [candidate(.liteRT, .gpu), candidate(.llama, .gpu), candidate(.llama, .cpu)]
        let reported = Locked<[TuneOutcome]>([])

        let outcomes = await bench(failing: ["llama gpu"]).tune(candidates, of: installed, host: host) { _, outcome in
            reported.withValue { $0.append(outcome) }
        }

        XCTAssertNotNil(outcomes[0].result)
        XCTAssertEqual(outcomes[1], .failed(.engine(.unavailable(.notEnoughMemory(needed: 9, available: 1)))))
        XCTAssertNotNil(outcomes[2].result)
        XCTAssertEqual(reported.value.count, 3)
        XCTAssertNotNil(outcomes[2].result?.quality)
    }

    func testOptionsTheEngineCantTakeFailWithoutRunningIt() async throws {
        var cache = candidate(.liteRT, .gpu)
        cache.options.kvCache = .q8
        var window = candidate(.llama, .gpu)
        window.options.window = 5_000
        let measured = Locked<[String]>([])

        let outcomes = await bench(measured: measured).tune([cache, window], of: installed, host: host)

        XCTAssertEqual(outcomes[0], .failed(.invalidOptions(.unsupported(.kvCache, .liteRT))))
        XCTAssertEqual(
            outcomes[1], .failed(.invalidOptions(.windowUnavailable(5_000, offered: [4_096, 8_192, 16_384, 32_768]))))
        XCTAssertEqual(measured.value, [])
    }

    func testAFormatThatIsntDownloadedIsntRun() async throws {
        let liteRTOnly = installed.filter { $0.spec.runtime == .liteRT }

        let outcomes = await bench().tune([candidate(.llama, .gpu)], of: liteRTOnly, host: host)

        XCTAssertEqual(outcomes, [.failed(.notDownloaded)])
    }

    func testTheOptionsReachTheEngine() async throws {
        var tuned = candidate(.llama, .gpu)
        tuned.options = EngineOptions(
            backend: .gpu, window: 4_096, cpuThreads: 3, gpuLayers: 12, batchTokens: 256, kvCache: .f16)
        let seen = Locked<[InstalledModel]>([])
        let bench = bench(measure: { model, _ throws(ModelRuntimeError) in seen.withValue { $0.append(model) } })

        _ = await bench.tune([tuned], of: installed, host: host)

        let model = try XCTUnwrap(seen.value.first)
        XCTAssertEqual(model.options, tuned.options)
        XCTAssertEqual(model.window, 4_096)
        XCTAssertEqual(model.profile.cpuThreads, 3)
        XCTAssertEqual(model.profile.sampling, .greedy)
    }

    func testCancellingStopsTheRunAndSkipsTheRest() async throws {
        let candidates = [candidate(.liteRT, .gpu), candidate(.llama, .gpu), candidate(.llama, .cpu)]
        let (started, starting) = AsyncStream<Void>.makeStream()
        let bench = bench(measure: { model, backend throws(ModelRuntimeError) in
            guard model.spec.runtime == .llama, backend == .gpu else { return }
            starting.yield()
            do {
                try await Task.sleep(for: .seconds(60))
            } catch {
                throw .cancelled
            }
        })
        let installed = installed
        let host = host

        let run = Task { await bench.tune(candidates, of: installed, host: host) }
        for await _ in started { break }
        run.cancel()
        let outcomes = await run.value

        XCTAssertNotNil(outcomes[0].result)
        XCTAssertEqual(outcomes[1], .skipped)
        XCTAssertEqual(outcomes[2], .skipped)
    }
}

extension TuneOutcome {
    fileprivate var result: TuneResult? {
        if case .measured(let result) = self { result } else { nil }
    }
}
