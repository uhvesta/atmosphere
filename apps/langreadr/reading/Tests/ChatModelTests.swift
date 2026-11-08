import ComposableArchitecture
import Foundation
import InferenceKit
import LangReadrCore
import LangReadrModels
import LangReadrServices
import LangReadrServicesFake
import XCTest

@testable import LangReadrReading

/// Records the use and choice each model is asked for, and runs them all
/// on one scripted model.
private final class Runs: Sendable {
    let runtime: ScriptedRuntime
    let asked = LockIsolated<[ModelUse]>([])
    let choices = LockIsolated<[ModelChoice]>([])

    init(_ steps: [ScriptedRuntime.Step]) {
        runtime = ScriptedRuntime(steps: steps)
    }

    var tutor: Tutor {
        Tutor(
            runtime: { [self] use, choice in
                asked.withValue { $0.append(use) }
                choices.withValue { $0.append(choice) }
                return runtime
            }, reader: Language("en"))
    }
}

@MainActor
final class ChatModelTests: XCTestCase {
    private let qwen = ModelCatalog.qwen35Small.id

    func testTranslatingRunsOnTheReadingModelWhateverAChatPicked() async throws {
        let runs = Runs([ScriptedRuntime.answer("Hello.")])
        _ = try await runs.tutor.translate("Hola.", from: Language("es"))
        XCTAssertEqual(runs.asked.value, [.reading])
        XCTAssertEqual(runs.choices.value, [.defaults])
    }

    func testTheLibraryOpensSettingsWithItsModels() async throws {
        let store = TestStore(initialState: LibraryFeature.State(books: [])) {
            LibraryFeature()
        } withDependencies: {
            $0.shelf = .inMemory()
            $0.readerServices = .fake()
            $0.tutor = tutor([])
            $0.models = InMemoryModelLibrary(installed: [ModelCatalog.qwen35Small]).client
        }
        store.exhaustivity = .off
        await store.send(.settingsTapped)
        await store.send(.settings(.presented(.models(.task))))
        await store.receive(\.settings.presented.models.loaded)
        XCTAssertEqual(store.state.settings?.models.choices.map(\.id), [.apple, qwen])
        await store.send(.settings(.presented(.doneTapped)))
        await store.receive(\.settings.dismiss)
        XCTAssertNil(store.state.settings)
    }
}
