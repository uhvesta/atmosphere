import ComposableArchitecture
import Foundation
import LifrAPI
import LifrModelsClient

/// A loaded LiteRT-LM model. Each turn gets a conversation of its own.
package protocol LiteRTEngine: Sendable {
    /// A new conversation that follows `instructions`. `constrained` when
    /// its answers will be held to a JSON schema.
    func chat(instructions: String, constrained: Bool) async throws(ModelRuntimeError) -> any LiteRTChat
}

/// One conversation with a loaded LiteRT-LM model.
package protocol LiteRTChat: Sendable {
    /// What the model writes back to `message` and its `attachments`, held
    /// to the JSON schema `format` when there is one. `partial` gets the
    /// answer as far as it's written, each time it grows.
    func send(
        _ message: String, attachments: [ModelAttachment], format: String?,
        partial: @escaping @Sendable (String) -> Void
    ) async throws(ModelRuntimeError) -> String
    /// Stops the answer being written.
    func cancel()
}

/// How a turn runs on a LiteRT-LM model. LiteRT-LM's own tools are Swift
/// types known when Lifr is built, so Lifr describes the turn's tools in
/// the instructions instead, and holds each answer to a schema that's
/// either a call to one of them or the answer itself. Gemma follows it
/// because decoding can't leave the schema.
enum LiteRTTurn {
    /// Tool calls one turn may make before it must answer.
    static let toolRounds = 5

    /// What a turn has done so far: each tool call the model wrote, and
    /// what Lifr sent back. When the turn starts again, as after Lifr went
    /// to the background, it carries on from there rather than calling the
    /// tools again.
    final class Journal: Sendable {
        struct Round: Equatable, Sendable {
            /// The call, as the model wrote it.
            var written: String
            /// What Lifr sent back.
            var reply: String
        }

        private let recorded = LockIsolated<[Round]>([])

        var rounds: [Round] { recorded.value }

        func record(_ round: Round) {
            recorded.withValue { $0.append(round) }
        }
    }

    static func respond(
        to turn: ModelTurn, on engine: any LiteRTEngine, journal: Journal = Journal(),
        partial: @escaping @Sendable (JSONValue) -> Void
    ) async throws(ModelRuntimeError) -> JSONValue {
        let answer = turn.schema.map(JSONSchema.value)
        let chat = try await engine.chat(
            instructions: instructions(turn), constrained: answer != nil || !turn.tools.isEmpty)
        let result = await withTaskCancellationHandler {
            do throws(ModelRuntimeError) {
                return Result<JSONValue, ModelRuntimeError>.success(
                    try await answering(turn, answer: answer, chat: chat, journal: journal, partial: partial))
            } catch {
                return .failure(error)
            }
        } onCancel: {
            chat.cancel()
        }
        return try result.get()
    }

    private static func answering(
        _ turn: ModelTurn, answer: JSONSchema.Node?, chat: any LiteRTChat, journal: Journal,
        partial: @escaping @Sendable (JSONValue) -> Void
    ) async throws(ModelRuntimeError) -> JSONValue {
        guard turn.tools.isEmpty else {
            return try await withTools(turn, answer: answer, chat: chat, journal: journal)
        }
        guard let answer else {
            return .string(
                try await chat.send(turn.prompt, attachments: turn.attachments, format: nil) { partial(.string($0)) })
        }
        return try parse(
            await chat.send(turn.prompt, attachments: turn.attachments, format: JSONSchema.text(answer)) { _ in })
    }

    private static func withTools(_ turn: ModelTurn, answer: JSONSchema.Node?, chat: any LiteRTChat, journal: Journal)
        async throws(ModelRuntimeError) -> JSONValue
    {
        let calling = JSONSchema.text(step(tools: turn.tools, answer: answer))
        let answering = JSONSchema.text(step(tools: [], answer: answer))
        let done = journal.rounds
        var message = done.isEmpty ? turn.prompt : resumed(turn.prompt, after: done)
        // The photos and recordings go with the prompt; the conversation
        // keeps them for the tools' rounds.
        var attachments = turn.attachments
        for round in min(done.count, toolRounds)...toolRounds {
            let format = round == toolRounds ? answering : calling
            let text = try await chat.send(message, attachments: attachments, format: format) { _ in }
            let written = try parse(text)
            attachments = []
            switch try Step(written) {
            case .answer(let value):
                return answer == nil ? .string(value.text ?? "") : value
            case .call(let name, let arguments):
                guard let tool = turn.tools.first(where: { $0.name == name }) else { throw .decodingFailure }
                let result = await tool.call(arguments)
                message = "\(name) returned:\n\(result)"
                journal.record(Journal.Round(written: text, reply: message))
            }
        }
        throw .decodingFailure
    }

    /// The prompt of a turn starting again after `rounds`, in a new
    /// conversation that hasn't seen them.
    static func resumed(_ prompt: String, after rounds: [Journal.Round]) -> String {
        var parts = [
            prompt,
            "You started answering this before, and Lifr had to stop. Here is what you did; it's done and "
                + "saved, so don't do it again. Carry on from there.",
        ]
        for round in rounds {
            parts.append("You wrote: \(round.written)\n\(round.reply)")
        }
        return parts.joined(separator: "\n\n")
    }

    /// The turn's instructions, with its tools and schema spelled out.
    static func instructions(_ turn: ModelTurn) -> String {
        var parts = [turn.instructions]
        if !turn.tools.isEmpty {
            var lines = [
                "Before you answer, you can call a tool by writing {\"tool\": name, \"arguments\": {…}}. Lifr "
                    + "sends back what it returns. When you have what you need, write {\"answer\": …}. Tools:"
            ]
            for tool in turn.tools {
                let arguments = tool.parameters.map { $0.isOptional ? "\($0.name)?" : $0.name }
                lines.append("- \(tool.name)(\(arguments.joined(separator: ", "))): \(tool.description)")
            }
            parts.append(lines.joined(separator: "\n"))
        }
        if let schema = turn.schema, turn.schemaInPrompt {
            let shape = JSONSchema.text(JSONSchema.value(schema))
            parts.append(
                turn.tools.isEmpty
                    ? "Answer with JSON that fits this schema: \(shape)"
                    : "The answer is JSON that fits this schema: \(shape)")
        }
        return parts.joined(separator: "\n\n")
    }

    /// The schema of one answer when the turn has tools: a call to one of
    /// `tools`, or the answer.
    static func step(tools: [ModelTool], answer: JSONSchema.Node?) -> JSONSchema.Node {
        var choices = tools.map { tool in
            JSONSchema.object([
                ("tool", .value(["const": .string(tool.name)])),
                ("arguments", JSONSchema.value(.object(name: tool.name, tool.parameters))),
            ])
        }
        choices.append(JSONSchema.object([("answer", answer ?? .value(["type": "string"]))]))
        if choices.count == 1, let only = choices.first { return only }
        return .object([.init(name: "anyOf", node: .array(choices))])
    }

    private static func parse(_ text: String) throws(ModelRuntimeError) -> JSONValue {
        guard let value = try? JSONDecoder().decode(JSONValue.self, from: Data(text.utf8)) else {
            throw .decodingFailure
        }
        return value
    }

    /// One answer in a turn with tools.
    enum Step: Equatable {
        case call(String, arguments: JSONValue)
        case answer(JSONValue)

        init(_ value: JSONValue) throws(ModelRuntimeError) {
            guard case .object(let fields) = value else { throw .decodingFailure }
            if let answer = fields["answer"] {
                self = .answer(answer)
            } else if case .string(let name)? = fields["tool"] {
                self = .call(name, arguments: fields["arguments"] ?? .object([:]))
            } else {
                throw .decodingFailure
            }
        }
    }
}

/// The JSON Schema LiteRT-LM holds an answer to, for a `ModelSchema`.
/// The decoder writes an object's properties in the order the schema
/// lists them, so objects keep the order they're declared in: a task's
/// title and place come before the fields that are only sometimes filled.
enum JSONSchema {
    /// JSON whose objects keep their keys in order.
    indirect enum Node: Equatable, Sendable {
        case value(JSONValue)
        case array([Node])
        case object([Field])

        struct Field: Equatable, Sendable {
            var name: String
            var node: Node
        }

        /// The same JSON, without the order.
        var json: JSONValue {
            switch self {
            case .value(let value): value
            case .array(let items): .array(items.map(\.json))
            case .object(let fields):
                .object(Dictionary(fields.map { ($0.name, $0.node.json) }, uniquingKeysWith: { first, _ in first }))
            }
        }
    }

    static func value(_ schema: ModelSchema) -> Node {
        switch schema {
        case .string: .value(["type": "string"])
        case .bool: .value(["type": "boolean"])
        case .choice(_, let choices): .value(["type": "string", "enum": .array(choices.map(JSONValue.string))])
        case .array(let item):
            .object([.init(name: "type", node: .value("array")), .init(name: "items", node: value(item))])
        case .object(_, let properties):
            object(
                properties.map { property in
                    var field = value(property.schema)
                    if !property.description.isEmpty {
                        field = described(field, property.description)
                    }
                    return (property.name, field)
                },
                required: properties.filter { !$0.isOptional }.map(\.name))
        }
    }

    private static func described(_ node: Node, _ description: String) -> Node {
        switch node {
        case .value(.object(var keys)):
            keys["description"] = .string(description)
            return .value(.object(keys))
        case .object(let fields):
            return .object(fields + [.init(name: "description", node: .value(.string(description)))])
        case .value, .array:
            return node
        }
    }

    /// An object with `fields` in order, all required unless `required`
    /// says which.
    static func object(_ fields: [(String, Node)], required: [String]? = nil) -> Node {
        var seen = Set<String>()
        let properties = fields.filter { seen.insert($0.0).inserted }.map { Node.Field(name: $0.0, node: $0.1) }
        return .object([
            .init(name: "type", node: .value("object")),
            .init(name: "properties", node: .object(properties)),
            .init(name: "required", node: .value(.array((required ?? fields.map(\.0)).map(JSONValue.string)))),
            .init(name: "additionalProperties", node: .value(.bool(false))),
        ])
    }

    static func text(_ node: Node) -> String {
        switch node {
        case .value(let value): encoded(value)
        case .array(let items): "[" + items.map(text).joined(separator: ",") + "]"
        case .object(let fields):
            "{" + fields.map { encoded(.string($0.name)) + ":" + text($0.node) }.joined(separator: ",") + "}"
        }
    }

    private static func encoded(_ value: JSONValue) -> String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
        guard let data = try? encoder.encode(value), let text = String(bytes: data, encoding: .utf8) else {
            return "{}"
        }
        return text
    }
}

extension JSONValue {
    fileprivate var text: String? {
        if case .string(let text) = self { text } else { nil }
    }
}
