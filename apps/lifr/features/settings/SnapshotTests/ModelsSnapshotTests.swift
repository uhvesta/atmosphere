import ComposableArchitecture
import LifrModelsClient
import LifrSettings
import LifrSnapshotTesting
import SnapshotTesting
import SwiftUI
import XCTest

/// Models with Gemma 4 E2B downloaded at a 16K window and picked for
/// writing skills, E4B downloading, and a model pasted from a link whose
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

    private var models: ModelsFeature.State {
        var state = ModelsFeature.State()
        state.models = ModelCatalog.all + [Self.pasted]
        state.installed = [ModelCatalog.gemma4E2B.id]
        state.selection = [.skills: ModelCatalog.gemma4E2B.id]
        state.windows = [ModelCatalog.gemma4E2B.id: 16_384]
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
                selected: ModelUse.allCases.map { ($0, models.selected(for: $0)) }, canAdd: models.canAdd
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
