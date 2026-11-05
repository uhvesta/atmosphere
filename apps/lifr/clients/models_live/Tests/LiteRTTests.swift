import ComposableArchitecture
import Foundation
import LifrAPI
import LifrModelsClient
import XCTest

@testable import LifrModelsLive

/// A conversation that writes what `reply` says for each message.
final class FakeChat: LiteRTChat, @unchecked Sendable {
    struct Sent: Equatable {
        var message: String
        var format: String?
        var attachments: [ModelAttachment] = []
    }

    let sent = LockIsolated<[Sent]>([])
    let cancelled = LockIsolated(false)
    private let reply: @Sendable (_ message: String, _ format: String?) async throws(ModelRuntimeError) -> String

    init(reply: @escaping @Sendable (_ message: String, _ format: String?) async throws(ModelRuntimeError) -> String) {
        self.reply = reply
    }

    convenience init(_ replies: [String]) {
        let next = LockIsolated(replies)
        self.init { _, _ throws(ModelRuntimeError) in
            next.withValue { $0.isEmpty ? "" : $0.removeFirst() }
        }
    }

    func send(
        _ message: String, attachments: [ModelAttachment], format: String?,
        partial: @escaping @Sendable (String) -> Void
    ) async throws(ModelRuntimeError) -> String {
        sent.withValue { $0.append(Sent(message: message, format: format, attachments: attachments)) }
        let written = try await reply(message, format)
        partial(written)
        return written
    }

    func cancel() { cancelled.setValue(true) }
}

struct FakeEngine: LiteRTEngine {
    let chat: FakeChat
    let instructions = LockIsolated<[String]>([])

    func chat(instructions: String, constrained: Bool) async throws(ModelRuntimeError) -> any LiteRTChat {
        self.instructions.withValue { $0.append(instructions) }
        return chat
    }
}

final class LiteRTTurnTests: XCTestCase {
    private let reply = ModelSchema.object(name: "Reply", [ModelProperty("text", "What to say.")])

    func testAPlainTurnIsTheTextWritten() async throws {
        let chat = FakeChat(["Hello there"])
        let partials = LockIsolated<[JSONValue]>([])

        let answer = try await LiteRTTurn.respond(
            to: ModelTurn(instructions: "Be brief.", prompt: "Hi"), on: FakeEngine(chat: chat)
        ) { partial in partials.withValue { $0.append(partial) } }

        XCTAssertEqual(answer, .string("Hello there"))
        XCTAssertEqual(partials.value.last, .string("Hello there"))
        XCTAssertEqual(chat.sent.value.map(\.format), [nil])
    }

    func testAnAnswerWithASchemaIsHeldToIt() async throws {
        let chat = FakeChat([#"{"text":"Done"}"#])

        let answer = try await LiteRTTurn.respond(
            to: ModelTurn(instructions: "", prompt: "Hi", schema: reply), on: FakeEngine(chat: chat)
        ) { _ in }

        XCTAssertEqual(answer, .object(["text": .string("Done")]))
        let format = try XCTUnwrap(chat.sent.value.first?.format)
        let schema = try JSONDecoder().decode(JSONValue.self, from: Data(format.utf8))
        XCTAssertEqual(schema, JSONSchema.value(reply))
    }

    func testAToolCallRunsTheToolAndSendsBackItsResult() async throws {
        let calls = LockIsolated<[JSONValue]>([])
        let parameters = [ModelProperty("code", "")]
        let tool = ModelTool(name: "getTask", description: "One task.", parameters: parameters) { arguments in
            calls.withValue { $0.append(arguments) }
            return "HOME-0001 Buy milk"
        }
        let chat = FakeChat([
            #"{"tool":"getTask","arguments":{"code":"HOME-0001"}}"#,
            #"{"answer":{"text":"It's Buy milk."}}"#,
        ])

        let answer = try await LiteRTTurn.respond(
            to: ModelTurn(instructions: "", prompt: "What's HOME-0001?", tools: [tool], schema: reply),
            on: FakeEngine(chat: chat)
        ) { _ in }

        XCTAssertEqual(answer, .object(["text": .string("It's Buy milk.")]))
        XCTAssertEqual(calls.value, [.object(["code": .string("HOME-0001")])])
        XCTAssertEqual(chat.sent.value.count, 2)
        XCTAssertTrue(chat.sent.value[1].message.contains("HOME-0001 Buy milk"))
    }

    func testPhotosAndRecordingsGoWithThePromptOnly() async throws {
        let tool = ModelTool(name: "overview", description: "", parameters: []) { _ in "Nothing's due." }
        let chat = FakeChat([#"{"tool":"overview","arguments":{}}"#, #"{"answer":"A receipt."}"#])
        let attachments = [
            ModelAttachment(.image, file: URL(filePath: "/a.jpg")),
            ModelAttachment(.audio, file: URL(filePath: "/b.wav")),
        ]

        _ = try await LiteRTTurn.respond(
            to: ModelTurn(instructions: "", prompt: "What's this?", tools: [tool], attachments: attachments),
            on: FakeEngine(chat: chat)
        ) { _ in }

        XCTAssertEqual(chat.sent.value.map(\.attachments), [attachments, []])
    }

    func testAPlainAnswerWithToolsIsItsText() async throws {
        let tool = ModelTool(name: "overview", description: "", parameters: []) { _ in "" }
        let chat = FakeChat([#"{"answer":"Nothing's due."}"#])

        let answer = try await LiteRTTurn.respond(
            to: ModelTurn(instructions: "", prompt: "Anything due?", tools: [tool]), on: FakeEngine(chat: chat)
        ) { _ in }

        XCTAssertEqual(answer, .string("Nothing's due."))
    }

    func testAModelThatKeepsCallingToolsMustAnswerInTheEnd() async throws {
        let calls = LockIsolated(0)
        let tool = ModelTool(name: "overview", description: "", parameters: []) { _ in
            calls.withValue { $0 += 1 }
            return "Nothing's due."
        }
        // Decoding can't leave the schema, so once tools are left out of
        // it the model answers.
        let chat = FakeChat { _, format throws(ModelRuntimeError) in
            (format ?? "").contains("\"tool\"") ? #"{"tool":"overview","arguments":{}}"# : #"{"answer":"Done."}"#
        }

        let answer = try await LiteRTTurn.respond(
            to: ModelTurn(instructions: "", prompt: "Hi", tools: [tool]), on: FakeEngine(chat: chat)
        ) { _ in }

        XCTAssertEqual(answer, .string("Done."))
        XCTAssertEqual(calls.value, LiteRTTurn.toolRounds)
    }

    func testAnAnswerThatIsntJSONOrCallsAnUnknownToolCantBeRead() async {
        let tool = ModelTool(name: "overview", description: "", parameters: []) { _ in "" }
        for written in ["Sure! Here it is", #"{"tool":"deleteEverything","arguments":{}}"#, #"{"other":1}"#] {
            do {
                _ = try await LiteRTTurn.respond(
                    to: ModelTurn(instructions: "", prompt: "Hi", tools: [tool], schema: reply),
                    on: FakeEngine(chat: FakeChat([written]))
                ) { _ in }
                XCTFail("\(written) was read")
            } catch {
                XCTAssertEqual(error, .decodingFailure, written)
            }
        }
    }

    func testTheInstructionsNameEachTool() {
        let tools = [
            ModelTool(name: "findTasks", description: "Search.", parameters: []) { _ in "" },
            ModelTool(name: "revert", description: "Undo.", parameters: []) { _ in "" },
        ]

        let instructions = LiteRTTurn.instructions(ModelTurn(instructions: "Be brief.", prompt: "", tools: tools))

        XCTAssertTrue(instructions.hasPrefix("Be brief."))
        XCTAssertTrue(instructions.contains("findTasks"))
        XCTAssertTrue(instructions.contains("revert"))
    }

    func testOptionalFieldsArentRequired() {
        let schema = ModelSchema.object(
            name: "Find",
            [
                ModelProperty("words", ""),
                ModelProperty("status", "", .choice(name: "Status", ["todo", "done"]), optional: true),
            ])

        guard case .object(let fields) = JSONSchema.value(schema) else { return XCTFail("not an object") }

        XCTAssertEqual(fields["required"], .array([.string("words")]))
        guard case .object(let properties)? = fields["properties"], case .object(let status)? = properties["status"]
        else { return XCTFail("no status") }
        XCTAssertEqual(status["enum"], .array([.string("todo"), .string("done")]))
    }
}

final class ModelHostTests: XCTestCase {
    private func model(_ id: ModelID, memory: Int64 = 1_000_000_000) -> InstalledModel {
        InstalledModel(
            spec: ModelSpec(
                id: id, name: id.rawValue, summary: "", runtime: .liteRT,
                source: ModelSource(
                    repo: "org/model", commit: String(repeating: "a", count: 40),
                    files: [ModelFile(path: "m.litertlm", size: 10, digest: .sha256(String(repeating: "b", count: 64)))]
                ), contextTokens: 8_192, memoryBytes: memory, license: ""),
            directory: URL(filePath: "/nonexistent"), installedAt: Date(timeIntervalSince1970: 0))
    }

    private func host(
        loads: LockIsolated<[ModelID]> = LockIsolated([]), chat: FakeChat = FakeChat(["hi", "hi", "hi"]),
        memory: Int64 = 8_000_000_000,
        sleep: @escaping @Sendable (Duration) async throws -> Void = { _ in
            try await Task.sleep(for: .seconds(3_600))
        }
    ) -> ModelHost {
        ModelHost(
            load: { model throws(ModelRuntimeError) in
                loads.withValue { $0.append(model.id) }
                return FakeEngine(chat: chat)
            }, memory: { memory }, sleep: sleep)
    }

    private func plain(_ engine: any LiteRTEngine) async throws(ModelRuntimeError) -> JSONValue {
        try await LiteRTTurn.respond(to: ModelTurn(instructions: "", prompt: "Hi"), on: engine) { _ in }
    }

    func testAModelLoadsOnceForEveryTurn() async throws {
        let loads = LockIsolated<[ModelID]>([])
        let host = host(loads: loads)

        _ = try await host.run(model("a"), plain)
        _ = try await host.run(model("a"), plain)

        XCTAssertEqual(loads.value, ["a"])
    }

    func testAnotherModelTakesTheLoadedOnesPlace() async throws {
        let loads = LockIsolated<[ModelID]>([])
        let host = host(loads: loads)

        _ = try await host.run(model("a"), plain)
        _ = try await host.run(model("b"), plain)

        XCTAssertEqual(loads.value, ["a", "b"])
        let loaded = await host.loadedModel
        XCTAssertEqual(loaded, "b")
    }

    func testAModelThatDoesntFitIsntLoaded() async {
        let loads = LockIsolated<[ModelID]>([])
        let host = host(loads: loads, memory: 3_000_000_000)

        do {
            _ = try await host.run(model("a", memory: 2_600_000_000), plain)
            XCTFail("it loaded")
        } catch {
            XCTAssertEqual(error, .unavailable(.notEnoughMemory(needed: 2_600_000_000, available: 3_000_000_000)))
        }
        XCTAssertEqual(loads.value, [])
    }

    func testRunningLowOnMemoryStopsTheTurn() async throws {
        let started = AsyncStream.makeStream(of: Void.self)
        let chat = FakeChat { _, _ throws(ModelRuntimeError) in
            started.continuation.yield()
            do {
                try await Task.sleep(for: .seconds(60))
            } catch {
                throw .cancelled
            }
            return "too late"
        }
        let host = host(chat: chat)
        let model = model("a")

        let turn = Task { try await host.run(model, plain) }
        var events = started.stream.makeAsyncIterator()
        _ = await events.next()
        await host.unload(.memory)

        guard case .failure(let error) = await turn.result else { return XCTFail("it answered") }
        XCTAssertEqual(error as? ModelRuntimeError, .interrupted(.memory))
        XCTAssertTrue(chat.cancelled.value)
        let loaded = await host.loadedModel
        XCTAssertNil(loaded)
    }

    func testAnIdleModelIsUnloaded() async throws {
        let wake = AsyncStream.makeStream(of: Void.self)
        let host = host { _ in
            var iterator = wake.stream.makeAsyncIterator()
            _ = await iterator.next()
        }

        _ = try await host.run(model("a"), plain)
        let before = await host.loadedModel
        XCTAssertEqual(before, "a")
        wake.continuation.yield()

        for _ in 0..<200 {
            if await host.loadedModel == nil { break }
            try await Task.sleep(for: .milliseconds(10))
        }
        let after = await host.loadedModel
        XCTAssertNil(after)
    }

    func testDeletingTheLoadedModelUnloadsIt() async throws {
        let host = host()

        _ = try await host.run(model("a"), plain)
        await host.unload(model: "b")
        let kept = await host.loadedModel
        await host.unload(model: "a")
        let unloaded = await host.loadedModel

        XCTAssertEqual(kept, "a")
        XCTAssertNil(unloaded)
    }

    func testAModelFitsWithHeadroomToSpare() {
        XCTAssertTrue(MemoryPlan.fits(1_450_000_000, available: 2_000_000_000))
        XCTAssertFalse(MemoryPlan.fits(1_600_000_000, available: 2_000_000_000))
        XCTAssertFalse(MemoryPlan.fits(9_100_000_000, available: 10_000_000_000))
        XCTAssertTrue(MemoryPlan.fits(8_900_000_000, available: 10_000_000_000))
    }
}

final class LiteRTRuntimeTests: XCTestCase {
    func testAModelWhoseFilesAreGoneIsntDownloaded() {
        let spec = ModelCatalog.gemma4E2B
        let runtime = LiteRTRuntime(
            model: InstalledModel(spec: spec, directory: URL(filePath: "/nonexistent"), installedAt: Date()),
            host: ModelHost(load: { _ throws(ModelRuntimeError) in throw .failed("no") }))

        XCTAssertEqual(runtime.status, .notDownloaded)
    }

    func testABiggerWindowKeepsMoreForTheAnswer() {
        let runtime = LiteRTRuntime(
            model: InstalledModel(spec: ModelCatalog.gemma4E2B, directory: URL(filePath: "/"), installedAt: Date()),
            host: ModelHost(load: { _ throws(ModelRuntimeError) in throw .failed("no") }))

        XCTAssertEqual(runtime.budget.window, 8_192)
        XCTAssertGreaterThan(runtime.budget.reservedForAnswer, ContextBudget.appleOnDevice.reservedForAnswer)
        XCTAssertGreaterThan(runtime.budget.room, ContextBudget.appleOnDevice.room)
    }

    func testOnlyAModelThatReadsPhotosGetsThem() async throws {
        let root = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let host = ModelHost(
            load: { _ throws(ModelRuntimeError) in FakeEngine(chat: FakeChat(["Done"])) },
            memory: { 16_000_000_000 }, sleep: { _ in try await Task.sleep(for: .seconds(3_600)) })
        let photo = ModelTurn(
            instructions: "", prompt: "What's this?",
            attachments: [ModelAttachment(.image, file: URL(filePath: "/a.jpg"))])
        for spec in [ModelCatalog.gemma4E2B, ModelCatalog.gemma4E4B] {
            let directory = root.appending(path: spec.id.directoryName)
            for file in spec.source?.files ?? [] {
                let url = directory.appending(path: file.path)
                try FileManager.default.createDirectory(
                    at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
                try Data().write(to: url)
            }
            let runtime = LiteRTRuntime(
                model: InstalledModel(spec: spec, directory: directory, installedAt: Date()), host: host)
            do {
                let answer = try await runtime.respond(to: photo) { _ in }
                XCTAssertEqual(spec.id, ModelCatalog.gemma4E4B.id, "\(spec.name) read a photo")
                XCTAssertEqual(answer, .string("Done"))
            } catch {
                XCTAssertEqual(spec.id, ModelCatalog.gemma4E2B.id, "\(spec.name) failed: \(error)")
                XCTAssertEqual(error, .unsupportedInput(.image))
            }
        }
    }

    func testAModelDownloadedBeforeInputsReadsWhatTheCatalogSays() throws {
        var old = ModelCatalog.gemma4E4B
        old.inputs = []
        let manifest = try JSONEncoder().encode(old)
        var json = try XCTUnwrap(JSONSerialization.jsonObject(with: manifest) as? [String: Any])
        json["inputs"] = nil
        let decoded = try JSONDecoder().decode(
            ModelSpec.self, from: JSONSerialization.data(withJSONObject: json))

        XCTAssertEqual(decoded.inputs, [])
        XCTAssertEqual(ModelCatalog.entry(for: decoded)?.inputs, [.image, .audio])
    }

    func testAPickedModelThatIsntDownloadedSaysSo() throws {
        let root = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let library = ModelLibrary(root: root, session: .shared)
        let runtimes = ModelRuntimes(
            library: library, host: ModelHost(load: { _ throws(ModelRuntimeError) in throw .failed("no") }))

        library.select(ModelCatalog.gemma4E2B.id, for: .assistant)

        XCTAssertEqual(runtimes.runtime(for: .assistant).status, .notDownloaded)
        XCTAssertTrue(runtimes.runtime(for: .titles) is AppleRuntime)
    }
}
