import Foundation
import InferenceKit
import InferenceServerKit
import JSONKit
import XCTest

final class ModelServerTests: XCTestCase {
    private var server: ModelServer?

    override func tearDown() {
        server?.stop()
        super.tearDown()
    }

    /// Starts a server for `models`, and returns its base URL.
    private func serve(_ models: [ServedModel]) async throws -> URL {
        let server = ModelServer { models }
        self.server = server
        let port = try await server.start()
        return try XCTUnwrap(URL(string: "http://127.0.0.1:\(port)/v1"))
    }

    private func post(_ base: URL, _ body: JSONValue) async throws -> (Int, Data) {
        var request = URLRequest(url: base.appending(path: "chat/completions"))
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try Wire.encoder.encode(body)
        let (data, response) = try await URLSession.shared.data(for: request)
        return ((response as? HTTPURLResponse)?.statusCode ?? 0, data)
    }

    /// The JSON of each `data:` event of a streamed reply, up to [DONE].
    private func events(_ data: Data) throws -> (events: [JSONValue], done: Bool) {
        var events: [JSONValue] = []
        for line in String(decoding: data, as: UTF8.self).components(separatedBy: "\n") {
            guard line.hasPrefix("data: ") else { continue }
            let payload = line.dropFirst(6)
            if payload == "[DONE]" { return (events, true) }
            events.append(try Wire.decoder.decode(JSONValue.self, from: Data(payload.utf8)))
        }
        return (events, false)
    }

    private func field(_ value: JSONValue?, _ path: String...) -> JSONValue? {
        path.reduce(value) { value, key in
            switch value {
            case .object(let fields)?: fields[key]
            case .array(let items)?: Int(key).flatMap { items.indices.contains($0) ? items[$0] : nil }
            default: nil
            }
        }
    }

    private func user(_ text: String) -> JSONValue { ["role": "user", "content": .string(text)] }

    func testItListsItsModels() async throws {
        let base = try await serve([
            ServedModel(id: "apple", runtime: ScriptedRuntime(steps: [])),
            ServedModel(id: "qwen", runtime: ScriptedRuntime(steps: [])),
        ])
        let (data, _) = try await URLSession.shared.data(from: base.appending(path: "models"))
        let listed = try Wire.decoder.decode(JSONValue.self, from: data)
        guard case .array(let models)? = field(listed, "data") else { return XCTFail("no models in \(listed)") }
        XCTAssertEqual(models.compactMap { field($0, "id") }, ["apple", "qwen"])
    }

    func testAnAnswerComesWhole() async throws {
        let base = try await serve([
            ServedModel(id: "m", runtime: ScriptedRuntime(steps: [ScriptedRuntime.answer("Hello")]))
        ])
        let (status, data) = try await post(base, ["model": "m", "messages": [user("Hi")]])
        XCTAssertEqual(status, 200)
        let reply = try Wire.decoder.decode(JSONValue.self, from: data)
        XCTAssertEqual(field(reply, "choices", "0", "message", "content"), "Hello")
        XCTAssertEqual(field(reply, "choices", "0", "finish_reason"), "stop")
    }

    func testAnAnswerStreamsAsItGrows() async throws {
        let runtime = ScriptedRuntime(steps: [
            ScriptedRuntime.answer("Hello there", partials: ["Hel", "Hello", "Hello th"])
        ])
        let base = try await serve([ServedModel(id: "m", runtime: runtime)])
        let (status, data) = try await post(
            base,
            [
                "model": "m", "messages": [user("Hi")], "stream": true, "stream_options": ["include_usage": true],
            ])
        XCTAssertEqual(status, 200)
        let (events, done) = try events(data)
        XCTAssertTrue(done)
        let deltas = events.compactMap { field($0, "choices", "0", "delta", "content") }
        XCTAssertGreaterThan(deltas.count, 1, "the answer came in one piece")
        XCTAssertEqual(deltas.map { if case .string(let text) = $0 { text } else { "" } }.joined(), "Hello there")
        XCTAssertEqual(
            events.compactMap { field($0, "choices", "0", "finish_reason") }.filter { $0 != .null }, ["stop"])
    }

    func testAToolCallEndsTheReplyForTheClientToRun() async throws {
        let runtime = ScriptedRuntime(steps: [
            ScriptedRuntime.call(
                "write_file", ["path": "notes.md", "contents": "# Notes"], then: ScriptedRuntime.answer("I wrote it"))
        ])
        let base = try await serve([ServedModel(id: "m", runtime: runtime)])
        let tools: JSONValue = [
            [
                "type": "function",
                "function": [
                    "name": "write_file", "description": "Writes a file.",
                    "parameters": [
                        "type": "object", "required": ["path", "contents"],
                        "properties": ["path": ["type": "string"], "contents": ["type": "string"]],
                    ],
                ],
            ]
        ]
        let (_, data) = try await post(
            base, ["model": "m", "messages": [user("Write notes")], "tools": tools, "stream": true])
        let (events, done) = try events(data)
        XCTAssertTrue(done)
        let call = events.compactMap { field($0, "choices", "0", "delta", "tool_calls", "0", "function") }.first
        XCTAssertEqual(field(call, "name"), "write_file")
        guard case .string(let arguments)? = field(call, "arguments") else { return XCTFail("no arguments") }
        XCTAssertEqual(
            try Wire.decoder.decode(JSONValue.self, from: Data(arguments.utf8)),
            ["path": "notes.md", "contents": "# Notes"])
        XCTAssertEqual(
            events.compactMap { field($0, "choices", "0", "finish_reason") }.filter { $0 != .null }, ["tool_calls"])
        // What the model would have said without the tool's result isn't sent.
        XCTAssertTrue(events.allSatisfy { field($0, "choices", "0", "delta", "content") == nil })
        XCTAssertEqual(runtime.turns.first?.tools.first?.parameters.map(\.name), ["path", "contents"])
    }

    func testTheModelReadsTheInstructionsAndWhatItsToolsReturned() async throws {
        let runtime = ScriptedRuntime(steps: [ScriptedRuntime.answer("Done")])
        let base = try await serve([ServedModel(id: "m", runtime: runtime)])
        let messages: JSONValue = [
            ["role": "system", "content": "You are a coding agent."],
            user("Write notes"),
            [
                "role": "assistant", "content": .null,
                "tool_calls": [
                    ["id": "c1", "type": "function", "function": ["name": "write_file", "arguments": "{}"]]
                ],
            ],
            ["role": "tool", "tool_call_id": "c1", "content": "wrote notes.md"],
        ]
        let (status, _) = try await post(base, ["model": "m", "messages": messages])
        XCTAssertEqual(status, 200)
        let turn = try XCTUnwrap(runtime.turns.first)
        XCTAssertEqual(turn.instructions, "You are a coding agent.")
        XCTAssertTrue(turn.prompt.contains("Write notes"))
        XCTAssertTrue(turn.prompt.contains("wrote notes.md"))
    }

    func testAnImageSentAsADataURLReachesTheModel() async throws {
        let seen = Locked<Data?>(nil)
        let look: ScriptedRuntime.Step = { turn, _ throws(ModelRuntimeError) in
            seen.setValue(turn.attachments.first.flatMap { try? Data(contentsOf: $0.file) })
            return "A cat"
        }
        let runtime = ScriptedRuntime(inputs: [.image], steps: [look])
        let base = try await serve([ServedModel(id: "m", runtime: runtime)])
        let image = Data([1, 2, 3])
        let message: JSONValue = [
            "role": "user",
            "content": [
                [
                    "type": "image_url",
                    "image_url": ["url": .string("data:image/png;base64,\(image.base64EncodedString())")],
                ],
                ["type": "text", "text": "What is this?"],
            ],
        ]
        let (status, _) = try await post(base, ["model": "m", "messages": [message]])
        XCTAssertEqual(status, 200)
        XCTAssertEqual(seen.value, image)
        XCTAssertEqual(runtime.turns.first?.attachments.map(\.kind), [.image])
        XCTAssertEqual(runtime.turns.first?.prompt, "What is this?")
    }

    func testTheListSaysWhichModelsReadImages() async throws {
        let base = try await serve([
            ServedModel(id: "sees", runtime: ScriptedRuntime(inputs: [.image], steps: [])),
            ServedModel(id: "text", runtime: ScriptedRuntime(steps: [])),
        ])
        let (data, _) = try await URLSession.shared.data(from: base.appending(path: "models"))
        let list = try Wire.decoder.decode(JSONValue.self, from: data)
        XCTAssertEqual(field(list, "data", "0", "inputs"), ["image"])
        XCTAssertNil(field(list, "data", "1", "inputs"))
    }

    func testTheListSaysHowMuchEachModelReads() async throws {
        let wide = ContextBudget(window: 32_768, reservedForAnswer: 4_096)
        let base = try await serve([
            ServedModel(id: "wide", runtime: ScriptedRuntime(budget: wide, steps: [])),
            ServedModel(id: "small", runtime: ScriptedRuntime(steps: [])),
        ])
        let (data, _) = try await URLSession.shared.data(from: base.appending(path: "models"))
        let list = try Wire.decoder.decode(JSONValue.self, from: data)
        XCTAssertEqual(field(list, "data", "0", "context_length"), .number(32_768))
        XCTAssertEqual(
            field(list, "data", "1", "context_length"), .number(Double(ContextBudget.appleOnDevice.window)))
    }

    func testAModelItDoesNotHaveIsNotFound() async throws {
        let base = try await serve([ServedModel(id: "m", runtime: ScriptedRuntime(steps: []))])
        let (status, data) = try await post(base, ["model": "other", "messages": [user("Hi")], "stream": true])
        XCTAssertEqual(status, 404)
        XCTAssertNotNil(field(try Wire.decoder.decode(JSONValue.self, from: data), "error", "message"))
    }

    func testAModelThatCannotRunSaysSoWithAStatus() async throws {
        let base = try await serve([ServedModel(id: "m", runtime: ScriptedRuntime(status: .notDownloaded, steps: []))])
        let (status, _) = try await post(base, ["model": "m", "messages": [user("Hi")], "stream": true])
        XCTAssertEqual(status, 503)
    }
}
