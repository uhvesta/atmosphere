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

    func testAPlanOfOnlyWhatsThereAlreadyIsAskedForAgain() async {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        _ = records.avenue("HOME", "Home")
        let model = ScriptedRuntime(steps: [
            ScriptedRuntime.answer([
                "message": "I added buy milk to HOME.", "plan": ["avenues": [["key": "HOME", "name": "Home"]]],
            ]),
            ScriptedRuntime.answer([
                "message": "I added buy milk to HOME.", "plan": ["tasks": [["title": "buy milk", "place": "HOME"]]],
            ]),
        ])

        let (replies, error) = await replies(
            .live(runtime: { _ in model }), request("Add a task to buy milk, in HOME", workspace: records.workspace))

        XCTAssertNil(error)
        XCTAssertEqual(replies.last?.plan?.tasks.map(\.title), ["buy milk"])
        XCTAssertEqual(model.turns.count, 2)
        XCTAssertNotEqual(model.turns.first?.prompt, model.turns.last?.prompt, "Asked the same way again")
    }

    func testAPlanTheModelKeepsLeavingEmptyEndsInPlainText() async {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        _ = records.avenue("HOME", "Home")
        let empty: JSONValue = ["message": "Added it.", "plan": ["avenues": [["key": "HOME", "name": "Home"]]]]
        let model = ScriptedRuntime(steps: [
            ScriptedRuntime.answer(empty), ScriptedRuntime.answer(empty),
            ScriptedRuntime.answer("I couldn't make a plan. Try asking again."),
        ])

        let (replies, error) = await replies(
            .live(runtime: { _ in model }), request("Add a task to buy milk, in HOME", workspace: records.workspace))

        XCTAssertNil(error)
        XCTAssertNil(replies.last?.plan)
        XCTAssertEqual(model.turns.count, 3)
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
                "addComment", ["code": .string(code), "text": "Paid"],
                then: ScriptedRuntime.fail(.decodingFailure)),
            ScriptedRuntime.answer(turn),
        ])

        let (_, error) = await replies(
            .live(runtime: { _ in model }),
            request("Note that rent is paid", workspace: records.workspace, writes: store))

        XCTAssertEqual(error, .unreadable(saved: true))
        XCTAssertEqual(model.turns.count, 1)
        let saved = try await store.load()
        XCTAssertEqual(saved.comments.values.map(\.body), ["Paid"])
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
                "addComment", ["code": .string(code), "text": "Paid"],
                then: ScriptedRuntime.fail(.interrupted(.background)))
        ])

        let (_, before) = await replies(.live(runtime: { _ in unsaved }), request("Plan the garden"))
        let (_, after) = await replies(
            .live(runtime: { _ in saved }),
            request("Note that rent is paid", workspace: records.workspace, writes: store))

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
                "addComment", ["code": .string(code), "text": "Paid"],
                then: ScriptedRuntime.fail(.exceededContext)),
            ScriptedRuntime.answer(turn),
        ])

        let (_, error) = await replies(
            .live(runtime: { _ in model }),
            request("Note that rent is paid", workspace: records.workspace, writes: store))

        XCTAssertEqual(error, .tooLong(saved: true))
        XCTAssertEqual(model.turns.count, 1)
    }

    func testTheRepliesCarryWhatTheModelThoughtAndHowLong() async {
        let model = ScriptedRuntime(steps: [
            ScriptedRuntime.think("Plants need water.", for: .seconds(3)) { turn, partial throws(ModelRuntimeError) in
                try await ScriptedRuntime.answer(self.turn, partials: [["message": "Here's"]])(turn, partial)
            }
        ])

        let (replies, error) = await replies(.live(runtime: { _ in model }), request("Plan the garden"))

        XCTAssertNil(error)
        XCTAssertEqual(replies.first?.thinking, ModelThinking(text: "Plants need water.", processor: .gpu))
        XCTAssertEqual(
            replies.last?.thinking, ModelThinking(text: "Plants need water.", duration: .seconds(3), processor: .gpu))
        XCTAssertEqual(replies.last?.isComplete, true)
    }

    func testNamingAConversationDoesntWaitForThinking() async throws {
        let model = ScriptedRuntime(steps: [ScriptedRuntime.answer("Garden plans")])

        _ = try await AssistantClient.live(runtime: { _ in model }).suggestTitle([
            ChatMessage(id: UUID(), role: .person, text: "Plan the garden", at: now)
        ])

        XCTAssertNil(model.turns.first?.thinking)
    }

    func testAModelWithALargerWindowGetsTheFullRequest() async {
        let tasks = (0..<400).map { Plan.TaskDraft(title: "Task \($0) with a fairly long title", place: "HOME") }
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

    private func homeStore() -> (RecordsBuilder, LifrTask, LifrTask, StoreClient) {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        let home = records.avenue("HOME", "Home")
        records.avenue("WORK", "Work")
        let rent = records.task("Pay rent", in: home)
        let passport = records.task("Renew passport", in: home)
        let store = StoreClient.inMemory(records.workspace, actor: .user(device: "iPhone", surface: .iPhone))
        return (records, rent, passport, store)
    }

    func testTheAnswersChangesAreSavedAndTheirTasksLeftOutOfThePlan() async throws {
        let (records, rent, passport, store) = homeStore()
        let model = ScriptedRuntime(steps: [
            ScriptedRuntime.answer([
                "message": "Moved it and marked rent done.",
                "changes": [
                    ["code": .string(records.workspace.displayCode(of: passport)), "moveTo": "WORK"],
                    ["code": .string(records.workspace.displayCode(of: rent)), "status": "done"],
                ],
                "plan": ["tasks": [["title": "Renew passport", "place": "WORK"]]],
            ])
        ])

        let (replies, error) = await replies(
            .live(runtime: { _ in model }),
            request("Move Renew passport to WORK and pay rent is done", workspace: records.workspace, writes: store))

        XCTAssertNil(error)
        XCTAssertNil(replies.last?.plan, "the moved task would be added again")
        XCTAssertEqual(replies.last?.text, "Moved it and marked rent done.")
        let saved = try await store.load()
        XCTAssertEqual(saved.tasks[passport.id].flatMap { saved.avenues[$0.avenueID]?.key.rawValue }, "WORK")
        XCTAssertEqual(saved.tasks[rent.id]?.status, .done)
        XCTAssertNotNil(model.turns.first?.prompt.range(of: records.workspace.displayCode(of: passport)))
    }

    func testARenameToTheOldTitleIsToTheQuotedOne() async throws {
        let (records, rent, _, store) = homeStore()
        let model = ScriptedRuntime(steps: [
            ScriptedRuntime.answer([
                "message": "Renamed it.",
                "changes": [["code": .string(records.workspace.displayCode(of: rent)), "rename": "Pay rent"]],
            ])
        ])

        let (_, error) = await replies(
            .live(runtime: { _ in model }),
            request("Rename “Pay rent” to “Pay the rent”", workspace: records.workspace, writes: store))

        XCTAssertNil(error)
        let saved = try await store.load()
        XCTAssertEqual(saved.tasks[rent.id]?.title, "Pay the rent")
    }

    func testAChangeLifrCantMakeIsSaidAfterTheMessage() async throws {
        let (records, rent, _, store) = homeStore()
        let model = ScriptedRuntime(steps: [
            ScriptedRuntime.answer([
                "message": "Moved it.",
                "changes": [["code": .string(records.workspace.displayCode(of: rent)), "moveTo": "GARDEN"]],
            ])
        ])

        let (replies, _) = await replies(
            .live(runtime: { _ in model }),
            request("Move pay rent to garden", workspace: records.workspace, writes: store))

        let text = try XCTUnwrap(replies.last?.text)
        XCTAssertTrue(text.hasPrefix("Moved it."))
        XCTAssertTrue(text.contains(TaskChangeError.unknownAvenue(key: "GARDEN", suggestion: nil).description))
        let journal = try await store.load().journal
        XCTAssertEqual(journal, [])
    }

    func testWithoutWritesTheAnswerHasNoChanges() async throws {
        let (records, rent, _, store) = homeStore()
        let model = ScriptedRuntime(steps: [
            ScriptedRuntime.answer([
                "message": "Done.",
                "changes": [["code": .string(records.workspace.displayCode(of: rent)), "status": "done"]],
            ])
        ])

        _ = await replies(.live(runtime: { _ in model }), request("Mark rent done", workspace: records.workspace))

        let saved = try await store.load()
        XCTAssertEqual(saved.tasks[rent.id]?.status, .todo)
    }

    func testTheAnswersProjectChangesAreSaved() async throws {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        let health = records.avenue("HEALTH", "Health")
        let marathon = records.project("Half marathon", in: health)
        let store = StoreClient.inMemory(records.workspace, actor: .user(device: "iPhone", surface: .iPhone))
        let model = ScriptedRuntime(steps: [
            ScriptedRuntime.answer([
                "message": "Renamed it and set it for 6 December.",
                "projectChanges": [
                    ["project": "HEALTH/Half marathon", "rename": "Spring half marathon", "due": "2026-12-06"]
                ],
            ])
        ])

        let (replies, error) = await replies(
            .live(runtime: { _ in model }),
            request(
                "Rename the half marathon to “Spring half marathon”, due 2026-12-06", workspace: records.workspace,
                writes: store))

        XCTAssertNil(error)
        XCTAssertEqual(replies.last?.text, "Renamed it and set it for 6 December.")
        let saved = try await store.load()
        XCTAssertEqual(saved.projects[marathon.id]?.name, "Spring half marathon")
        XCTAssertEqual(saved.projects[marathon.id]?.dueDate?.description, "2026-12-06")
    }

    func testAProjectChangeLifrCantMakeIsSaidAfterTheMessage() async throws {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        let health = records.avenue("HEALTH", "Health")
        let marathon = records.project("Half marathon", in: health)
        records.task("Buy running shoes", in: health, project: marathon)
        let store = StoreClient.inMemory(records.workspace, actor: .user(device: "iPhone", surface: .iPhone))
        let model = ScriptedRuntime(steps: [
            ScriptedRuntime.answer([
                "message": "Completed it.",
                "projectChanges": [["project": "HEALTH/Half marathon", "status": "completed"]],
            ])
        ])

        let (replies, _) = await replies(
            .live(runtime: { _ in model }),
            request("Complete the half marathon", workspace: records.workspace, writes: store))

        let text = try XCTUnwrap(replies.last?.text)
        XCTAssertTrue(text.hasPrefix("Completed it."))
        XCTAssertGreaterThan(text.count, "Completed it.".count, "the refusal is said")
        let saved = try await store.load()
        XCTAssertEqual(saved.projects[marathon.id]?.status, .active)
    }

    func testOnlyTheTasksTheMessageMeansCanBeChanged() async throws {
        let (records, rent, passport, store) = homeStore()
        let model = ScriptedRuntime(steps: [
            ScriptedRuntime.answer([
                "message": "Done.",
                "changes": [["code": .string(records.workspace.displayCode(of: passport)), "status": "done"]],
            ])
        ])

        _ = await replies(
            .live(runtime: { _ in model }), request("Pay rent is done", workspace: records.workspace, writes: store))

        let codes = model.turns.first.map { Self.changeCodes(in: $0.schema) }
        XCTAssertEqual(codes, [records.workspace.displayCode(of: rent)])
        let saved = try await store.load()
        XCTAssertEqual(saved.tasks[passport.id]?.status, .todo, "a change to a task the message didn't mean")
    }

    func testWithoutAMeantTaskTheAnswerHasNoChanges() async throws {
        let (records, _, _, store) = homeStore()
        let model = ScriptedRuntime(steps: [ScriptedRuntime.answer(["message": "Added."])])

        _ = await replies(
            .live(runtime: { _ in model }), request("Add buy eggs", workspace: records.workspace, writes: store))

        XCTAssertEqual(model.turns.first.map { Self.changeCodes(in: $0.schema) }, [])
    }

    /// The task codes a turn's schema lets the model change.
    private static func changeCodes(in schema: ModelSchema?) -> [String] {
        guard case .object(_, let properties)? = schema,
            case .array(.object(_, let fields))? = properties.first(where: { $0.name == "changes" })?.schema,
            case .choice(_, let codes)? = fields.first(where: { $0.name == "code" })?.schema
        else { return [] }
        return codes
    }

    func testAToolTheModelCallsReadsTheRecords() async {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        let home = records.avenue("HOME", "Home")
        records.task("Pay rent", in: home)
        let found = LockIsolated("")
        // Typed, not inferred in the array: Swift 6.4.2 crashes inferring it.
        let step: ScriptedRuntime.Step = { turn, partial throws(ModelRuntimeError) in
            guard let tool = turn.tools.first(where: { $0.name == "findTasks" }) else {
                throw .failed("no findTasks")
            }
            let result = await tool.call(["words": "rent"])
            found.setValue(result)
            return try await ScriptedRuntime.answer(["message": "You have one."])(turn, partial)
        }
        let model = ScriptedRuntime(steps: [step])

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
            case .search: ScriptedRuntime(steps: [])
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

    func testCompactingSummarizesOnTheAssistantsModelWithTheEarlierSummary() async throws {
        let model = ScriptedRuntime(steps: [ScriptedRuntime.answer("They planned the garden.")])
        let messages = [
            ChatMessage(id: UUID(), role: .person, text: "Plan the garden", at: now),
            ChatMessage(id: UUID(), role: .assistant, text: "Here's a plan.", at: now),
        ]
        let used = LockIsolated<[ModelUse]>([])
        let client = AssistantClient.live { use in
            used.withValue { $0.append(use) }
            return model
        }

        let summary = try await client.compact(messages, "They moved in.")

        XCTAssertEqual(summary, "They planned the garden.")
        XCTAssertEqual(used.value, [.assistant])
        let prompt = try XCTUnwrap(model.turns.first?.prompt)
        XCTAssertTrue(prompt.contains("They moved in."))
        XCTAssertTrue(prompt.contains("Plan the garden"))
    }

    func testAReplySaysHowManyMessagesDidntFit() async {
        let model = ScriptedRuntime(steps: [ScriptedRuntime.answer(turn)])
        var long = request("Plan the garden")
        long.messages =
            (0..<30).map { index in
                ChatMessage(
                    id: UUID(), role: index % 2 == 0 ? .person : .assistant,
                    text: String(repeating: "Message \(index) ", count: 10), at: now)
            } + long.messages

        let (replies, error) = await replies(.live(runtime: { _ in model }), long)

        XCTAssertNil(error)
        XCTAssertGreaterThan(replies.last?.leftOut ?? 0, 0)
    }
}
