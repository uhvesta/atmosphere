import ComposableArchitecture
import HumataDaemonClient
import HumataSession
import XCTest

/// The box's chips, what ⌘V brings into it, and when a turn can be stopped.
@MainActor
final class ComposerTests: XCTestCase {
    /// A store for session `id`, which also names where its draft is kept.
    private func store(
        _ id: String, _ dependencies: @escaping (inout DependencyValues) -> Void = { _ in }
    ) -> TestStoreOf<SessionFeature> {
        let session = Session(id: id, name: "first", backend: .scripted, workingDirectory: "/work")
        let store = TestStore(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            $0.daemon.markSeen = { _, _ in }
            $0.uuid = .incrementing
            dependencies(&$0)
        }
        store.exhaustivity = .off
        return store
    }

    private func event(_ id: String, _ kind: SessionEvent.Kind) -> SessionEvent {
        SessionEvent(id: id, time: Date(timeIntervalSince1970: 0), kind: kind)
    }

    /// The message the session handed on to be sent, once it has.
    private func handedOn(_ store: TestStoreOf<SessionFeature>) async -> SessionFeature.Outgoing? {
        var message: SessionFeature.Outgoing?
        await store.receive({ action in
            guard case .delegate(.send(let outgoing)) = action else { return false }
            message = outgoing
            return true
        })
        return message
    }

    private let log = (1...30).map { "line \($0)" }.joined(separator: "\n")

    func testAnEditedChipIsSentAsItWasChanged() async {
        let store = store("composer-edited-chip")
        await store.send(\.binding.draft, "why does this fail? ")
        await store.send(\.binding.draft, "why does this fail? " + log)
        let id = store.state.pastes.first?.id ?? ""

        await store.send(.pasteEdited(id: id, text: "only the last line\nline 30"))
        XCTAssertEqual(store.state.pastes.map(\.id), [id])
        XCTAssertEqual(store.state.pastes.first?.lineCount, 2)

        await store.send(.sendTapped(.enqueue))
        let message = await handedOn(store)
        XCTAssertEqual(message?.text, "why does this fail?\n\nonly the last line\nline 30")
    }

    func testAChipEditedToNothingIsTakenOut() async {
        let store = store("composer-emptied-chip")
        await store.send(\.binding.draft, log)
        let id = store.state.pastes.first?.id ?? ""

        await store.send(.pasteEdited(id: id, text: " \n"))
        XCTAssertEqual(store.state.pastes, [])
        XCTAssertFalse(store.state.canSend)
    }

    func testAChipPutInTheMessageGoesAfterWhatsTypedAndStaysThere() async {
        let store = store("composer-chip-to-box")
        await store.send(\.binding.draft, "see this")
        await store.send(\.binding.draft, "see this" + log)
        let id = store.state.pastes.first?.id ?? ""

        await store.send(.pasteMovedToBox(id: id, text: log))
        XCTAssertEqual(store.state.pastes, [])
        XCTAssertEqual(store.state.draft, "see this\n\n" + log)

        // Typing on doesn't turn it back into a chip.
        await store.send(\.binding.draft, "see this\n\n" + log + "!")
        XCTAssertEqual(store.state.pastes, [])
        XCTAssertEqual(store.state.draft, "see this\n\n" + log + "!")
    }

    func testAChipTooLongForTheFieldStaysAChip() async {
        let store = store("composer-long-chip")
        let long = String(repeating: "x", count: 50_000)
        await store.send(\.binding.draft, long)
        let id = store.state.pastes.first?.id ?? ""
        XCTAssertFalse(Paste.fitsInBox(long))

        await store.send(.pasteMovedToBox(id: id, text: long))
        XCTAssertEqual(store.state.pastes.map(\.text), [long])
        XCTAssertEqual(store.state.draft, "")
    }

    func testPasteBringsAnImageOnlyWhenTheFieldWouldPasteNothing() {
        let shot = Data([1, 2, 3])
        let file = URL(fileURLWithPath: "/tmp/shot.png")
        XCTAssertEqual(ClipboardImage.pasted(imageFiles: [], hasText: false) { shot }, .data(shot))
        XCTAssertNil(ClipboardImage.pasted(imageFiles: [], hasText: true) { shot }, "text is the field's to paste")
        XCTAssertNil(ClipboardImage.pasted(imageFiles: [], hasText: false) { nil })
        // A file copied in the Finder comes with its name as text.
        XCTAssertEqual(ClipboardImage.pasted(imageFiles: [file], hasText: true) { nil }, .files([file]))
    }

    func testATurnCanBeStoppedOnlyUntilItIsToldTo() async {
        let store = store("composer-stop") { $0.daemon.abort = { _ in } }
        XCTAssertFalse(store.state.canStop, "there's no turn to stop")
        await store.send(.received(event("e1", .userMessage("write the notes"))))
        await store.send(.received(event("e2", .turnStarted)))
        XCTAssertTrue(store.state.canStop)

        await store.send(.abortTapped)
        XCTAssertFalse(store.state.canStop, "it's stopping")

        await store.send(.received(event("e3", .turnEnded(aborted: true))))
        XCTAssertFalse(store.state.canStop)
        await store.send(.received(event("e4", .userMessage("again"))))
        await store.send(.received(event("e5", .turnStarted)))
        XCTAssertTrue(store.state.canStop)
        await store.finish()
    }

    func testSteerNowGivesUpItsKeyWhileAnEarlierMessageIsEdited() async {
        let store = store("composer-steer-key")
        await store.send(.received(event("e1", .userMessage("write the notes"))))
        await store.send(.received(event("e2", .turnStarted)))
        XCTAssertTrue(store.state.steerTakesShortcut)

        await store.send(.editTapped("e1"))
        XCTAssertFalse(store.state.steerTakesShortcut, "⌘Return is the editor's")

        await store.send(.editCancelled)
        XCTAssertTrue(store.state.steerTakesShortcut)
        await store.finish()
    }
}
