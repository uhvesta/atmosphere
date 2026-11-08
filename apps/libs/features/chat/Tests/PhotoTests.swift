import CaptureKit
import ChatKit
import ComposableArchitecture
import Foundation
import ImageIO
import InferenceKit
import ModelsClient
import XCTest

@testable import ChatFeatures

@MainActor
final class PhotoTests: XCTestCase {
    private let gemma = ModelCatalog.gemma4E4B

    /// A chat on a model that reads photos, with `recents` in the library.
    private func seeing(
        _ runs: Runs, recents: [RecentPhoto] = [], kept: ChatPhotosClient = .inMemory()
    ) async -> TestStoreOf<ChatThreadFeature> {
        let store = chat(
            runs, model: ModelChoice(model: gemma.id), library: InMemoryModelLibrary(installed: [gemma]),
            photos: .scripted(recents, camera: true), kept: kept)
        await store.send(.task)
        await store.receive(\.modelsLoaded)
        return store
    }

    func testPlusOffersPhotosForAModelThatReadsThem() async throws {
        let recent = RecentPhoto(id: "fern", thumbnail: try photo())
        let store = await seeing(Runs([], inputs: [.image]), recents: [recent])
        await store.send(.addContextTapped)
        await store.receive(\.photosLoaded)
        XCTAssertTrue(store.state.addsContext)
        XCTAssertFalse(store.state.picksFiles)
        XCTAssertEqual(store.state.recentPhotos, [recent])
        XCTAssertTrue(store.state.hasCamera)
    }

    func testPlusOpensTheFilePickerForAModelThatReadsOnlyText() async {
        let store = chat(Runs([]), photos: .scripted([]))
        await store.send(.task)
        await store.receive(\.modelsLoaded)
        await store.send(.addContextTapped)
        XCTAssertTrue(store.state.picksFiles)
        XCTAssertFalse(store.state.addsContext)
    }

    func testATakenPhotoGoesWithTheMessageAndTheModelSeesIt() async throws {
        let runs = Runs([ScriptedRuntime.answer(.string("A fern."))], inputs: [.image])
        let store = await seeing(runs)
        await store.send(.photoTaken(try photo()))
        await store.receive(\.photoAttached)
        XCTAssertEqual(store.state.pendingPhotos.count, 1)
        XCTAssertTrue(store.state.canSend, "A photo can go on its own")

        await store.ask("What's this?")
        let sent = try store.state.thread.messages.first.unwrap()
        XCTAssertEqual(sent.photos.count, 1)
        XCTAssertEqual(store.state.pendingPhotos, [])
        let turn = try runs.runtime.turns.first.unwrap()
        XCTAssertEqual(turn.attachments.map(\.kind), [.image])
        let file = try turn.attachments.first.unwrap().file
        XCTAssertNotNil(CGImageSourceCreateWithURL(file as CFURL, nil), "The model gets the photo itself")
    }

    func testTheModelStillSeesAPhotoWhenAskedAboutItAgain() async throws {
        let runs = Runs(
            [ScriptedRuntime.answer(.string("A fern.")), ScriptedRuntime.answer(.string("Green."))], inputs: [.image])
        let store = await seeing(runs)
        await store.send(.photoTaken(try photo()))
        await store.receive(\.photoAttached)
        await store.ask("What's this?")
        await store.ask("What color is it?")
        XCTAssertEqual(runs.runtime.turns.last?.attachments.count, 1)
    }

    func testARecentPhotoTappedAgainIsTakenOff() async throws {
        let recent = RecentPhoto(id: "fern", thumbnail: try photo())
        let store = await seeing(Runs([], inputs: [.image]), recents: [recent])
        await store.send(.recentPhotoTapped("fern"))
        await store.receive(\.photoAttached)
        XCTAssertEqual(store.state.pendingPhotos.count, 1)
        XCTAssertNotNil(store.state.attachedRecents["fern"])

        await store.send(.recentPhotoTapped("fern"))
        XCTAssertEqual(store.state.pendingPhotos, [])
        XCTAssertTrue(store.state.attachedRecents.isEmpty)
        XCTAssertFalse(store.state.canSend)
    }

    func testAPhotoPickedAsAFileIsAttachedForAModelThatReadsPhotos() async throws {
        let store = chat(
            Runs([], inputs: [.image]), model: ModelChoice(model: gemma.id),
            library: InMemoryModelLibrary(installed: [gemma]), files: ["fern.png": .photo(try photo())])
        await store.send(.task)
        await store.receive(\.modelsLoaded)
        await store.send(.filesPicked(.success([URL(filePath: "/tmp/fern.png")])))
        await store.receive(\.filesRead)
        XCTAssertEqual(store.state.pendingPhotos.count, 1)
        XCTAssertNil(store.state.problem)
    }

    func testAnUnreadablePhotoIsntAttachedAndSaysWhy() async throws {
        let store = await seeing(Runs([], inputs: [.image]))
        await store.send(.photoTaken(Data([1, 2, 3])))
        await store.receive(\.photoFailed)
        XCTAssertEqual(store.state.pendingPhotos, [])
        XCTAssertNotNil(store.state.problem)
        XCTAssertFalse(store.state.attaching)
    }

    func testAModelThatReadsOnlyTextIsToldOfAPhotoButNotSentIt() async throws {
        let runs = Runs([ScriptedRuntime.answer(.string("I can't see it."))])
        var thread = ChatThread(id: UUID(999), createdAt: epoch)
        thread.append(ThreadMessage(role: .person, text: "What's this?", at: epoch, photos: [ThreadPhoto(id: UUID())]))
        let store = chat(runs, thread: thread)
        await store.send(.retryTapped(messageID: try thread.messages.last.unwrap().id))
        await store.receive(\.replied)
        await store.settle()
        let turn = try runs.runtime.turns.first.unwrap()
        XCTAssertEqual(turn.attachments, [])
        XCTAssertTrue(turn.prompt.contains("can't see"))
    }

    func testAKeptPhotoIsAJPEGWithoutItsMetadataAndGoesWithItsChat() throws {
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: folder) }
        let files = ChatPhotoFiles(root: folder)
        let chat = UUID()
        let kept = try files.save(try photo(side: 2_000), chat: chat)
        let file = try files.file(kept, chat: chat).unwrap()
        let source = try CGImageSourceCreateWithURL(file as CFURL, nil).unwrap()
        XCTAssertEqual(CGImageSourceGetType(source) as String?, "public.jpeg")
        let properties = try (CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any]).unwrap()
        XCTAssertLessThanOrEqual(properties[kCGImagePropertyPixelWidth] as? Int ?? .max, ChatPhotoFiles.longestSide)
        XCTAssertNil(properties[kCGImagePropertyGPSDictionary])

        try files.removeAll(chat)
        XCTAssertNil(files.file(kept, chat: chat))
    }
}
