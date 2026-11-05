import Foundation
import LifrAssistantClient
import LifrCore
import LifrStoreClient
import XCTest

final class AssistantBudgetTests: XCTestCase {
    /// 2026-11-03 10:00 UTC, a Tuesday.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)

    private func request(
        _ texts: [String], draft: Plan? = nil, skill: Skill? = nil, workspace: Workspace = Workspace()
    ) -> AssistantRequest {
        AssistantRequest(
            messages: texts.enumerated().map { index, text in
                ChatMessage(id: UUID(), role: index % 2 == 0 ? .person : .assistant, text: text, at: now)
            },
            draft: draft, workspace: workspace, now: now, timeZone: .gmt,
            writes: AssistantWrites(conversation: UUID()) { _ in Workspace() }, skill: skill)
    }

    func testAShortConversationStartsAtFullSize() {
        XCTAssertEqual(AssistantPrompt.sizes(request(["Plan my week"])), [.full, .smaller, .smallest])
    }

    func testAsTheDraftGrowsLessIsSentAndWhatsSentFits() {
        let room = AssistantPrompt.contextWindow - AssistantPrompt.reservedForAnswer
        let earlier = String(repeating: "word ", count: 50)
        var firsts: [AssistantPrompt.Size?] = []
        for count in 0...200 {
            let tasks = (0..<count).map { Plan.TaskDraft(title: "Task \($0) with a fairly long title", place: "HOME") }
            let request = request(
                [earlier, earlier, "Add these"], draft: tasks.isEmpty ? nil : Plan(tasks: tasks))
            let sizes = AssistantPrompt.sizes(request)
            if let first = sizes.first {
                XCTAssertLessThanOrEqual(AssistantPrompt.estimatedTokens(request, size: first), room)
                XCTAssertEqual(sizes, AssistantPrompt.Size.allCases.filter { $0 >= first })
            }
            if let bigger = AssistantPrompt.Size.allCases.last(where: { $0 < (sizes.first ?? .smallest) }) {
                XCTAssertGreaterThan(AssistantPrompt.estimatedTokens(request, size: bigger), room)
            }
            if firsts.last != sizes.first { firsts.append(sizes.first) }
        }

        XCTAssertEqual(firsts, [.full, .smaller, .smallest, nil])
    }

    func testADraftTooBigForAnySizeFitsNone() {
        let tasks = (1...300).map { Plan.TaskDraft(title: "Task number \($0) with a long title", place: "HOME") }

        XCTAssertEqual(AssistantPrompt.sizes(request(["Add these"], draft: Plan(tasks: tasks))), [])
    }

    func testTheSmallestSizeSendsOnlyTheLastMessage() {
        let prompt = AssistantPrompt.prompt(
            request(["Plan the garden", "Which beds?", "The north one"]), size: .smallest)

        XCTAssertTrue(prompt.contains("The north one"))
        XCTAssertFalse(prompt.contains("Plan the garden"))
    }

    func testFullSizeSendsEveryAllowedTool() {
        XCTAssertEqual(
            AssistantPrompt.tools(request(["What's due?"]), size: .full), Set(AssistantTool.allCases))
    }

    func testSmallerSizesSendTheToolsTheMessageNeeds() {
        let question = AssistantPrompt.tools(request(["What's due this week?"]), size: .smaller)
        XCTAssertEqual(question, [.findTasks, .getTask, .overview])

        let change = AssistantPrompt.tools(request(["Mark HOME-0003 done"]), size: .smaller)
        XCTAssertTrue(change.contains(.updateTasks))
        XCTAssertFalse(change.contains(.revert))

        let undo = AssistantPrompt.tools(request(["Undo that"]), size: .smallest)
        XCTAssertTrue(undo.isSuperset(of: [.recentActivity, .revert]))

        let comment = AssistantPrompt.tools(request(["Add a note to the rent task"]), size: .smaller)
        XCTAssertTrue(comment.contains(.addComment))
    }

    func testASkillKeepsItsToolsAtEverySize() {
        for size in AssistantPrompt.Size.allCases {
            XCTAssertEqual(
                AssistantPrompt.tools(request(["Go"], skill: .undo), size: size), [.recentActivity, .revert])
        }
    }

    func testSmallerSizesLeaveOutLabels() {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        _ = records.avenue("HOME", "Home")
        _ = records.label("errand")
        let request = request(["Plan"], workspace: records.workspace)

        XCTAssertTrue(AssistantPrompt.instructions(request, size: .full).contains("errand"))
        XCTAssertFalse(AssistantPrompt.instructions(request, size: .smaller).contains("errand"))
        XCTAssertTrue(AssistantPrompt.instructions(request, size: .smaller).contains("HOME"))
    }

    func testALongToolResultIsCutAtALineAndSaysSo() {
        let tools = AssistantTools(workspace: Workspace(), now: now, timeZone: .gmt)
        tools.resultBudget = 200
        let lines = (1...50).map { "- line \($0) of the result" }
        let result = lines.joined(separator: "\n")

        let capped = tools.capped(result)

        XCTAssertLessThanOrEqual(capped.count, 200)
        XCTAssertTrue(capped.hasPrefix(lines[0] + "\n"))
        let kept = capped.split(separator: "\n").dropLast()
        XCTAssertTrue(kept.allSatisfy { lines.contains(String($0)) })
        XCTAssertLessThan(kept.count, lines.count)
    }

    func testAShortToolResultIsUnchanged() {
        let tools = AssistantTools(workspace: Workspace(), now: now, timeZone: .gmt)

        XCTAssertEqual(tools.capped("- one task"), "- one task")
    }

    func testManyFoundTasksStayWithinTheCap() {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        let home = records.avenue("HOME", "Home")
        for index in 1...40 { records.task("Chore \(index) with quite a long title to fill the line", in: home) }
        let tools = AssistantTools(workspace: records.workspace, now: now, timeZone: .gmt)

        let result = tools.capped(tools.findTasks(words: "", avenue: nil, status: nil, dueBy: nil))

        XCTAssertLessThanOrEqual(result.count, tools.resultBudget)
    }
}
