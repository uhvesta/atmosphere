import ChatKit
import ComposableArchitecture
import Foundation
import InferenceKit
import ModelsClient

/// A message the shared chat can show, send and reply to: what's sent with
/// it, and for a reply, what the model thought, which model wrote it and
/// how fast. An app with more to keep on a message, such as what a reply
/// asks to change, keeps it as the message's `Extra`.
public protocol ThreadEntry: ChatEntry {
    /// What the app's replies carry besides their text.
    associatedtype Extra: Hashable, Sendable

    var files: [ThreadFile] { get }
    var photos: [ThreadPhoto] { get }
    var thinking: ThreadThinking? { get }
    /// The model that wrote a reply, by name.
    var model: String? { get }
    var speed: ThreadSpeed? { get }
    /// Whether the person stopped the reply before the model finished it.
    var stopped: Bool { get set }

    /// The person's message, sent with `files` and `photos`.
    init(person id: UUID, text: String, at: Date, files: [ThreadFile], photos: [ThreadPhoto])

    /// A reply, with what the app's replier added to it.
    init(
        reply id: UUID, text: String, at: Date, thinking: ThreadThinking?, model: String?, speed: ThreadSpeed?,
        extra: Extra?)
}

extension ThreadMessage: ThreadEntry {
    public typealias Extra = Never

    public init(person id: UUID, text: String, at: Date, files: [ThreadFile], photos: [ThreadPhoto]) {
        self.init(id: id, role: .person, text: text, at: at, files: files, photos: photos)
    }

    public init(
        reply id: UUID, text: String, at: Date, thinking: ThreadThinking?, model: String?, speed: ThreadSpeed?,
        extra: Never?
    ) {
        self.init(id: id, role: .assistant, text: text, at: at, thinking: thinking, model: model, speed: speed)
    }
}

/// Writes a chat's replies: a model on the device, or an app's assistant
/// with its own tools and what it knows.
public protocol ChatReplier: Sendable {
    associatedtype Message: ThreadEntry
    associatedtype Context: ConversationContext
    /// Why a reply didn't come, said for the person.
    associatedtype Failure: Error, CustomStringConvertible

    /// What the app tells its replier besides the chat, kept in the chat's
    /// state, such as the records its assistant looks things up in.
    associatedtype Host: Equatable, Sendable = NoHost

    /// What's sent for the reply to the last message of `request.chat`, and
    /// how to write it. Called as the reply is asked for, so it reads the
    /// chat as it is then.
    func ask(_ request: ReplyRequest<Self>) -> ReplyAsk<Message.Extra, Failure>

    /// The person's message as it's sent, in place of `old` when they
    /// edited that, with what the app adds to it. Called before `ask`.
    func sending(_ message: Message, replacing old: Message?, host: Host) -> Message

    /// Whether writing the reply again can help after `failure`: not for a
    /// message that's too long, or one that already changed something.
    func canRetry(after failure: Failure) -> Bool
}

extension ChatReplier {
    public func sending(_ message: Message, replacing old: Message?, host: Host) -> Message { message }
    public func canRetry(after failure: Failure) -> Bool { true }
}

/// What an app that tells its replier nothing besides the chat keeps.
public struct NoHost: Equatable, Sendable {
    public init() {}
}

/// The reply a chat asks for: to the last message of `chat`, from `choice`
/// for the app's `use`.
public struct ReplyRequest<Replier: ChatReplier>: Sendable {
    public var chat: Conversation<Replier.Message, Replier.Context>
    /// What the model is told before the messages.
    public var instructions: String
    public var use: ModelUseKey
    public var choice: ModelChoice
    public var host: Replier.Host
    /// Whether it's the second model's answer, beside the first's, which
    /// the person may keep in its place. It shouldn't change anything.
    public var isRival = false
}

/// A reply asked for: what it's sent, and how to write it.
public struct ReplyAsk<Extra: Hashable & Sendable, Failure: Error>: Sendable {
    public var prompt: ReplyPrompt
    /// Writes the reply, sending each part as it's written.
    public var write:
        @Sendable (_ progress: @escaping @Sendable (ReplyProgress) -> Void) async throws(Failure) -> ChatReply<Extra>

    public init(
        prompt: ReplyPrompt,
        write:
            @escaping @Sendable (_ progress: @escaping @Sendable (ReplyProgress) -> Void) async throws(Failure) ->
            ChatReply<Extra>
    ) {
        self.prompt = prompt
        self.write = write
    }

    /// Sends the reply as it's written, then once it's done.
    func run<Action: Sendable>(
        progressed: @escaping @Sendable (ReplyProgress) -> Action,
        replied: @escaping @Sendable (Result<ChatReply<Extra>, Failure>) -> Action
    ) -> Effect<Action> {
        .run { [write] send in
            let (updates, continuation) = AsyncStream.makeStream(
                of: ReplyProgress.self, bufferingPolicy: .bufferingNewest(8))
            async let written = outcome { () throws(Failure) in
                defer { continuation.finish() }
                return try await write { continuation.yield($0) }
            }
            for await update in updates { await send(progressed(update)) }
            await send(replied(await written))
        }
    }
}

/// Replies from a model on the device, from the chat's messages that fit
/// its window, with the photos last sent for a model that reads them.
public struct ModelReplier: ChatReplier {
    public typealias Message = ThreadMessage
    public typealias Context = NoContext
    public typealias Failure = ModelRuntimeError

    @Dependency(\.chatRuntime) private var chatRuntime
    @Dependency(\.chatPhotos) private var chatPhotos

    public init() {}

    public func ask(_ request: ReplyRequest<Self>) -> ReplyAsk<Never, ModelRuntimeError> {
        let model = chatRuntime.runtime(request.use, request.choice)
        let tokens = request.choice.thinkingLevel.tokens
        let chat = request.chat
        // The newest photos sent, so the model can be asked about them
        // again, for a model that reads them.
        let sent = chat.messages.last { !$0.photos.isEmpty }
        let photos =
            model.inputs.contains(.image) ? (sent?.photos ?? []).compactMap { chatPhotos.file($0, chat.id) } : []
        let turn = ReplyTurn(
            thread: chat, instructions: request.instructions, budget: model.budget, thinkingTokens: tokens,
            photos: photos, photosFrom: sent?.id)
        return ReplyAsk(prompt: turn.prompt) { progress throws(ModelRuntimeError) in
            try await turn.ask(model, thinks: tokens != nil, clock: ContinuousClock(), progress: progress)
        }
    }
}
