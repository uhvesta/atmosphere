import Foundation
import LifrAPI
import LifrAssistantClient
import LifrCore
import LifrStoreClient
import XCTest

/// What Lifr makes of a small model's answers to follow-ups and requests
/// to add or change tasks.
final class FollowUpTests: XCTestCase {
    /// 2026-11-03 10:00 UTC, a Tuesday.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)

    private func request(_ texts: String..., draft: Plan? = nil, problem: String? = nil) -> AssistantRequest {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        let home = records.avenue("HOME", "Home")
        records.task("Call the plumber", in: home)
        return AssistantRequest(
            messages: texts.map { ChatMessage(id: UUID(), role: .person, text: $0, at: now) }, draft: draft,
            problem: problem, workspace: records.workspace, now: now, timeZone: .gmt,
            writes: AssistantWrites(conversation: UUID()))
    }

    private let milk = Plan(tasks: [.init(title: "buy milk", place: "HOME")])

    // MARK: - Changing the draft

    func testAnAnswerAddingToTheDraftKeepsItAndAddsOnlyWhatsNew() throws {
        let added = Plan(tasks: [.init(title: "Buy milk", place: "HOME"), .init(title: "buy eggs", place: "HOME")])

        let plan = try XCTUnwrap(AssistantPrompt.followUp(added, changes: [], to: milk))

        XCTAssertEqual(plan.tasks.map(\.title), ["buy milk", "buy eggs"])
    }

    func testChangesToTheDraftRescheduleRenameMoveAndRemoveItsTasks() throws {
        let draft = Plan(
            projects: [.init(name: "Shop", avenue: "HOME", tasks: [.init(title: "list", due: "today")])],
            tasks: [.init(title: "buy milk", place: "HOME"), .init(title: "buy eggs", due: "friday", place: "HOME")])

        let plan = try XCTUnwrap(
            AssistantPrompt.followUp(
                nil,
                changes: [
                    DraftChange(task: "Buy Milk", rename: "buy oat milk", priority: .high, moveTo: "WORK"),
                    DraftChange(task: "buy eggs", due: "none"),
                    DraftChange(task: "list", remove: true),
                ], to: draft))

        XCTAssertEqual(plan.tasks.map(\.title), ["buy oat milk", "buy eggs"])
        XCTAssertEqual(plan.tasks.map(\.place), ["WORK", "HOME"])
        XCTAssertEqual(plan.tasks.map(\.priority), [.high, .none])
        XCTAssertEqual(plan.tasks.map(\.due), [nil, nil])
        XCTAssertEqual(plan.projects.first?.tasks, [])
    }

    func testAnAnswerWithNothingNewOrChangedLeavesTheDraft() {
        XCTAssertNil(AssistantPrompt.followUp(nil, changes: [], to: milk))
    }

    func testDraftChangesNameTasksByNumberOrTitle() {
        let answer: JSONValue = [
            "draftChanges": [
                ["task": "2", "due": "Friday", "priority": "none"],
                ["task": "Buy Milk", "priority": "urgent"],
                ["task": "7", "remove": true],
                ["task": "buy bread", "remove": true],
            ]
        ]

        let changes = AssistantPrompt.draftChanges(
            from: answer, titles: ["buy milk", "buy eggs"], request("Change that to Friday"))

        XCTAssertEqual(
            changes,
            [DraftChange(task: "buy eggs", due: "Friday"), DraftChange(task: "buy milk", priority: .urgent)])
    }

    func testTheDraftsTasksAreOnlyChangeableWithoutAProblem() {
        XCTAssertEqual(AssistantPrompt.draftTitles(request("Also add eggs", draft: milk)), ["buy milk"])
        XCTAssertEqual(AssistantPrompt.draftTitles(request("Fix it", draft: milk, problem: "No such Avenue")), [])
        XCTAssertEqual(AssistantPrompt.draftTitles(request("Also add eggs")), [])
    }

    // MARK: - Renames

    func testARenameToTheOldTitleIsToTheOtherQuotedPhrase() {
        let request = request("Rename “Call the plumber” to “Call the electrician”")

        XCTAssertEqual(
            AssistantPrompt.renamed("Call the plumber", to: "call the plumber", request), "Call the electrician")
        XCTAssertEqual(AssistantPrompt.renamed("Call the plumber", to: "Ring the plumber", request), "Ring the plumber")
        XCTAssertNil(AssistantPrompt.renamed("Call the plumber", to: "Call the plumber", self.request("Rename it")))
        XCTAssertNil(AssistantPrompt.renamed("Call the plumber", to: nil, request))
    }

    func testChangingThatIsToTheTaskAddedLast() {
        let draft = Plan(tasks: [.init(title: "buy milk", place: "HOME"), .init(title: "buy eggs", place: "HOME")])
        let answer: JSONValue = ["draftChanges": [["task": "1", "due": "friday"]]]
        let titles = ["buy milk", "buy eggs"]

        let that = AssistantPrompt.draftChanges(
            from: answer, titles: titles, request("Change that to Friday", draft: draft))
        XCTAssertEqual(that.map(\.task), ["buy eggs"])
        let first = AssistantPrompt.draftChanges(
            from: answer, titles: titles, request("Change the first one to Friday", draft: draft))
        XCTAssertEqual(first.map(\.task), ["buy milk"])
        let named = AssistantPrompt.draftChanges(
            from: answer, titles: titles, request("Change buy milk, that's for Friday", draft: draft))
        XCTAssertEqual(named.map(\.task), ["buy milk"])
    }

    // MARK: - Dates

    func testADateWorkedOutWrongIsTheDayThePersonNamed() throws {
        let request = request("Push “Book physio appointment” to next Monday")
        let due = try XCTUnwrap(AssistantPrompt.dated("2026-11-07", request))

        XCTAssertEqual(CalendarDate.resolving(due, today: CalendarDate(now, in: .gmt))?.description, "2026-11-09")
    }

    func testADateThePersonGaveOrNoSingleDayStaysAsWritten() {
        XCTAssertEqual(AssistantPrompt.dated("2026-11-07", request("Move it to 2026-11-07")), "2026-11-07")
        XCTAssertEqual(AssistantPrompt.dated("2026-11-07", request("Move it to Friday or Monday")), "2026-11-07")
        XCTAssertEqual(AssistantPrompt.dated("2026-11-07", request("Move it to next week")), "2026-11-07")
        XCTAssertEqual(AssistantPrompt.dated("friday", request("Move it to Monday")), "friday")
        XCTAssertNil(AssistantPrompt.dated(nil, request("Move it to Monday")))
    }

    // MARK: - What a message asks for

    func testAddingAndChangingMessagesAskForAPlanButQuestionsDont() {
        XCTAssertTrue(AssistantPrompt.asksForPlan(request("Remove the second one")))
        XCTAssertTrue(AssistantPrompt.asksForPlan(request("Please add a task to buy milk")))
        XCTAssertFalse(AssistantPrompt.asksForPlan(request("Can you add a task?")))
        XCTAssertFalse(AssistantPrompt.asksForPlan(request("What's due today")))
    }

    func testAMessageAsksForAChangeWhenItNamesATaskAndAChange() {
        XCTAssertTrue(AssistantPrompt.asksForChange(request("Push call the plumber to Friday")))
        XCTAssertFalse(AssistantPrompt.asksForChange(request("Tell me about the plumber")))
        XCTAssertFalse(AssistantPrompt.asksForChange(request("Move the dentist to Friday")))
    }

    // MARK: - Plans the model gets wrong

    func testANewProjectNobodyAskedForIsATaskWithSubTasks() {
        let plan = Plan(
            projects: [.init(name: "Plan the offsite", avenue: "HOME", tasks: [.init(title: "Book a venue")])])

        let asked = AssistantPrompt.withAskedProjects(plan, request("Plan the offsite: book a venue"))
        let kept = AssistantPrompt.withAskedProjects(plan, request("Make a project to plan the offsite"))

        XCTAssertEqual(asked.projects, [])
        XCTAssertEqual(asked.tasks.map(\.title), ["Plan the offsite"])
        XCTAssertEqual(asked.tasks.first?.subtasks.map(\.title), ["Book a venue"])
        XCTAssertEqual(kept, plan)
    }

    func testATaskRestatingItsProjectsGoalIsLeftOutUnlessThePersonSaidIt() {
        let plan = Plan(
            projects: [.init(name: "Swimming", avenue: "HEALTH", tasks: [.init(title: "Go swimming")])],
            tasks: [
                .init(title: "Stretch after every run", place: "HEALTH/Half marathon"),
                .init(title: "Run half marathon", place: "HEALTH/Half marathon"),
            ])

        let unsaid = AssistantPrompt.withoutRestatedGoals(plan, request("Add stretching to my Half marathon project"))
        let said = AssistantPrompt.withoutRestatedGoals(plan, request("Add run half marathon and go swimming"))

        XCTAssertEqual(unsaid.tasks.map(\.title), ["Stretch after every run"])
        XCTAssertEqual(unsaid.projects.first?.tasks, [])
        XCTAssertEqual(said, plan)
    }
}
