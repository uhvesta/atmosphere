import CaptureKit
import ComposableArchitecture
import Foundation
import InferenceKit
import LangReadrCore
import XCTest

@testable import LangReadrReading

@MainActor
final class PassageChatTests: XCTestCase {
    private func chat(
        _ runtime: ScriptedRuntime = ScriptedRuntime(steps: [ScriptedRuntime.answer("It means friend.")]),
        dictation: DictationClient = .scripted(), files: [String: PickedFile] = [:]
    ) throws -> TestStoreOf<PassageChatFeature> {
        let book = try spanish("Hola amigo.")
        let passage = try Passage(in: try book.paragraphs.first.unwrap(), from: 5, to: 10).unwrap()
        let anchored = AnchoredChat(passage: passage, createdAt: epoch)
        let store = TestStore(
            initialState: PassageChatFeature.State(chat: anchored, book: ChatBook(book, chat: anchored), compact: false)
        ) {
            PassageChatFeature()
        } withDependencies: {
            $0.tutor = Tutor(runtime: runtime, reader: Language("en"))
            $0.date.now = epoch
            $0.uuid = .incrementing
            $0.dictation = dictation
            $0.fileReader = .scripted(files)
        }
        store.exhaustivity = .off
        return store
    }

    private func notes(_ text: String = "Amigo: friend. Hola: hello.") throws -> PickedFile {
        .text(try FileText(name: "notes.md", text: text, limit: 8_000))
    }

    func testDictationWritesAfterWhatsTypedUntilItsStopped() async throws {
        let store = try chat()
        await store.send(.binding(.set(\.input, "Please:")))
        await store.send(.micTapped)
        await store.receive(\.dictated)
        await store.receive(\.dictated)
        XCTAssertEqual(store.state.input, "Please: What does this word mean?")
        XCTAssertTrue(store.state.dictating)
        XCTAssertFalse(store.state.canSend, "Send waits for the last words")

        await store.send(.micTapped)
        await store.receive(\.dictationEnded)
        XCTAssertFalse(store.state.dictating)
        XCTAssertTrue(store.state.canSend)
    }

    func testDictationThatCantStartSaysWhy() async throws {
        let store = try chat(dictation: .failing(.speechDenied))
        await store.send(.micTapped)
        await store.receive(\.dictationFailed)
        XCTAssertFalse(store.state.dictating)
        XCTAssertNotNil(store.state.problem)
        XCTAssertEqual(store.state.input, "")
    }

    func testPlusOpensAndPickingFilesClosesIt() async throws {
        let store = try chat()
        await store.send(.addContextTapped)
        XCTAssertTrue(store.state.addsContext)
        await store.send(.filesPicked(.success([])))
        XCTAssertFalse(store.state.addsContext)
    }

    func testAFilesTextGoesWithTheMessageAndToTheModel() async throws {
        let runtime = ScriptedRuntime(steps: [ScriptedRuntime.answer("It means friend.")])
        let store = try chat(runtime, files: ["notes.md": try notes()])
        await store.send(.filesPicked(.success([URL(filePath: "/tmp/notes.md")])))
        await store.receive(\.filesRead)
        XCTAssertEqual(store.state.pending.map(\.name), ["notes.md"])
        XCTAssertTrue(store.state.canSend, "A file can go on its own")

        await store.send(.sendTapped)
        await store.receive(\.replied)
        await store.settle()
        XCTAssertEqual(store.state.pending, [])
        let asked = try store.state.chat.conversation.messages.first.unwrap()
        XCTAssertEqual(asked.files.map(\.name), ["notes.md"])
        XCTAssertTrue(try runtime.turns.first.unwrap().prompt.contains("Amigo: friend."))
    }

    func testARemovedFileIsntSent() async throws {
        let store = try chat(files: ["notes.md": try notes()])
        await store.send(.filesPicked(.success([URL(filePath: "/tmp/notes.md")])))
        await store.receive(\.filesRead)
        await store.send(.removeFileTapped(0))
        XCTAssertEqual(store.state.pending, [])
        XCTAssertFalse(store.state.canSend)
    }

    func testAPhotoOrAnUnreadableFileIsntAttachedAndSaysWhy() async throws {
        let store = try chat(files: ["fern.jpg": .photo(Data([1])), "notes.md": try notes()])
        let picked = ["/tmp/fern.jpg", "/tmp/missing.txt", "/tmp/notes.md"].map { URL(filePath: $0) }
        await store.send(.filesPicked(.success(picked)))
        await store.receive(\.filesRead)
        XCTAssertEqual(store.state.pending.map(\.name), ["notes.md"])
        XCTAssertNotNil(store.state.problem)
    }

    func testAMessageKeptBeforeFilesReadsAsHavingNone() throws {
        let saved = Data(
            #"{"id":"6F9619FF-8B86-D011-B42D-00C04FC964FF","role":"person","text":"Hola","at":0}"#.utf8)
        let message = try JSONDecoder().decode(ReaderMessage.self, from: saved)
        XCTAssertEqual(message.text, "Hola")
        XCTAssertEqual(message.files, [])
        let withFile = ReaderMessage(
            role: .person, text: "", at: epoch, files: [ReaderFile(name: "notes.md", text: "Amigo")])
        XCTAssertEqual(try JSONDecoder().decode(ReaderMessage.self, from: JSONEncoder().encode(withFile)), withFile)
    }

    func testAFilesTextIsCutToFit() {
        let long = ReaderMessage(
            role: .person, text: "", at: epoch,
            files: [ReaderFile(name: "long.txt", text: String(repeating: "amigo ", count: 1_000))])
        XCTAssertLessThan(Tutor.files(of: long, fitting: 500).count, 600)
        XCTAssertEqual(Tutor.files(of: ReaderMessage(role: .person, text: "Hola", at: epoch), fitting: 500), "")
    }
}
