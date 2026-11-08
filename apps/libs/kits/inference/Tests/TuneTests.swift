import Foundation
import InferenceKit
import XCTest

final class EngineOptionsTests: XCTestCase {
    private let gemma = ModelCatalog.gemma4E2B

    func testTheEnginesOwnSettingsAreValid() throws {
        for runtime in [RuntimeKind.liteRT, .llama] {
            for backend in runtime.backends {
                XCTAssertNoThrow(try EngineOptions(backend: backend, window: 8_192).validate(for: gemma, on: runtime))
            }
        }
    }

    func testEverySettingLlamaHasIsValidTogether() {
        let options = EngineOptions(
            backend: .gpu, window: 4_096, cpuThreads: 4, gpuLayers: 20, batchTokens: 1_024, microBatchTokens: 256,
            kvCache: .q4_0, flashAttention: true)

        XCTAssertNoThrow(try options.validate(for: gemma, on: .llama))
    }

    func testASettingTheEngineDoesntHaveIsRefused() {
        let cache = EngineOptions(backend: .gpu, window: 8_192, kvCache: .q8_0)
        let threads = EngineOptions(backend: .gpu, window: 8_192, cpuThreads: 4)

        XCTAssertEqual(validation(cache, on: .liteRT), .unsupported(.kvCache, .liteRT))
        XCTAssertEqual(validation(threads, on: .mlx), .unsupported(.cpuThreads, .mlx))
    }

    func testAnEngineThatDoesntRunOnTheCPUIsntTunedThere() {
        XCTAssertEqual(
            validation(EngineOptions(backend: .cpu, window: 8_192), on: .mlx), .backendUnavailable(.cpu, .mlx))
    }

    func testAWindowTheModelDoesntOfferIsRefused() {
        XCTAssertEqual(
            validation(EngineOptions(backend: .gpu, window: 5_000), on: .llama),
            .windowUnavailable(5_000, offered: [4_096, 8_192, 16_384, 32_768]))
    }

    func testCountsUnderTheirLeastAreRefused() {
        XCTAssertEqual(
            validation(EngineOptions(backend: .cpu, window: 8_192, cpuThreads: 0), on: .llama),
            .outOfRange(.cpuThreads, 0))
        XCTAssertEqual(
            validation(EngineOptions(backend: .gpu, window: 8_192, gpuLayers: -1), on: .llama),
            .outOfRange(.gpuLayers, -1))
        XCTAssertEqual(
            validation(EngineOptions(backend: .gpu, window: 8_192, batchTokens: 0), on: .llama),
            .outOfRange(.batchTokens, 0))
    }

    func testGPULayersNeedTheGPU() {
        XCTAssertEqual(
            validation(EngineOptions(backend: .cpu, window: 8_192, gpuLayers: 10), on: .llama), .gpuLayersOnCPU)
        XCTAssertNoThrow(
            try EngineOptions(backend: .cpu, window: 8_192, gpuLayers: 0).validate(for: gemma, on: .llama))
    }

    func testAMicroBatchFitsInItsBatch() {
        XCTAssertEqual(
            validation(
                EngineOptions(backend: .gpu, window: 8_192, batchTokens: 256, microBatchTokens: 512), on: .llama),
            .microBatchOverBatch(micro: 512, batch: 256))
        XCTAssertEqual(
            validation(EngineOptions(backend: .gpu, window: 8_192, microBatchTokens: 1_024), on: .llama),
            .microBatchOverBatch(micro: 1_024, batch: EngineOptions.llamaBatchTokens))
    }

    func testAQuantizedCacheNeedsFlashAttention() {
        XCTAssertEqual(
            validation(EngineOptions(backend: .gpu, window: 8_192, kvCache: .q8_0, flashAttention: false), on: .llama),
            .quantizedCacheNeedsFlashAttention)
        XCTAssertNoThrow(
            try EngineOptions(backend: .gpu, window: 8_192, kvCache: .f16, flashAttention: false)
                .validate(for: gemma, on: .llama))
    }

    private func validation(_ options: EngineOptions, on runtime: RuntimeKind) -> EngineOptionsError? {
        do throws(EngineOptionsError) {
            try options.validate(for: gemma, on: runtime)
            return nil
        } catch {
            return error
        }
    }
}

final class TuneTests: XCTestCase {
    private let candidate = TuneCandidate(
        model: "gemma-4-e2b", runtime: .llama, quantization: "Q4_K_M",
        options: EngineOptions(backend: .gpu, window: 8_192))

    func testResultsAreKeptApartByDeviceSystemFormatAndOptions() {
        var cache = candidate
        cache.options.kvCache = .q4_0
        var format = candidate
        format.runtime = .liteRT
        format.quantization = ""
        let keys = [
            candidate.key(device: "iPhone18,1", system: "27.0"), candidate.key(device: "Mac16,10", system: "27.0"),
            candidate.key(device: "iPhone18,1", system: "26.4"), cache.key(device: "iPhone18,1", system: "27.0"),
            format.key(device: "iPhone18,1", system: "27.0"),
        ]

        XCTAssertEqual(Set(keys).count, keys.count)
        XCTAssertEqual(candidate.key(device: "iPhone18,1", system: "27.0"), keys[0])
    }

    func testAKeyReadsBackAsItWasKept() throws {
        var options = candidate
        options.options = EngineOptions(
            backend: .gpu, window: 4_096, cpuThreads: 2, gpuLayers: 8, batchTokens: 256, microBatchTokens: 128,
            kvCache: .q8_0, flashAttention: true)
        let key = options.key(device: "iPhone18,1", system: "27.0")

        XCTAssertEqual(try JSONDecoder().decode(TuneKey.self, from: JSONEncoder().encode(key)), key)
    }

    func testTheSameAnswerAgreesFully() {
        XCTAssertEqual(TuneQuality.agreement("Ice is less dense than water.", "ice is less dense than water"), 1)
        XCTAssertEqual(TuneQuality.agreement("", ""), 1)
    }

    func testAnswersWithNoWordInCommonDontAgree() {
        XCTAssertEqual(TuneQuality.agreement("Boots, water, map", "391"), 0)
        XCTAssertEqual(TuneQuality.agreement("", "391"), 0)
    }

    func testAgreementIsTheWordsInOrderOverTheLongerAnswer() {
        // "ice is less dense" in order, of the reference's six words.
        XCTAssertEqual(
            TuneQuality.agreement("Ice is less dense", "Ice is less dense than water"), 4.0 / 6, accuracy: 1e-9)
        // Only one of the two words keeps its place.
        XCTAssertEqual(TuneQuality.agreement("water ice", "ice water"), 0.5, accuracy: 1e-9)
    }

    func testAgreementOverPromptsIsTheMean() throws {
        let agreement = try XCTUnwrap(TuneQuality.agreement(["391", "boots map"], ["391", "boots water map"]))

        XCTAssertEqual(agreement, (1 + 2.0 / 3) / 2, accuracy: 1e-9)
        XCTAssertNil(TuneQuality.agreement([], []))
    }

    func testTheReferenceIsTheMostPreciseSetup() {
        let gpu = TuneCandidate(
            model: "gemma-4-e2b", runtime: .llama, quantization: "Q4_K_M",
            options: EngineOptions(backend: .gpu, window: 8_192, kvCache: .q8_0))
        var cpu = gpu
        cpu.options.backend = .cpu
        var full = gpu
        full.options.kvCache = .f16

        XCTAssertEqual(tuneReference(of: [gpu, cpu, full]), cpu)
        var fullOnCPU = cpu
        fullOnCPU.options.kvCache = .f16
        XCTAssertEqual(tuneReference(of: [gpu, fullOnCPU, cpu]), fullOnCPU)
        XCTAssertEqual(tuneReference(of: [gpu, gpu]), gpu)
        XCTAssertNil(tuneReference(of: []))
    }

    func testEveryDownloadedFormatIsACandidateOnEachBackendItsEngineHas() {
        let entry = ModelCatalog.qwen35Tiny
        let installed = entry.variants.map {
            InstalledModel(spec: entry.running($0), directory: URL(filePath: "/"), installedAt: Date())
        }

        let candidates = tuneCandidates(for: installed, window: 4_096)

        XCTAssertEqual(candidates.map(\.runtime), [.mlx, .llama, .llama])
        XCTAssertEqual(candidates.map(\.options.backend), [.gpu, .gpu, .cpu])
        XCTAssertTrue(candidates.allSatisfy { $0.options.window == 4_096 })
    }

    func testGemma4E2BRunsOnLiteRTAndLlama() {
        XCTAssertEqual(ModelCatalog.gemma4E2B.runtime, .liteRT)
        XCTAssertEqual(ModelCatalog.gemma4E2B.variants.map(\.runtime), [.liteRT, .llama])
    }

    func testAModelInTwoFormatsIsOneToPick() {
        let entry = ModelCatalog.gemma4E2B
        let installed = entry.variants.map {
            InstalledModel(spec: entry.running($0), directory: URL(filePath: "/"), installedAt: Date())
        }

        XCTAssertEqual(RunnableModel.list(installed).map(\.id), [.apple, entry.id])
    }

    func testAProfileKeptBeforeMarkupReadsAsChatML() throws {
        let old = Data(
            #"{"onScreen":"gpu","gpuActivations":"half","vision":"withModel","audio":"cpu","speculation":"off"}"#.utf8)

        XCTAssertNil(try JSONDecoder().decode(ModelProfile.self, from: old).markup)
    }
}
