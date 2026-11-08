import ComposableArchitecture
import FlowKit
import LifrModelsClient
import LifrSettings
import ModelsFeatures
import SnapshotTesting
import SwiftUI
import XCTest

/// Models with Gemma 4 E2B downloaded at a 16K window, measured on the
/// GPU and the CPU and picked for writing skills, E4B downloading, and a model pasted from a link whose
/// file failed its check.
@MainActor
final class ModelsSnapshotTests: XCTestCase {
    private static let pasted = ModelSpec(
        id: "hf/litert-community/Qwen3-0.6B/Qwen3-0.6B.litertlm", name: "Qwen3 0.6B",
        summary: "litert-community/Qwen3-0.6B", runtime: .liteRT,
        source: ModelSource(
            repo: "litert-community/Qwen3-0.6B", commit: "0123456789abcdef0123456789abcdef01234567",
            files: [
                ModelFile(
                    path: "Qwen3-0.6B.litertlm", size: 614_236_160,
                    digest: .sha256("aa11bb22cc33dd44ee55ff6600aa11bb22cc33dd44ee55ff6600aa11bb22cc33"))
            ]), contextTokens: 4_096, memoryBytes: 900_000_000, license: "See its Hugging Face page")

    /// Measured here at 16K: the GPU twice as fast and light on the CPU.
    private static let measured: ModelTuning = {
        func measurement(
            _ backend: ModelBackend, decode: Double, memory: Int64, cpu: Double, joules: Double, gpu: Double
        ) -> ModelMeasurement {
            ModelMeasurement(
                backend: backend, runtime: .liteRT, window: 16_384, device: "iPhone18,1", system: "27.0",
                prefillTokensPerSecond: decode * 17, decodeTokensPerSecond: decode, timeToFirstToken: 0.6,
                peakMemoryBytes: memory, cpuSecondsPerToken: cpu, cpuJoulesPerToken: joules, gpuSecondsPerToken: gpu,
                measuredAt: Date(timeIntervalSince1970: 0))
        }
        let gpu = measurement(.gpu, decode: 24, memory: 3_328_599_654, cpu: 0.004, joules: 0.012, gpu: 0.03)
        return ModelTuning(
            plan: ModelPlanner.Plan(backend: .gpu, reason: .measured(gpu)),
            measurements: [gpu, measurement(.cpu, decode: 11, memory: 2_899_102_924, cpu: 0.31, joules: 0.85, gpu: 0)])
    }()

    private var models: ModelsFeature.State {
        var state = ModelsFeature.State()
        state.models = ModelCatalog.all + [Self.pasted]
        state.installed = [ModelCatalog.gemma4E2B.id]
        // What each use runs on, as the library gives it: writing skills
        // was picked, and the rest are as they start.
        state.selection = [
            .assistant: .apple, .skills: ModelCatalog.gemma4E2B.id, .titles: .apple, .search: .builtIn,
        ]
        state.windows = [ModelCatalog.gemma4E2B.id: 16_384]
        state.tunings = [ModelCatalog.gemma4E2B.id: Self.measured]
        state.downloads = [
            ModelCatalog.gemma4E4B.id: .downloading(received: 1_200_000_000, total: 3_659_530_240),
            Self.pasted.id: .failed(.digestMismatch(file: "Qwen3-0.6B.litertlm")),
        ]
        return state
    }

    private func screen() -> some View {
        NavigationStack {
            ModelsView(store: Store(initialState: models) { EmptyReducer() })
        }
    }

    func testDisplayData() {
        let models = models
        assertSnapshot(
            of: (
                choices: models.choices.map(\.id), downloadable: models.downloadable.map(\.id),
                selected: ModelUse.allCases.map { ($0, models.selected(for: ModelUseKey($0))) }, canAdd: models.canAdd
            ),
            as: Snapshotting<Any, String>.dump.pullback { $0 }, named: "models")
    }

    func testAccessibilityTree() {
        assertSnapshot(of: screen(), as: .accessibilityTree(size: CGSize(width: 402, height: 874)), named: "models")
    }

    func testImages() {
        assertScreenImages(of: screen(), named: "models")
    }
}
