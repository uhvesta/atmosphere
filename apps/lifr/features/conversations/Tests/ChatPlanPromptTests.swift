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

    /// A chat over a HOME Avenue, with `clipboard` as the clipboard,
    /// replying when `replying`.
    private func chat(
        _ clipboard: LockIsolated<String?>, _ workspace: Workspace? = nil, replying: Bool = false
    ) -> (TestStoreOf<ChatFeature>, Workspace) {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        _ = records.avenue("HOME", "Home")
        let workspace = workspace ?? records.workspace
        var state = ChatFeature.State(conversation: Conversation(id: UUID(0), createdAt: now), workspace: workspace)
        state.core.isReplying = replying
        let store = TestStore(initialState: state) {
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
            $0.withRandomNumberGenerator = WithRandomNumberGenerator(SystemRandomNumberGenerator())
        }
        store.exhaustivity = .off
        return (store, workspace)
    }

    func testExportTypedWithAGoalShowsAPromptNamingTheAvenuesAndTheGoalAndCopiesIt() async throws {
        let clipboard = LockIsolated<String?>(nil)
        let (store, _) = chat(clipboard)

        await store.send(.core(.binding(.set(\.input, "/export Move flat"))))
        await store.receive(\.exportPlanTapped)
        XCTAssertNil(clipboard.value, "nothing is copied before Copy")
        let prompt = try XCTUnwrap(store.state.export?.prompt)
        XCTAssertTrue(prompt.contains("HOME: Home"))
        XCTAssertTrue(prompt.contains("Move flat"))

        await store.send(.export(.presented(.copyTapped)))
        await store.finish()
        XCTAssertEqual(clipboard.value, prompt)
        XCTAssertEqual(store.state.export?.copied, true)
        XCTAssertEqual(store.state.input, "")
        XCTAssertTrue(store.state.conversation.messages.isEmpty, "nothing was sent")
    }

    func testExportFromTheMenuPlansWhatsTypedInTheField() async throws {
        let (store, _) = chat(LockIsolated<String?>(nil))

        await store.send(.core(.binding(.set(\.input, "Move flat"))))
        await store.send(.commandChosen(.export))
        await store.receive(\.exportPlanTapped)

        XCTAssertTrue(try XCTUnwrap(store.state.export?.prompt).contains("Move flat"))
        XCTAssertEqual(store.state.input, "")
        XCTAssertTrue(store.state.conversation.messages.isEmpty, "nothing was sent")
    }

    func testEditingTheGoalRemakesThePromptToCopyAgain() async throws {
        let clipboard = LockIsolated<String?>(nil)
        let (store, _) = chat(clipboard)
        await store.send(.exportPlanTapped(goal: "Move flat"))
        await store.send(.export(.presented(.copyTapped)))
        await store.send(.export(.presented(.binding(.set(\.goal, "Paint the hall")))))

        XCTAssertEqual(store.state.export?.copied, false)
        XCTAssertTrue(try XCTUnwrap(store.state.export?.prompt).contains("Paint the hall"))
    }

    func testExportWithOneOfThePersonsSkillsPickedCarriesItsInstructions() async throws {
        let (store, _) = chat(LockIsolated<String?>(nil))
        let errands = try XCTUnwrap(
            Skill(
                userNamed: "errands", summary: "Plan errands.", instructions: "Every task needs a due date.",
                tools: [], draftsPlans: true))
        await store.send(.skillsLoaded(Skill.builtIn + [errands]))
        await store.send(.pickSkillTapped(errands.id))

        await store.send(.exportPlanTapped(goal: "Saturday"))
        XCTAssertTrue(try XCTUnwrap(store.state.export?.prompt).contains("Every task needs a due date."))

        let builtIn = try XCTUnwrap(Skill.builtIn.first { $0.isOn })
        await store.send(.pickSkillTapped(builtIn.id))
        await store.send(.exportPlanTapped(goal: "Saturday"))
        XCTAssertFalse(try XCTUnwrap(store.state.export?.prompt).contains(builtIn.instructions))
    }

    func testExportListsTheTasksInThePickedScopeAndClosedOnesOnlyWhenAsked() async throws {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        let home = records.avenue("HOME", "Home")
        _ = records.task("Fix the gate", in: home, due: 0)
        let keys = records.task("Return the keys", in: home, due: 0)
        records.setStatus(.done, of: keys)
        let (store, _) = chat(LockIsolated<String?>(nil), records.workspace)

        await store.send(.exportPlanTapped(goal: "Move flat"))
        XCTAssertFalse(try XCTUnwrap(store.state.export?.prompt).contains("Fix the gate"))

        await store.send(.export(.presented(.binding(.set(\.listed, .avenue(home.id))))))
        var prompt = try XCTUnwrap(store.state.export?.prompt)
        XCTAssertTrue(prompt.contains("HOME: Fix the gate"))
        XCTAssertFalse(prompt.contains("Return the keys"))

        await store.send(.export(.presented(.binding(.set(\.openOnly, false)))))
        prompt = try XCTUnwrap(store.state.export?.prompt)
        XCTAssertTrue(prompt.contains("Fix the gate"))
        XCTAssertTrue(prompt.contains("Return the keys"))
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

    func testAReplyNamingAnAvenueCutShortPutsItsTasksInThatAvenue() async throws {
        let (store, _) = chat(LockIsolated<String?>(reply.replacingOccurrences(of: "\"HOME\"", with: "\"Hom\"")))

        await store.send(.pastePlanTapped)
        await store.receive(\.planPasted)

        XCTAssertEqual(store.state.conversation.draft?.tasks.map(\.place), ["HOME"])
        XCTAssertNil(store.state.problem)
    }

    func testApplyingAPastedPlanThenUndoingLeavesTheTasksAsTheyWere() async throws {
        let (store, _) = chat(LockIsolated<String?>(reply))
        let open = { store.state.workspace.tasks.values.filter { $0.deletedAt == nil }.map(\.title).sorted() }
        let before = open()

        await store.send(.pastePlanTapped)
        await store.receive(\.planPasted)
        await store.send(.addTapped)
        await store.skipReceivedActions()
        XCTAssertEqual(open(), (before + ["Book the van"]).sorted())

        await store.send(.undoAddTapped)
        await store.skipReceivedActions()
        XCTAssertEqual(open(), before)
        XCTAssertEqual(store.state.conversation.draft?.tasks.map(\.title), ["Book the van"])
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

    func testPastingOverADraftAsksBeforeReplacingIt() async throws {
        let clipboard = LockIsolated<String?>(reply)
        let (store, _) = chat(clipboard)
        await store.send(.pastePlanTapped)
        await store.receive(\.planPasted)
        let first = try XCTUnwrap(store.state.conversation.draft)

        clipboard.setValue(reply.replacingOccurrences(of: "Book the van", with: "Pack the books"))
        await store.send(.pastePlanTapped)
        await store.receive(\.planPasted)
        XCTAssertEqual(store.state.conversation.draft, first, "the draft stays until Replace")
        XCTAssertNotNil(store.state.alert)

        let plan = try XCTUnwrap(PlanPrompt.read(clipboard.value ?? ""))
        await store.send(.alert(.presented(.replaceDraft(plan.pinned(today: CalendarDate(now, in: .gmt))))))
        XCTAssertEqual(store.state.conversation.draft?.tasks.map(\.title), ["Pack the books"])
    }

    func testPasteDoesNothingWhileAReplyIsWritten() async {
        let (store, _) = chat(LockIsolated<String?>(reply), replying: true)

        await store.send(.commandChosen(.paste))
        await store.send(.pastePlanTapped)

        XCTAssertNil(store.state.conversation.draft)
    }
}
