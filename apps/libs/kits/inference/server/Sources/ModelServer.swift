import Foundation
import InferenceKit
import JSONKit
import Network

/// A model the server answers for, by the name clients ask for it by.
public struct ServedModel: Sendable {
    public var id: String
    public var runtime: any ModelRuntime
    /// What a client's model picker shows beside it, such as how well it
    /// calls tools; listed as `detail`, a field OpenAI's list doesn't have.
    public var detail: String?

    public init(id: String, runtime: any ModelRuntime, detail: String? = nil) {
        self.id = id
        self.runtime = runtime
        self.detail = detail
    }
}

/// Why the server didn't start.
public enum ModelServerError: Error, CustomStringConvertible {
    /// The port couldn't be listened on.
    case listen(port: UInt16, underlying: any Error)

    public var description: String {
        switch self {
        case .listen(let port, let underlying):
            "Couldn't listen on 127.0.0.1:\(port) (\(underlying)). Another program may have the port; quit it, or "
                + "start the server on another port."
        }
    }
}

/// An OpenAI-compatible server on loopback for the models `models` lists:
/// `GET /v1/models`, and `POST /v1/chat/completions`, streamed or not. A
/// model's tool call ends its reply with `finish_reason: tool_calls`, so the
/// client runs the tool and asks again with what it returned.
public final class ModelServer: Sendable {
    private let models: @Sendable () -> [ServedModel]
    private let listener = Locked<NWListener?>(nil)
    private let queue = DispatchQueue(label: "ModelServer")

    public init(models: @escaping @Sendable () -> [ServedModel]) {
        self.models = models
    }

    /// Listens on 127.0.0.1 at `port`, or a free one for 0, and returns the
    /// port. Only programs on this Mac reach it; a remote host does through
    /// a forward over SSH.
    public func start(port: UInt16 = 0) async throws(ModelServerError) -> UInt16 {
        let parameters = NWParameters.tcp
        parameters.allowLocalEndpointReuse = true
        parameters.requiredLocalEndpoint = .hostPort(
            host: .ipv4(.loopback), port: NWEndpoint.Port(rawValue: port) ?? .any)
        let made: NWListener
        do {
            made = try NWListener(using: parameters)
        } catch {
            throw .listen(port: port, underlying: error)
        }
        made.newConnectionHandler = { [weak self] connection in
            guard let self else { return connection.cancel() }
            connection.start(queue: queue)
            Task { await self.serve(connection) }
        }
        let ready: Result<UInt16, NWError> = await withCheckedContinuation { continuation in
            let resumed = Locked(false)
            made.stateUpdateHandler = { state in
                let result: Result<UInt16, NWError>? =
                    switch state {
                    case .ready: .success(made.port?.rawValue ?? port)
                    case .failed(let error), .waiting(let error): .failure(error)
                    default: nil
                    }
                guard let result,
                    resumed.withValue({ done in
                        defer { done = true }
                        return !done
                    })
                else { return }
                continuation.resume(returning: result)
            }
            made.start(queue: queue)
        }
        switch ready {
        case .success(let bound):
            listener.withValue {
                $0?.cancel()
                $0 = made
            }
            return bound
        case .failure(let error):
            made.cancel()
            throw .listen(port: port, underlying: error)
        }
    }

    /// Stops listening. Replies already streaming are cut off.
    public func stop() {
        listener.withValue {
            $0?.cancel()
            $0 = nil
        }
    }

    private func serve(_ connection: NWConnection) async {
        let channel = HTTPChannel(connection)
        defer { connection.cancel() }
        guard let request = try? await channel.request() else { return }
        switch (request.method, request.path) {
        case ("GET", "/v1/models"):
            let data: JSONValue = .array(
                models().map { model in
                    var fields: [String: JSONValue] = ["id": .string(model.id), "object": "model"]
                    if let detail = model.detail { fields["detail"] = .string(detail) }
                    // What it reads besides text, such as "image"; a field
                    // OpenAI's list doesn't have.
                    let inputs = model.runtime.inputs.map(\.rawValue).sorted()
                    if !inputs.isEmpty { fields["inputs"] = .array(inputs.map { .string($0) }) }
                    // How many tokens it reads and writes in all, so a client
                    // can size what it sends; the field llama.cpp and others use.
                    fields["context_length"] = .number(Double(model.runtime.budget.window))
                    return .object(fields)
                })
            try? await channel.respond(200, json: ["object": "list", "data": data])
        case ("POST", "/v1/chat/completions"):
            await complete(request.body, on: channel)
        default:
            try? await channel.respond(
                404,
                error: "There's nothing at \(request.method) \(request.path). The server has "
                    + "GET /v1/models and POST /v1/chat/completions.")
        }
    }

    private func complete(_ body: Data, on channel: HTTPChannel) async {
        let request: ChatCompletions.Request
        do {
            request = try JSONDecoder().decode(ChatCompletions.Request.self, from: body)
        } catch {
            try? await channel.respond(
                400,
                error: "The request isn't a chat completion (\(error)). Send JSON with "
                    + "model and messages.")
            return
        }
        guard let model = models().first(where: { $0.id == request.model }) else {
            let names = models().map(\.id).joined(separator: ", ")
            try? await channel.respond(404, error: "There's no model \(request.model) here. Pick one of: \(names).")
            return
        }
        let reply = Reply(model: request.model)
        if request.stream == true {
            await stream(request, on: model.runtime, reply: reply, channel: channel)
        } else {
            do throws(ModelRuntimeError) {
                let (outcome, speed) = try await ChatCompletions.run(request, on: model.runtime) { _ in }
                try? await channel.respond(200, json: reply.whole(outcome, speed: speed))
            } catch {
                try? await channel.respond(Self.status(of: error), error: error.description)
            }
        }
    }

    /// Streams the reply as server-sent events. The head is sent with the
    /// first of them, so a model that fails before it writes anything is
    /// reported with a status.
    private func stream(
        _ request: ChatCompletions.Request, on runtime: any ModelRuntime, reply: Reply, channel: HTTPChannel
    ) async {
        let (events, writer) = AsyncStream<Data>.makeStream()
        let work = Task { () -> Result<(ChatCompletions.Outcome, TurnSpeed?), ModelRuntimeError> in
            do throws(ModelRuntimeError) {
                return .success(
                    try await ChatCompletions.run(request, on: runtime) { text in
                        writer.yield(Reply.event(reply.delta(["role": "assistant", "content": .string(text)])))
                    })
            } catch {
                return .failure(error)
            }
        }
        let sending = Task {
            var started = false
            do {
                for await event in events {
                    if !started {
                        started = true
                        try await channel.head(200, type: "text/event-stream")
                    }
                    try await channel.chunk(event)
                }
            } catch {
                // The client hung up, so the model stops.
                work.cancel()
            }
            return started
        }
        let outcome = await work.value
        if case .success(let (result, speed)) = outcome {
            for event in reply.ending(result, speed: request.streamOptions?.includeUsage == true ? speed : nil) {
                writer.yield(Reply.event(event))
            }
            writer.yield(Data("data: [DONE]\n\n".utf8))
        }
        writer.finish()
        let started = await sending.value
        switch outcome {
        case .success: try? await channel.chunk(Data())
        // A reply cut off part way is left unfinished, which the client
        // reads as the connection failing.
        case .failure(let error) where !started:
            try? await channel.respond(Self.status(of: error), error: error.description)
        case .failure: break
        }
    }

    private static func status(of error: ModelRuntimeError) -> Int {
        switch error {
        case .unavailable, .engineMissing, .simulatorHost: 503
        case .exceededContext, .unsupportedInput, .decodingFailure, .declined: 400
        case .busy, .interrupted: 429
        case .cancelled, .failed: 500
        }
    }
}

/// The JSON a reply is made of.
struct Reply: Sendable {
    let id = "chatcmpl-\(UUID().uuidString.lowercased())"
    let created = Int(Date().timeIntervalSince1970)
    let model: String

    func chunk(_ choice: JSONValue?, usage: JSONValue? = nil) -> JSONValue {
        var fields: [String: JSONValue] = [
            "id": .string(id), "object": "chat.completion.chunk", "created": .number(Double(created)),
            "model": .string(model), "choices": .array(choice.map { [$0] } ?? []),
        ]
        if let usage { fields["usage"] = usage }
        return .object(fields)
    }

    func delta(_ delta: [String: JSONValue], finish: String? = nil) -> JSONValue {
        chunk(.object(["index": 0, "delta": .object(delta), "finish_reason": finish.map(JSONValue.string) ?? .null]))
    }

    /// What follows the answer's text: the tool call if there is one, why
    /// it ended, and what it used if that's asked for.
    func ending(_ outcome: ChatCompletions.Outcome, speed: TurnSpeed?) -> [JSONValue] {
        var events: [JSONValue] = []
        if case .call(let name, let arguments) = outcome {
            events.append(delta(["tool_calls": .array([Self.call(name, arguments, index: true)])]))
        }
        events.append(delta([:], finish: Self.finish(outcome)))
        if let speed { events.append(chunk(nil, usage: Self.usage(speed))) }
        return events
    }

    /// The reply in one piece, for a request that isn't streamed.
    func whole(_ outcome: ChatCompletions.Outcome, speed: TurnSpeed?) -> JSONValue {
        let message: JSONValue =
            switch outcome {
            case .answer(let text): .object(["role": "assistant", "content": .string(text)])
            case .call(let name, let arguments):
                .object(["role": "assistant", "content": .null, "tool_calls": .array([Self.call(name, arguments)])])
            }
        var fields: [String: JSONValue] = [
            "id": .string(id), "object": "chat.completion", "created": .number(Double(created)),
            "model": .string(model),
            "choices": .array([
                .object(["index": 0, "message": message, "finish_reason": .string(Self.finish(outcome))])
            ]),
        ]
        if let speed { fields["usage"] = Self.usage(speed) }
        return .object(fields)
    }

    static func event(_ value: JSONValue) -> Data {
        var data = Data("data: ".utf8)
        data.append((try? Wire.encoder.encode(value)) ?? Data("{}".utf8))
        data.append(Data("\n\n".utf8))
        return data
    }

    private static func call(_ name: String, _ arguments: JSONValue, index: Bool = false) -> JSONValue {
        let json = (try? Wire.encoder.encode(arguments)).map { String(decoding: $0, as: UTF8.self) } ?? "{}"
        var fields: [String: JSONValue] = [
            "id": .string("call_\(UUID().uuidString.prefix(8).lowercased())"), "type": "function",
            "function": .object(["name": .string(name), "arguments": .string(json)]),
        ]
        if index { fields["index"] = 0 }
        return .object(fields)
    }

    private static func finish(_ outcome: ChatCompletions.Outcome) -> String {
        if case .call = outcome { "tool_calls" } else { "stop" }
    }

    private static func usage(_ speed: TurnSpeed) -> JSONValue {
        [
            "prompt_tokens": .number(Double(speed.read)), "completion_tokens": .number(Double(speed.written)),
            "total_tokens": .number(Double(speed.read + speed.written)),
        ]
    }
}

/// One HTTP/1.1 request and its response on a connection, which is closed
/// after it.
final class HTTPChannel: Sendable {
    struct Request {
        var method: String
        var path: String
        var body: Data
    }

    /// Bodies beyond this are refused, which no conversation reaches.
    private static let maxBody = 64 << 20
    private let connection: NWConnection

    init(_ connection: NWConnection) {
        self.connection = connection
    }

    func request() async throws -> Request {
        var buffer = Data()
        let separator = Data("\r\n\r\n".utf8)
        while buffer.range(of: separator) == nil {
            guard let more = try await receive(), buffer.count < 64 << 10 else { throw HTTPError.malformed }
            buffer.append(more)
        }
        guard let end = buffer.range(of: separator),
            let head = String(data: buffer[..<end.lowerBound], encoding: .utf8)
        else { throw HTTPError.malformed }
        let lines = head.components(separatedBy: "\r\n")
        let words = lines.first?.split(separator: " ") ?? []
        guard words.count >= 2 else { throw HTTPError.malformed }
        var length = 0
        for line in lines.dropFirst() {
            let parts = line.split(separator: ":", maxSplits: 1)
            if parts.count == 2, parts[0].lowercased() == "content-length" {
                length = Int(parts[1].trimmingCharacters(in: .whitespaces)) ?? 0
            }
        }
        guard length <= Self.maxBody else { throw HTTPError.malformed }
        var body = Data(buffer[end.upperBound...])
        while body.count < length {
            guard let more = try await receive() else { throw HTTPError.malformed }
            body.append(more)
        }
        let path = words[1].split(separator: "?", maxSplits: 1).first.map(String.init) ?? ""
        return Request(method: String(words[0]), path: path, body: body.prefix(length))
    }

    func respond(_ status: Int, json: JSONValue) async throws {
        let body = try Wire.encoder.encode(json)
        try await send(Data(Self.head(status, type: "application/json", length: body.count).utf8) + body)
    }

    func respond(_ status: Int, error message: String) async throws {
        try await respond(status, json: ["error": ["message": .string(message), "type": "invalid_request_error"]])
    }

    /// The head of a response whose body comes in chunks.
    func head(_ status: Int, type: String) async throws {
        try await send(Data(Self.head(status, type: type, length: nil).utf8))
    }

    /// One chunk of the body; an empty one ends it.
    func chunk(_ data: Data) async throws {
        var framed = Data(String(data.count, radix: 16).utf8)
        framed.append(Data("\r\n".utf8))
        framed.append(data)
        framed.append(Data("\r\n".utf8))
        try await send(framed)
    }

    private static func head(_ status: Int, type: String, length: Int?) -> String {
        let reason = HTTPURLResponse.localizedString(forStatusCode: status).capitalized
        let size = length.map { "Content-Length: \($0)" } ?? "Transfer-Encoding: chunked"
        return "HTTP/1.1 \(status) \(reason)\r\nContent-Type: \(type)\r\n\(size)\r\nConnection: close\r\n"
            + "Cache-Control: no-cache\r\n\r\n"
    }

    private func receive() async throws -> Data? {
        try await withCheckedThrowingContinuation { continuation in
            connection.receive(minimumIncompleteLength: 1, maximumLength: 1 << 20) { data, _, complete, error in
                if let error {
                    continuation.resume(throwing: error)
                } else if let data, !data.isEmpty {
                    continuation.resume(returning: data)
                } else {
                    continuation.resume(returning: complete ? nil : Data())
                }
            }
        }
    }

    private func send(_ data: Data) async throws {
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, any Error>) in
            connection.send(
                content: data,
                completion: .contentProcessed { error in
                    if let error { continuation.resume(throwing: error) } else { continuation.resume() }
                })
        }
    }
}

enum HTTPError: Error {
    case malformed
}
