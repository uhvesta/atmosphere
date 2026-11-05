import ComposableArchitecture
import Foundation
import LifrAssistantClient
import LifrSettings
import XCTest

@MainActor
final class InstructionsFeatureTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_793_700_000)

    private func instructions(_ client: GuidanceClient) -> TestStoreOf<InstructionsFeature> {
        let store = TestStore(initialState: InstructionsFeature.State()) {
            InstructionsFeature()
        } withDependencies: { [now] in
            $0.guidance = client
            $0.date.now = now
            $0.uuid = .incrementing
        }
        store.exhaustivity = .off
        return store
    }

    func testSavingANewVersionUsesItAndKeepsItOnTheDevice() async {
        let client = GuidanceClient.inMemory()
        let store = instructions(client)
        await store.send(.task)
        await store.receive(\.loaded.success)
        XCTAssertEqual(store.state.draft, Guidance.defaultText)
        XCTAssertFalse(store.state.canSave)

        await store.send(.binding(.set(\.draft, "Be brief.")))
        await store.send(.saveTapped)
        await store.receive(\.saved.success)

        XCTAssertEqual(store.state.guidance.text, "Be brief.")
        let kept = try? await client.load()
        XCTAssertEqual(kept?.text, "Be brief.")
        XCTAssertEqual(kept?.history.count, 1)
    }

    func testAPastVersionOrTheDefaultCanBeUsedAgain() async throws {
        var saved = Guidance()
        let first = UUID()
        try saved.save("Be brief.", id: first, at: now)
        try saved.save("Be cheerful.", id: UUID(), at: now)
        let client = GuidanceClient.inMemory(saved)
        let store = instructions(client)
        await store.send(.task)
        await store.receive(\.loaded.success)

        await store.send(.useTapped(first))
        await store.receive(\.saved.success)
        XCTAssertEqual(store.state.draft, "Be brief.")
        let used = try await client.load()
        XCTAssertEqual(used.text, "Be brief.")

        await store.send(.useTapped(nil))
        await store.receive(\.saved.success)
        XCTAssertEqual(store.state.draft, Guidance.defaultText)
        let reverted = try await client.load()
        XCTAssertTrue(reverted.isDefault)
        XCTAssertEqual(reverted.history.count, 2)
    }

    func testTooLongInstructionsCantBeSaved() async {
        let store = instructions(.inMemory())
        await store.send(.task)
        await store.receive(\.loaded.success)
        await store.send(.binding(.set(\.draft, String(repeating: "a", count: Guidance.limit + 1))))
        XCTAssertTrue(store.state.isTooLong)
        XCTAssertFalse(store.state.canSave)
    }

    func testAFailedSaveKeepsWhatWasInUseAndSaysWhy() async {
        var client = GuidanceClient.inMemory()
        client.save = { _ in throw CocoaError(.fileWriteOutOfSpace) }
        let store = instructions(client)
        await store.send(.task)
        await store.receive(\.loaded.success)
        await store.send(.binding(.set(\.draft, "Be brief.")))
        await store.send(.saveTapped)
        await store.receive(\.saved.failure)

        XCTAssertTrue(store.state.guidance.isDefault)
        XCTAssertEqual(store.state.draft, "Be brief.")
        guard case .storageFailed = store.state.error else {
            return XCTFail("A failed save should say storage failed")
        }
    }

    func testAnUnreadableFileIsShownAndNothingCanBeSaved() async {
        let store = instructions(GuidanceClient(load: { throw GuidanceError.storageFailed("damaged") }, save: { _ in }))
        await store.send(.task)
        await store.receive(\.loaded.failure)
        XCTAssertEqual(store.state.error, .storageFailed("damaged"))
        await store.send(.binding(.set(\.draft, "Be brief.")))
        XCTAssertFalse(store.state.canSave)
    }
}
