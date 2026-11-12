import Foundation
import JSONKit

/// Measures whether a model calls tools well enough to work as an agent: it
/// is asked, over an OpenAI-compatible server as an agent would ask, to do
/// small jobs that each take one of its tools, and must call that tool with
/// the arguments the job needs every time.
public enum ToolProbe {
    public enum Verdict: String, Sendable, Codable {
        /// It called the right tool, with the right arguments, every time.
        case good
        /// It answered in words, picked the wrong tool or got the arguments
        /// wrong at least once.
        case poor
    }

    /// Asks `model` on the server at `base`, such as
    /// `http://127.0.0.1:47830/v1`. Nil when it couldn't be asked, such as
    /// when the server isn't running or doesn't have the model; ask again
    /// later then.
    public static func run(base: URL, model: String, session: URLSession = .shared) async -> Verdict? {
        for job in jobs {
            guard let reply = await ask(base: base, model: model, job: job, session: session) else { return nil }
            guard job.done(by: reply) else { return .poor }
        }
        return .good
    }

    /// One small job and what doing it takes.
    struct Job {
        var prompt: String
        var tool: String
        /// The arguments' fields that must be strings, and what each must
        /// contain, ignoring case.
        var arguments: [String: String]

        func done(by reply: JSONValue) -> Bool {
            guard field(reply, "choices", "0", "finish_reason") == .string("tool_calls"),
                case .array(let calls)? = field(reply, "choices", "0", "message", "tool_calls"),
                calls.count == 1,
                field(calls[0], "function", "name") == .string(tool),
                case .string(let text)? = field(calls[0], "function", "arguments"),
                case .object(let given)? = try? Wire.decoder.decode(JSONValue.self, from: Data(text.utf8))
            else { return false }
            return arguments.allSatisfy { name, wanted in
                guard case .string(let value)? = given[name] else { return false }
                return value.localizedCaseInsensitiveContains(wanted)
            }
        }
    }

    static let jobs = [
        Job(prompt: "Show me what's in README.md.", tool: "read_file", arguments: ["path": "README.md"]),
        Job(
            prompt: "Create a file named notes.txt that says hello.", tool: "write_file",
            arguments: ["path": "notes.txt", "contents": "hello"]),
        Job(prompt: "Run the tests with `make test`.", tool: "run_command", arguments: ["command": "make test"]),
    ]

    /// The tools every job offers, so picking the right one is part of it.
    private static let tools: JSONValue = [
        function(
            "read_file", "Reads a file in the project and returns its contents.",
            ["path": "The file's path, relative to the project."]),
        function(
            "write_file", "Writes a file in the project, replacing it if it's there.",
            ["path": "The file's path, relative to the project.", "contents": "What the file holds."]),
        function(
            "run_command", "Runs a shell command in the project and returns its output.",
            ["command": "The command line."]),
    ]

    private static func function(_ name: String, _ description: String, _ fields: [String: String]) -> JSONValue {
        [
            "type": "function",
            "function": [
                "name": .string(name), "description": .string(description),
                "parameters": [
                    "type": "object",
                    "required": .array(fields.keys.sorted().map(JSONValue.string)),
                    "properties": .object(
                        fields.mapValues { ["type": "string", "description": .string($0)] }),
                ],
            ],
        ]
    }

    private static func ask(base: URL, model: String, job: Job, session: URLSession) async -> JSONValue? {
        let body: JSONValue = [
            "model": .string(model),
            "messages": [
                [
                    "role": "system",
                    "content": "You work in a software project. Use a tool to do what's asked; don't answer in words.",
                ],
                ["role": "user", "content": .string(job.prompt)],
            ],
            "tools": tools,
        ]
        var request = URLRequest(url: base.appending(path: "chat/completions"))
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.timeoutInterval = 600
        guard let encoded = try? Wire.encoder.encode(body) else { return nil }
        request.httpBody = encoded
        guard let (data, response) = try? await session.data(for: request),
            (response as? HTTPURLResponse)?.statusCode == 200
        else { return nil }
        return try? Wire.decoder.decode(JSONValue.self, from: data)
    }

    private static func field(_ value: JSONValue?, _ path: String...) -> JSONValue? {
        path.reduce(value) { value, key in
            switch value {
            case .object(let fields)?: fields[key]
            case .array(let items)?: Int(key).flatMap { items.indices.contains($0) ? items[$0] : nil }
            default: nil
            }
        }
    }
}
