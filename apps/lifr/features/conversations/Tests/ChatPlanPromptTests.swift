import ComposableArchitecture
import Foundation
import LifrAPI
import LifrAssistantClient
import LifrConversations
import LifrCore
import LifrPasteboardClient
import LifrStoreClient
import XCTest

/// Planning with another chat assistant: /export copies a prompt, and
/// /paste puts the plan in its reply on the draft card.
@MainActor
final class ChatPlanPromptTests: XCTestCase {
    /// 2026-11-03 10:00 UTC, a Tuesday.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)

    private let reply = """
        Here's a plan for the move.

        ```toml
        # Lifr plan v1

        [[task]]
        title = "Book the van"
        in = "HOME"
        ```

        Want reminders too?
        """

    /// A chat over a HOME Avenue, with `clipboard` as the clipboard.
    private func chat(_ clipboard: LockIsolated<String?>) -> (TestStoreOf<ChatFeature>, Workspace) {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        _ = records.avenue("HOME", "Home")
        let workspace = records.workspace
        let store = TestStore(
            initialState: ChatFeature.State(
                conversation: Conversation(id: UUID(0), createdAt: now), workspace: workspace)
        ) {
            ChatFeature()
        } withDependencies: { [now] in
            $0.store = .inMemory(workspace, actor: .user(device: "iPhone", surface: .iPhone))
            $0.skills = .inMemory()
            $0.assistant.availability = { _ in .available }
            $0.assistant.prewarm = { _ in }
            $0.pasteboard = PasteboardClient {
                clipboard.setValue($0)
            } read: {
                clipboard.value
            }
            $0.date.now = now
            $0.timeZone = .gmt
            $0.uuid = .incrementing
        }
        store.exhaustivity = .off
        return (store, workspace)
    }

    func testExportTypedWithAGoalCopiesAPromptNamingTheAvenuesAndTheGoal() async throws {
        let clipboard = LockIsolated<String?>(nil)
        let (store, _) = chat(clipboard)

        await store.send(.core(.binding(.set(\.input, "/export Move flat"))))
        await store.receive(\.exportPlanTapped)
        await store.finish()

        let prompt = try XCTUnwrap(clipboard.value)
        XCTAssertTrue(prompt.contains("HOME: Home"))
        XCTAssertTrue(prompt.contains("Move flat"))
        XCTAssertEqual(store.state.input, "")
        XCTAssertNotNil(store.state.alert)
        XCTAssertTrue(store.state.conversation.messages.isEmpty, "nothing was sent")
    }

    func testExportFromTheMenuPlansWhatsTypedInTheField() async throws {
        let clipboard = LockIsolated<String?>(nil)
        let (store, _) = chat(clipboard)

        await store.send(.core(.binding(.set(\.input, "Move flat"))))
        await store.send(.commandChosen(.export))
        await store.receive(\.exportPlanTapped)
        await store.finish()

        XCTAssertTrue(try XCTUnwrap(clipboard.value).contains("Move flat"))
        XCTAssertEqual(store.state.input, "")
        XCTAssertTrue(store.state.conversation.messages.isEmpty, "nothing was sent")
    }

    func testPasteFromTheMenuPutsTheReplysPlanOnTheDraftCard() async throws {
        let (store, _) = chat(LockIsolated<String?>(reply))

        await store.send(.commandChosen(.paste))
        await store.receive(\.planPasted)

        XCTAssertEqual(store.state.conversation.draft?.tasks.map(\.title), ["Book the van"])
    }

    func testPastingAReplyPutsItsPlanOnTheDraftCardWithoutAddingIt() async throws {
        let clipboard = LockIsolated<String?>(reply)
        let (store, workspace) = chat(clipboard)

        await store.send(.core(.binding(.set(\.input, "/paste "))))
        await store.receive(\.pastePlanTapped)
        await store.receive(\.planPasted)

        XCTAssertEqual(store.state.conversation.draft?.tasks.map(\.title), ["Book the van"])
        XCTAssertNil(store.state.alert)
        XCTAssertEqual(store.state.workspace, workspace)
    }

    func testPastingAMalformedReplyChangesNothingAndCopiesAFixUpPrompt() async throws {
        let clipboard = LockIsolated<String?>("```toml\n[[task]]\ntitel = \"Book the van\"\n```")
        let (store, workspace) = chat(clipboard)

        await store.send(.pastePlanTapped)
        await store.receive(\.planPasted)
        XCTAssertNil(store.state.conversation.draft)
        XCTAssertEqual(store.state.workspace, workspace)
        XCTAssertNotNil(store.state.alert)
        let fixUp: String
        do {
            _ = try PlanPrompt.read(clipboard.value ?? "")
            return XCTFail("the reply read as a plan")
        } catch {
            fixUp = PlanPrompt.fixUp(clipboard.value ?? "", error: error)
        }

        await store.send(.alert(.presented(.copyFixUp(fixUp))))
        await store.finish()
        XCTAssertEqual(clipboard.value, fixUp)
        XCTAssertTrue(fixUp.contains("titel"))
    }

    func testPastingWithNothingOnTheClipboardSaysSo() async {
        let (store, _) = chat(LockIsolated(nil))

        await store.send(.pastePlanTapped)
        await store.receive(\.planPasted)

        XCTAssertNil(store.state.conversation.draft)
        XCTAssertNotNil(store.state.alert)
    }
}
