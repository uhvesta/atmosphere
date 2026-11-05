import ComposableArchitecture
import Foundation
import LifrAPI
import LifrAssistantClient
import LifrAssistantLive
import LifrCore
import LifrModelsClient
import LifrStoreClient
import XCTest

final class LiveAssistantTests: XCTestCase {
    /// 2026-11-03 10:00 UTC, a Tuesday.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)

    private func request(_ text: String, workspace: Workspace = Workspace(), writes: StoreClient? = nil)
        -> AssistantRequest
    {
        let save = writes.map { store in
            AssistantWrites(conversation: UUID()) { [now] operation in
                try await withDependencies {
                    $0.date.now = now
                    $0.uuid = .incrementing
                } operation: {
                    try await store.save(operation).workspace
                }
            }
        }
        return AssistantRequest(
            messages: [ChatMessage(id: UUID(), role: .person, text: text, at: now)], workspace: workspace, now: now,
            timeZone: .gmt, writes: save)
    }

    /// Every reply until the stream ends, and the error it ended with.
    private func replies(_ client: AssistantClient, _ request: AssistantRequest) async
        -> ([AssistantReply], AssistantError?)
    {
        var replies: [AssistantReply] = []
        do {
            for try await reply in client.reply(request) { replies.append(reply) }
            return (replies, nil)
        } catch let error as AssistantError {
            return (replies, error)
        } catch {
            XCTFail("unexpected \(error)")
            return (replies, nil)
        }
    }

    private let turn: JSONValue = [
        "message": "Here's a plan.",
        "plan": ["tasks": [["title": "Water plants", "place": "HOME"]]],
    ]

    func testTheAnswerStreamsAndEndsWithTheWholePlan() async {
        let model = ScriptedRuntime(steps: [ScriptedRuntime.answer(turn, partials: [["message": "Here's"]])])

        let (replies, error) = await replies(.live(runtime: { _ in model }), request("Plan the garden"))

        XCTAssertNil(error)
        XCTAssertEqual(replies.first?.text, "Here's")
        XCTAssertEqual(replies.first?.isComplete, false)
        let last = replies.last
        XCTAssertEqual(last?.isComplete, true)
        XCTAssertEqual(last?.plan?.tasks.map(\.title), ["Water plants"])
        XCTAssertEqual(replies.filter(\.isComplete).count, 1)
    }

    func testAPhotoGoesToAModelThatReadsThem() async {
        let model = ScriptedRuntime(inputs: [.image], steps: [ScriptedRuntime.answer(turn)])
        let photo = ModelAttachment(.image, file: URL(filePath: "/tmp/photo.jpg"))
        var request = request("What's this?")
        request.attachments = [photo]

        let (_, error) = await replies(.live(runtime: { _ in model }), request)

        XCTAssertNil(error)
        XCTAssertEqual(model.turns.map(\.attachments), [[photo]])
        XCTAssertEqual(AssistantClient.live(runtime: { _ in model }).inputs(), [.image])
    }

    func testARecordingToAModelThatCantListenSaysPickAnother() async {
        let model = ScriptedRuntime(steps: [ScriptedRuntime.answer(turn)])
        var request = request("")
        request.attachments = [ModelAttachment(.audio, file: URL(filePath: "/tmp/note.wav"))]

        let (replies, error) = await replies(.live(runtime: { _ in model }), request)

        XCTAssertEqual(replies, [])
        XCTAssertEqual(error, .cantRead(.audio))
    }

    func testAnUnreadableAnswerIsAskedForAgain() async {
        let model = ScriptedRuntime(steps: [ScriptedRuntime.fail(.decodingFailure), ScriptedRuntime.answer(turn)])

        let (replies, error) = await replies(.live(runtime: { _ in model }), request("Plan the garden"))

        XCTAssertNil(error)
        XCTAssertNotNil(replies.last?.plan)
        XCTAssertEqual(model.turns.count, 2)
        XCTAssertTrue(model.turns.allSatisfy { $0.schema != nil })
    }

    func testTwoUnreadableAnswersFallBackToPlainText() async {
        let model = ScriptedRuntime(steps: [
            ScriptedRuntime.fail(.decodingFailure), ScriptedRuntime.fail(.decodingFailure),
            ScriptedRuntime.answer("I'd add Water plants. Send it again for a plan."),
        ])

        let (replies, error) = await replies(.live(runtime: { _ in model }), request("Plan the garden"))

        XCTAssertNil(error)
        XCTAssertEqual(replies.last?.text, "I'd add Water plants. Send it again for a plan.")
        XCTAssertEqual(replies.last?.isComplete, true)
        XCTAssertNil(replies.last?.plan)
        let plain = model.turns.last
        XCTAssertNil(plain?.schema)
        XCTAssertEqual(plain?.tools.count, 0)
    }

    func testWhenEvenPlainTextCantBeReadItSaysSo() async {
        let model = ScriptedRuntime(steps: Array(repeating: ScriptedRuntime.fail(.decodingFailure), count: 3))

        let (_, error) = await replies(.live(runtime: { _ in model }), request("Plan the garden"))

        XCTAssertEqual(error, .unreadable(saved: false))
    }

    func testAnUnreadableAnswerAfterSavingIsntAskedForAgain() async throws {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        let home = records.avenue("HOME", "Home")
        let rent = records.task("Pay rent", in: home)
        let code = records.workspace.displayCode(of: rent)
        let store = StoreClient.inMemory(records.workspace, actor: .user(device: "iPhone", surface: .iPhone))
        let model = ScriptedRuntime(steps: [
            ScriptedRuntime.call(
                "updateTasks", ["codes": [.string(code)], "status": "done"],
                then: ScriptedRuntime.fail(.decodingFailure)),
            ScriptedRuntime.answer(turn),
        ])

        let (_, error) = await replies(
            .live(runtime: { _ in model }), request("Mark rent done", workspace: records.workspace, writes: store))

        XCTAssertEqual(error, .unreadable(saved: true))
        XCTAssertEqual(model.turns.count, 1)
        let saved = try await store.load()
        XCTAssertEqual(saved.tasks[rent.id]?.status, .done)
    }

    func testAnInterruptedAnswerSaysWhetherItSavedChanges() async {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        let home = records.avenue("HOME", "Home")
        let rent = records.task("Pay rent", in: home)
        let code = records.workspace.displayCode(of: rent)
        let store = StoreClient.inMemory(records.workspace, actor: .user(device: "iPhone", surface: .iPhone))
        let unsaved = ScriptedRuntime(steps: [ScriptedRuntime.fail(.interrupted(.memory))])
        let saved = ScriptedRuntime(steps: [
            ScriptedRuntime.call(
                "updateTasks", ["codes": [.string(code)], "status": "done"],
                then: ScriptedRuntime.fail(.interrupted(.background)))
        ])

        let (_, before) = await replies(.live(runtime: { _ in unsaved }), request("Plan the garden"))
        let (_, after) = await replies(
            .live(runtime: { _ in saved }), request("Mark rent done", workspace: records.workspace, writes: store))

        XCTAssertEqual(before, .interrupted(.memory, saved: false))
        XCTAssertEqual(after, .interrupted(.background, saved: true))
        XCTAssertEqual(unsaved.turns.count, 1)
    }

    func testTooLongSendsLessUntilTheSmallestSize() async {
        let model = ScriptedRuntime(steps: Array(repeating: ScriptedRuntime.fail(.exceededContext), count: 3))
        let request = request("Plan the garden")

        let (_, error) = await replies(.live(runtime: { _ in model }), request)

        XCTAssertEqual(error, .tooLong(saved: false))
        XCTAssertEqual(
            model.turns.map(\.prompt), AssistantPrompt.Size.allCases.map { AssistantPrompt.prompt(request, size: $0) })
    }

    func testTooLongAfterSavingStopsWithoutSendingAgain() async {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        let home = records.avenue("HOME", "Home")
        let rent = records.task("Pay rent", in: home)
        let code = records.workspace.displayCode(of: rent)
        let store = StoreClient.inMemory(records.workspace, actor: .user(device: "iPhone", surface: .iPhone))
        let model = ScriptedRuntime(steps: [
            ScriptedRuntime.call(
                "updateTasks", ["codes": [.string(code)], "status": "done"],
                then: ScriptedRuntime.fail(.exceededContext)),
            ScriptedRuntime.answer(turn),
        ])

        let (_, error) = await replies(
            .live(runtime: { _ in model }), request("Mark rent done", workspace: records.workspace, writes: store))

        XCTAssertEqual(error, .tooLong(saved: true))
        XCTAssertEqual(model.turns.count, 1)
    }

    func testAModelWithALargerWindowGetsTheFullRequest() async {
        let tasks = (0..<120).map { Plan.TaskDraft(title: "Task \($0) with a fairly long title", place: "HOME") }
        var request = request("Add these")
        request.draft = Plan(tasks: tasks)
        let small = ScriptedRuntime(steps: [ScriptedRuntime.answer(turn)])
        let large = ScriptedRuntime(
            budget: ContextBudget(window: 32_768, reservedForAnswer: 2_048), steps: [ScriptedRuntime.answer(turn)])

        _ = await replies(.live(runtime: { _ in small }), request)
        _ = await replies(.live(runtime: { _ in large }), request)

        XCTAssertEqual(large.turns.first?.prompt, AssistantPrompt.prompt(request, size: .full))
        XCTAssertNotEqual(small.turns.first?.prompt, AssistantPrompt.prompt(request, size: .full))
    }

    func testAToolTheModelCallsReadsTheRecords() async {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        let home = records.avenue("HOME", "Home")
        records.task("Pay rent", in: home)
        let found = LockIsolated("")
        let model = ScriptedRuntime(steps: [
            { turn, partial throws(ModelRuntimeError) in
                guard let tool = turn.tools.first(where: { $0.name == "findTasks" }) else {
                    throw .failed("no findTasks")
                }
                let result = await tool.call(["words": "rent"])
                found.setValue(result)
                return try await ScriptedRuntime.answer(["message": "You have one."])(turn, partial)
            }
        ])

        let (replies, error) = await replies(
            .live(runtime: { _ in model }), request("What's due?", workspace: records.workspace))

        XCTAssertNil(error)
        XCTAssertEqual(replies.last?.text, "You have one.")
        XCTAssertTrue(found.value.contains("Pay rent"))
    }

    func testWithoutWritesTheModelGetsOnlyReadingTools() async {
        let model = ScriptedRuntime(steps: [ScriptedRuntime.answer(turn)])

        _ = await replies(.live(runtime: { _ in model }), request("Mark rent done"))

        let names = Set(model.turns.first?.tools.map(\.name) ?? [])
        XCTAssertEqual(names, ["findTasks", "getTask", "overview", "recentActivity"])
    }

    func testAnUnavailableModelSaysWhy() async {
        let model = ScriptedRuntime(status: .notEnabled, steps: [])
        let client = AssistantClient.live(runtime: { _ in model })

        let (_, error) = await replies(client, request("Plan the garden"))

        XCTAssertEqual(client.availability(), .notEnabled)
        XCTAssertEqual(error, .unavailable(.notEnabled))
        XCTAssertTrue(model.turns.isEmpty)
        client.prewarm()
        XCTAssertEqual(model.prewarmCount, 0)
    }

    func testACancelledAnswerEndsQuietly() async {
        let model = ScriptedRuntime(steps: [ScriptedRuntime.fail(.cancelled)])

        let (replies, error) = await replies(.live(runtime: { _ in model }), request("Plan the garden"))

        XCTAssertNil(error)
        XCTAssertTrue(replies.isEmpty)
    }

    func testModelErrorsBecomeTheAssistantsOwn() async {
        let cases: [(ModelRuntimeError, AssistantError)] = [
            (.declined, .declined), (.busy, .busy),
            (.simulatorHost("versions differ"), .simulatorHost("versions differ")),
        ]
        for (thrown, expected) in cases {
            let model = ScriptedRuntime(steps: [ScriptedRuntime.fail(thrown)])

            let (_, error) = await replies(.live(runtime: { _ in model }), request("Plan the garden"))

            XCTAssertEqual(error, expected)
        }
    }

    func testTheModelIsAskedForEachRequest() async {
        let first = ScriptedRuntime(steps: [ScriptedRuntime.answer(turn)])
        let second = ScriptedRuntime(steps: [ScriptedRuntime.answer(turn)])
        let current = LockIsolated(first)
        let client = AssistantClient.live(runtime: { _ in current.value })

        _ = await replies(client, request("Plan the garden"))
        current.setValue(second)
        _ = await replies(client, request("Plan the garden"))

        XCTAssertEqual(first.turns.count, 1)
        XCTAssertEqual(second.turns.count, 1)
    }

    func testEachUseRunsOnItsOwnModel() async throws {
        let chat = ScriptedRuntime(steps: [ScriptedRuntime.answer(turn)])
        let skills = ScriptedRuntime(steps: [
            ScriptedRuntime.answer([
                "name": "Weekly review", "summary": "", "instructions": "List what got done.", "tools": [],
                "draftsPlans": false,
            ])
        ])
        let titles = ScriptedRuntime(steps: [ScriptedRuntime.answer("Garden plans")])
        let client = AssistantClient.live { use in
            switch use {
            case .assistant: chat
            case .skills: skills
            case .titles: titles
            }
        }

        _ = await replies(client, request("Plan the garden"))
        _ = try await client.draftSkill("review my week")
        _ = try await client.suggestTitle([ChatMessage(id: UUID(), role: .person, text: "Plan the garden", at: now)])

        XCTAssertEqual(chat.turns.count, 1)
        XCTAssertEqual(skills.turns.count, 1)
        XCTAssertEqual(titles.turns.count, 1)
    }

    func testASkillIsDraftedFromTheModelsAnswer() async throws {
        let model = ScriptedRuntime(steps: [
            ScriptedRuntime.answer([
                "name": "Weekly review", "summary": "Look back at the week.",
                "instructions": "List what got done.", "tools": ["overview", "findTasks"], "draftsPlans": false,
            ])
        ])

        let draft = try await AssistantClient.live(runtime: { _ in model }).draftSkill("review my week")

        XCTAssertEqual(draft.name, "Weekly review")
        XCTAssertEqual(draft.tools, [.overview, .findTasks])
        XCTAssertFalse(draft.draftsPlans)
        XCTAssertNotNil(model.turns.first?.schema)
    }

    func testATitleIsTheModelsFirstLine() async throws {
        let model = ScriptedRuntime(steps: [ScriptedRuntime.answer("Title: Garden plans.\nMore")])

        let title = try await AssistantClient.live(runtime: { _ in model }).suggestTitle([
            ChatMessage(id: UUID(), role: .person, text: "Plan the garden", at: now)
        ])

        XCTAssertEqual(title, "Garden plans")
    }
}
