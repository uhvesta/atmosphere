import Foundation
import InferenceKit
import JSONKit

/// OpenAI's chat completions, as the server reads and writes them: the
/// request, the turn a runtime gets for it, and the reply.
enum ChatCompletions {
    struct Request: Decodable, Sendable {
        var model: String
        var messages: [Message]
        var tools: [Tool]?
        var stream: Bool?
        var streamOptions: StreamOptions?

        enum CodingKeys: String, CodingKey {
            case model, messages, tools, stream
            case streamOptions = "stream_options"
        }
    }

    struct StreamOptions: Decodable, Sendable {
        var includeUsage: Bool?

        enum CodingKeys: String, CodingKey {
            case includeUsage = "include_usage"
        }
    }

    struct Message: Decodable, Sendable {
        var role: String
        /// A string, or parts: text, and images as data URLs.
        var content: JSONValue?
        var toolCalls: [ToolCall]?
        var toolCallID: String?

        enum CodingKeys: String, CodingKey {
            case role, content
            case toolCalls = "tool_calls"
            case toolCallID = "tool_call_id"
        }

        var text: String {
            switch content {
            case .string(let text): text
            case .array(let parts):
                parts.compactMap { part -> String? in
                    guard case .object(let fields) = part, case .string(let text)? = fields["text"] else { return nil }
                    return text
                }.joined(separator: "\n")
            default: ""
            }
        }
    }

    struct ToolCall: Decodable, Sendable {
        var id: String?
        var function: Function
    }

    struct Function: Decodable, Sendable {
        var name: String
        /// JSON, as a string.
        var arguments: String?
    }

    struct Tool: Decodable, Sendable {
        var function: ToolFunction
    }

    struct ToolFunction: Decodable, Sendable {
        var name: String
        var description: String?
        /// A JSON schema of an object.
        var parameters: JSONValue?
    }

    /// What a turn came to: an answer, or the first tool the model called,
    /// which the client runs before it asks again.
    enum Outcome: Equatable, Sendable {
        case answer(String)
        case call(name: String, arguments: JSONValue)
    }

    /// The turn for `request`: the system messages as instructions, the rest
    /// as one transcript, and its tools, whose calls go to `called`.
    static func turn(
        for request: Request, called: @escaping @Sendable (_ name: String, _ arguments: JSONValue) async -> Void
    ) -> ModelTurn {
        let instructions = request.messages.filter { $0.role == "system" || $0.role == "developer" }
            .map(\.text).joined(separator: "\n\n")
        let tools = (request.tools ?? []).map { tool in
            let name = tool.function.name
            return ModelTool(
                name: name, description: tool.function.description ?? "",
                parameters: properties(of: tool.function.parameters)
            ) { arguments in
                await called(name, arguments)
                return ""
            }
        }
        return ModelTurn(
            instructions: instructions, prompt: prompt(request.messages), tools: tools, schema: nil,
            attachments: images(in: request.messages))
    }

    /// The images in the person's messages, written to files the model
    /// reads; an image that isn't a base64 data URL is skipped.
    static func images(in messages: [Message]) -> [ModelAttachment] {
        messages.filter { $0.role == "user" }.flatMap { message -> [ModelAttachment] in
            guard case .array(let parts)? = message.content else { return [] }
            return parts.compactMap { part -> ModelAttachment? in
                guard case .object(let fields) = part, case .string("image_url")? = fields["type"],
                    case .object(let image)? = fields["image_url"], case .string(let url)? = image["url"],
                    let file = write(dataURL: url)
                else { return nil }
                return ModelAttachment(.image, file: file)
            }
        }
    }

    /// Writes the image in a `data:image/png;base64,…` URL to a file of
    /// its own in the temporary folder.
    private static func write(dataURL url: String) -> URL? {
        guard url.hasPrefix("data:"), let comma = url.firstIndex(of: ",") else { return nil }
        let header = url[url.index(url.startIndex, offsetBy: 5)..<comma]
        guard header.hasSuffix(";base64"), let data = Data(base64Encoded: String(url[url.index(after: comma)...]))
        else { return nil }
        let type = header.dropLast(";base64".count)
        let ext = type == "image/jpeg" ? "jpg" : type == "image/gif" ? "gif" : type == "image/webp" ? "webp" : "png"
        let file = FileManager.default.temporaryDirectory.appending(path: "model-server-\(UUID().uuidString).\(ext)")
        guard (try? data.write(to: file)) != nil else { return nil }
        return file
    }

    /// The conversation after the instructions. A lone message from the
    /// person is the prompt as it is; anything longer is a transcript, so
    /// the model sees its own replies, its tool calls and what they
    /// returned.
    static func prompt(_ messages: [Message]) -> String {
        let conversation = messages.filter { $0.role != "system" && $0.role != "developer" }
        if conversation.count == 1, let only = conversation.first, only.role == "user" { return only.text }
        var names: [String: String] = [:]
        let lines = conversation.map { message -> String in
            switch message.role {
            case "assistant":
                let calls = (message.toolCalls ?? []).map { call in
                    if let id = call.id { names[id] = call.function.name }
                    return "You called \(call.function.name) with \(call.function.arguments ?? "{}")."
                }
                return (["You: \(message.text)"].filter { _ in !message.text.isEmpty } + calls)
                    .joined(separator: "\n")
            case "tool":
                let name = message.toolCallID.flatMap { names[$0] } ?? "The tool"
                return "\(name) returned:\n\(message.text)"
            default:
                return "Person: \(message.text)"
            }
        }
        return lines.joined(separator: "\n\n")
    }

    /// A tool's parameters, from its JSON schema: the required ones first
    /// in their order, then the rest by name.
    static func properties(of parameters: JSONValue?) -> [ModelProperty] {
        guard case .object(let root)? = parameters, case .object(let fields)? = root["properties"] else { return [] }
        let required: [String] =
            if case .array(let names)? = root["required"] {
                names.compactMap { if case .string(let name) = $0 { name } else { nil } }
            } else { [] }
        let order = required.filter { fields[$0] != nil } + fields.keys.filter { !required.contains($0) }.sorted()
        return order.compactMap { name -> ModelProperty? in
            guard let field = fields[name] else { return nil }
            let description =
                if case .object(let parts) = field, case .string(let text)? = parts["description"] {
                    text
                } else { "" }
            return ModelProperty(name, description, schema(of: field, named: name), optional: !required.contains(name))
        }
    }

    /// The nearest schema a runtime can enforce. Numbers are written as
    /// strings, which every runtime has; the client reads them back.
    static func schema(of value: JSONValue, named name: String) -> ModelSchema {
        guard case .object(let fields) = value else { return .string }
        if case .array(let choices)? = fields["enum"] {
            let names = choices.compactMap { if case .string(let choice) = $0 { choice } else { nil } }
            if !names.isEmpty { return .choice(name: name, names) }
        }
        switch fields["type"] {
        case .string("boolean"): return .bool
        case .string("array"):
            return .array(fields["items"].map { schema(of: $0, named: name) } ?? .string)
        case .string("object"):
            return .object(name: name, properties(of: value))
        default: return .string
        }
    }

    /// Runs `request` on `runtime`. `text` gets the answer as it grows,
    /// each time with what's new. A tool call stops the runtime, which the
    /// client is told to run the tool for.
    static func run(
        _ request: Request, on runtime: any ModelRuntime, text: @escaping @Sendable (String) -> Void
    ) async throws(ModelRuntimeError) -> (Outcome, TurnSpeed?) {
        let (calls, caller) = AsyncStream<Outcome>.makeStream()
        // Once the model calls a tool, nothing it writes after is sent.
        let called = Locked(false)
        var turn = turn(for: request) { name, arguments in
            called.setValue(true)
            caller.yield(.call(name: name, arguments: arguments))
            caller.finish()
            // Held until the call wins the race and the runtime is
            // cancelled, so the model doesn't go on without the result.
            while !Task.isCancelled { try? await Task.sleep(for: .seconds(60)) }
        }
        let files = turn.attachments.map(\.file)
        defer { for file in files { try? FileManager.default.removeItem(at: file) } }
        let speed = Locked<TurnSpeed?>(nil)
        turn.speed = { measured in speed.withValue { $0 = $0.map { $0.adding(measured) } ?? measured } }
        let written = Locked("")
        let partial: @Sendable (JSONValue) -> Void = { value in
            guard case .string(let answer) = value, !called.value else { return }
            let new = written.withValue { sent -> String? in
                guard answer.hasPrefix(sent), answer.count > sent.count else { return nil }
                defer { sent = answer }
                return String(answer.dropFirst(sent.count))
            }
            if let new { text(new) }
        }
        let request = turn
        let result: Result<Outcome, ModelRuntimeError> = await withTaskGroup(
            of: Result<Outcome, ModelRuntimeError>?.self
        ) { group in
            group.addTask {
                do throws(ModelRuntimeError) {
                    let answer = try await runtime.respond(to: request, partial: partial)
                    if case .string(let whole) = answer { partial(.string(whole)) }
                    return .success(.answer(answerText(answer)))
                } catch {
                    return .failure(error)
                }
            }
            group.addTask {
                for await call in calls { return .success(call) }
                return nil
            }
            var outcome: Result<Outcome, ModelRuntimeError> = .failure(.cancelled)
            for await finished in group {
                guard let finished else { continue }
                outcome = finished
                break
            }
            group.cancelAll()
            caller.finish()
            return outcome
        }
        return (try result.get(), speed.value)
    }

    /// The answer as text: as it is, or a structured one as its JSON.
    private static func answerText(_ answer: JSONValue) -> String {
        if case .string(let text) = answer { return text }
        guard let data = try? Wire.encoder.encode(answer) else { return "" }
        return String(decoding: data, as: UTF8.self)
    }
}
