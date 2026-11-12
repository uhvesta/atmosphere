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
        }
        store.exhaustivity = .off
        return (store, workspace)
    }

    func testExportTypedWithAGoalCopiesAPromptNamingTheAvenuesAndTheGoal() async throws {
        let clipboard = LockIsolated<String?>(nil)
        let (store, _) = chat(clipboard)

        await store.send(.core(.binding(.set(\.input, "/export Move flat"))))
        await store.receive(\.exportPlanTapped)
        XCTAssertNil(clipboard.value, "nothing is copied before the scope is picked")
        await store.send(.alert(.presented(.exportPlan(goal: "Move flat", scope: .names))))
        await store.receive(\.exportPlanScoped)
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
        await store.send(.alert(.presented(.exportPlan(goal: "Move flat", scope: .names))))
        await store.receive(\.exportPlanScoped)
        await store.finish()

        XCTAssertTrue(try XCTUnwrap(clipboard.value).contains("Move flat"))
        XCTAssertEqual(store.state.input, "")
        XCTAssertTrue(store.state.conversation.messages.isEmpty, "nothing was sent")
    }

    func testExportWithOneOfThePersonsSkillsPickedCarriesItsInstructions() async throws {
        let clipboard = LockIsolated<String?>(nil)
        let (store, _) = chat(clipboard)
        let errands = try XCTUnwrap(
            Skill(
                userNamed: "errands", summary: "Plan errands.", instructions: "Every task needs a due date.",
                tools: [], draftsPlans: true))
        await store.send(.skillsLoaded(Skill.builtIn + [errands]))
        await store.send(.pickSkillTapped(errands.id))

        await store.send(.exportPlanTapped(goal: "Saturday"))
        await store.send(.alert(.presented(.exportPlan(goal: "Saturday", scope: .names))))
        await store.receive(\.exportPlanScoped)
        await store.finish()
        XCTAssertTrue(try XCTUnwrap(clipboard.value).contains("Every task needs a due date."))

        let builtIn = try XCTUnwrap(Skill.builtIn.first { $0.isOn })
        await store.send(.pickSkillTapped(builtIn.id))
        await store.send(.exportPlanTapped(goal: "Saturday"))
        await store.send(.alert(.presented(.exportPlan(goal: "Saturday", scope: .names))))
        await store.receive(\.exportPlanScoped)
        await store.finish()
        XCTAssertFalse(try XCTUnwrap(clipboard.value).contains(builtIn.instructions))
    }

    func testExportListsAnAvenuesOpenTasksOnlyWhenThatScopeIsPicked() async throws {
        let clipboard = LockIsolated<String?>(nil)
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        let home = records.avenue("HOME", "Home")
        _ = records.task("Fix the gate", in: home, due: 0)
        let (store, _) = chat(clipboard, records.workspace)

        await store.send(.core(.binding(.set(\.input, "/export Move flat"))))
        await store.receive(\.exportPlanTapped)
        await store.send(.alert(.presented(.exportPlan(goal: "Move flat", scope: .names))))
        await store.receive(\.exportPlanScoped)
        await store.finish()
        XCTAssertFalse(try XCTUnwrap(clipboard.value).contains("Fix the gate"))

        await store.send(.exportPlanTapped(goal: "Move flat"))
        await store.send(.alert(.presented(.exportPlan(goal: "Move flat", scope: .avenue(home.id)))))
        await store.receive(\.exportPlanScoped)
        await store.finish()
        XCTAssertTrue(try XCTUnwrap(clipboard.value).contains("HOME: Fix the gate"))
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
