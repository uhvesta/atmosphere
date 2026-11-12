import ComposableArchitecture
import Foundation
import LifrAPI
import LifrAssistantClient
import LifrCore
import LifrModelsClient
import LifrStoreClient
import XCTest

final class AssistantTests: XCTestCase {
    /// 2026-11-03 10:00 UTC, a Tuesday.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)
    private lazy var records = RecordsBuilder(now: now, timeZone: .gmt)

    private var tools: AssistantTools { AssistantTools(workspace: records.workspace, now: now, timeZone: .gmt) }

    private func code(_ task: LifrTask) -> String { records.workspace.displayCode(of: task) }

    func testLookupListsAvenuesProjectsOpenTopLevelTasksAndLabels() {
        let home = records.avenue("HOME", "Home")
        let move = records.project("Move", in: home)
        let older = records.task("Pack books", in: home, project: move)
        let newer = records.task("Call movers", in: home)
        records.task("Tape", in: home, parent: older)
        let done = records.task("Done", in: home)
        records.setStatus(.done, of: done)
        _ = records.label("errand")

        let lookup = AssistantLookup(records.workspace)

        XCTAssertEqual(lookup.places, [.init(key: "HOME", name: "Home", projects: ["Move"])])
        XCTAssertEqual(Set(lookup.tasks.map(\.code)), [code(older), code(newer)])
        XCTAssertEqual(lookup.tasks.first { $0.code == code(older) }?.place, "HOME/Move")
        XCTAssertEqual(lookup.labels, ["errand"])
    }

    func testHistoryKeepsTheNewestMessagesThatFit() {
        let long = String(repeating: "a", count: AssistantPrompt.historyBudget - 5)
        let messages = ["first", long, "short", "last"].enumerated().map { index, text in
            ChatMessage(id: UUID(), role: index % 2 == 0 ? .person : .assistant, text: text, at: now)
        }

        XCTAssertEqual(AssistantPrompt.history(messages).map(\.text), ["short", "last"])
        XCTAssertEqual(AssistantPrompt.history(Array(messages.prefix(1))).map(\.text), ["first"])
        XCTAssertEqual(AssistantPrompt.history([]), [])
    }

    func testHistoryCutsALastMessageThatIsTooLongByItself() {
        let text = "start" + String(repeating: "b", count: AssistantPrompt.historyBudget)
        let message = ChatMessage(id: UUID(), role: .person, text: text, at: now)

        let kept = AssistantPrompt.history([message])

        XCTAssertEqual(kept.map(\.text.count), [AssistantPrompt.historyBudget])
        XCTAssertFalse(kept[0].text.hasPrefix("start"))
    }

    func testAPartialAnswerBecomesAPlanWithTheTasksWrittenSoFar() {
        let answer: JSONValue = .object([
            "message": .string(" Here you go. "),
            "plan": .object([
                "tasks": .array([
                    .object([
                        "title": .string("Clean"), "place": .string("HOME"), "priority": .string("high"),
                        "labels": .array([.string("chore")]),
                        "subtasks": .array([
                            .object(["title": .string("Vacuum"), "place": .string("WORK")]), .object([:]),
                        ]),
                    ]),
                    .object(["place": .string("HOME")]),
                ])
            ]),
        ])

        let reply = AssistantPrompt.reply(from: answer, isComplete: false)

        XCTAssertEqual(reply.text, "Here you go.")
        XCTAssertFalse(reply.isComplete)
        let task = try? XCTUnwrap(reply.plan?.tasks.first)
        XCTAssertEqual(reply.plan?.tasks.count, 1)
        XCTAssertEqual(task?.place, "HOME")
        XCTAssertEqual(task?.priority, .high)
        XCTAssertEqual(task?.labels, ["chore"])
        XCTAssertEqual(task?.subtasks.map(\.title), ["Vacuum"])
        XCTAssertNil(task?.subtasks.first?.place)
    }

    func testAnAnswerWithoutAPlanHasNone() {
        let reply = AssistantPrompt.reply(
            from: .object(["message": .string("Which Avenue?"), "plan": .object(["tasks": .array([])])]),
            isComplete: true)

        XCTAssertEqual(reply, AssistantReply(text: "Which Avenue?", plan: nil, isComplete: true))
    }

    func testThePromptCarriesTheDraftAndItsProblem() throws {
        _ = records.avenue("HOME", "Home")
        let request = AssistantRequest(
            messages: [ChatMessage(id: UUID(), role: .person, text: "Add laundry", at: now)],
            draft: Plan(tasks: [.init(title: "Laundry", place: "HOME")]), problem: "no such label",
            workspace: records.workspace, now: now, timeZone: .gmt, skill: .add)

        let prompt = AssistantPrompt.prompt(request)

        XCTAssertTrue(prompt.contains("Add laundry"))
        XCTAssertTrue(prompt.contains(PlanFile.write(try XCTUnwrap(request.draft))))
        XCTAssertTrue(prompt.contains("no such label"))
        XCTAssertTrue(AssistantPrompt.instructions(request).contains("HOME"))
    }

    func testThePlainTextFallbackOffersNoToolsAndNoPlan() {
        let home = records.avenue("HOME", "Home")
        let request = AssistantRequest(
            messages: [ChatMessage(id: UUID(), role: .person, text: "What's left at home?", at: now)],
            workspace: records.workspace, now: now, timeZone: .gmt, skill: .query)

        let instructions = AssistantPrompt.plainInstructions(request)

        XCTAssertTrue(instructions.contains(home.key.rawValue))
        for tool in AssistantTool.allCases {
            XCTAssertFalse(instructions.contains(tool.rawValue), "\(tool) is offered")
        }
        XCTAssertTrue(AssistantPrompt.instructions(request).contains(AssistantTool.findTasks.rawValue))
    }

    func testFindTasksMatchesWordsAvenueStatusAndDueDate() {
        let home = records.avenue("HOME", "Home")
        let work = records.avenue("WORK", "Work")
        let laundry = records.task("Do laundry", in: home, due: 1)
        let report = records.task("Laundry report", in: work, due: 10)
        let done = records.task("Fold laundry", in: home)
        records.setStatus(.done, of: done)

        func codes(_ text: String) -> Set<String> {
            Set([laundry, report, done].map(code).filter { text.contains($0) })
        }

        XCTAssertEqual(
            codes(tools.findTasks(words: "laundry", avenue: nil, status: nil, dueBy: nil)),
            [code(laundry), code(report)])
        XCTAssertEqual(
            codes(tools.findTasks(words: "laundry", avenue: "home", status: nil, dueBy: nil)),
            [code(laundry)])
        XCTAssertEqual(codes(tools.findTasks(words: "", avenue: nil, status: "done", dueBy: nil)), [code(done)])
        XCTAssertEqual(codes(tools.findTasks(words: "", avenue: nil, status: nil, dueBy: "+3d")), [code(laundry)])
        XCTAssertEqual(codes(tools.findTasks(words: "", avenue: "NOPE", status: nil, dueBy: nil)), [])
        XCTAssertEqual(codes(tools.findTasks(words: "", avenue: nil, status: "finished", dueBy: nil)), [])
    }

    func testFindTasksListsAtMostTheLimit() {
        let home = records.avenue("HOME", "Home")
        for index in 0..<(AssistantTools.limit + 3) { records.task("Task \(index)", in: home) }

        let lines = tools.findTasks(words: "task", avenue: nil, status: nil, dueBy: nil)
            .split(separator: "\n").filter { $0.hasPrefix("- ") }

        XCTAssertEqual(lines.count, AssistantTools.limit)
    }

    func testGetTaskIncludesSubtasksAndComments() {
        let home = records.avenue("HOME", "Home")
        let clean = records.task("Clean", in: home)
        let vacuum = records.task("Vacuum", in: home, parent: clean)
        records.comment("Use the new bags", on: clean)

        let text = tools.getTask(code: code(clean).lowercased())

        XCTAssertTrue(text.contains(code(vacuum)))
        XCTAssertTrue(text.contains("Use the new bags"))
        XCTAssertFalse(tools.getTask(code: "HOME-9999").contains("HOME-9999 ·"))
    }

    func testOverviewSortsTasksByWhenTheyAreDue() throws {
        let home = records.avenue("HOME", "Home")
        let late = records.task("Late", in: home, due: -2)
        let today = records.task("Today", in: home, due: 0)
        let later = records.task("Later", in: home, due: 30)

        let text = tools.overview()

        XCTAssertTrue(text.contains(code(late)))
        XCTAssertTrue(text.contains(code(today)))
        XCTAssertFalse(text.contains(code(later)))
        let lateAt = try XCTUnwrap(text.range(of: code(late))).lowerBound
        XCTAssertLessThan(lateAt, try XCTUnwrap(text.range(of: code(today))).lowerBound)
    }

    func testOverviewCountsEverySectionAndShowsSomeOfEach() {
        let home = records.avenue("HOME", "Home")
        for index in 0..<30 { records.task("Late \(index)", in: home, due: -2) }
        let soon = records.task("Soon", in: home, due: 3)

        let text = tools.overview()

        XCTAssertTrue(text.contains("(30)"))
        XCTAssertTrue(text.contains(code(soon)))
    }

    func testTheScriptedClientStreamsTheTextThenTheWholeReply() async throws {
        let client = AssistantClient.scripted([.example])
        let request = AssistantRequest(messages: [], workspace: Workspace(), now: now, timeZone: .gmt)

        var replies: [AssistantReply] = []
        for try await reply in client.reply(request) { replies.append(reply) }

        XCTAssertEqual(replies, [AssistantReply(text: AssistantReply.example.text), AssistantReply.example])
        XCTAssertEqual(client.availability(.defaults), .available)
    }

    func testASuggestedTitleIsItsFirstLineWithoutQuotesOrALabel() {
        XCTAssertEqual(AssistantPrompt.title(from: "Title: \"Weekly plan.\"\nMore"), "Weekly plan")
        XCTAssertEqual(AssistantPrompt.title(from: "  “Race day”  "), "Race day")
        XCTAssertNil(AssistantPrompt.title(from: "\"\""))
        XCTAssertEqual(AssistantPrompt.title(from: String(repeating: "a", count: 60))?.count, 40)
    }

    func testAConversationSavedBeforeItHadAModelOfItsOwnFollowsTheDefaults() throws {
        let old = Data(#"{"changes":[],"added":[]}"#.utf8)
        XCTAssertEqual(try JSONDecoder().decode(AssistantContext.self, from: old).model, .defaults)

        var conversation = Conversation(id: UUID(), createdAt: now)
        conversation.context.model = ModelChoice(model: "qwen3.5-2b", thinking: .long, window: 16_384)
        let kept = try JSONDecoder().decode(Conversation.self, from: JSONEncoder().encode(conversation))
        XCTAssertEqual(kept.context.model, conversation.context.model)
    }

    func testAConversationKeepsWhetherItPicksSkillsItself() throws {
        let old = Data(#"{"changes":[],"added":[]}"#.utf8)
        XCTAssertNil(try JSONDecoder().decode(AssistantContext.self, from: old).picksSkills)

        var conversation = Conversation(id: UUID(), createdAt: now)
        XCTAssertNil(conversation.context.picksSkills, "a new one follows Settings")
        conversation.context.picksSkills = false
        let kept = try JSONDecoder().decode(Conversation.self, from: JSONEncoder().encode(conversation))
        XCTAssertEqual(kept.context.picksSkills, false)
    }

    func testAConversationSavedBeforeNamesHasNone() throws {
        let old = Data(#"{"id":"\#(UUID())","createdAt":0}"#.utf8)
        let decoded = try JSONDecoder().decode(Conversation.self, from: old)
        XCTAssertNil(decoded.name)
        XCTAssertFalse(decoded.isNamedByHand)
        XCTAssertEqual(decoded.title, "New conversation")
    }
}
