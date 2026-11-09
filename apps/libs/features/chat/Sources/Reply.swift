import ChatKit
import ComposableArchitecture
import Foundation
import InferenceKit

/// A reply as the model writes it, how much of its window the chat took,
/// and how fast the model read and wrote.
public struct ChatReply: Hashable, Sendable {
    public var text: String
    public var thinking: ThreadThinking?
    public var context: ContextUse
    public var speed: ThreadSpeed?

    public init(text: String, thinking: ThreadThinking? = nil, context: ContextUse, speed: ThreadSpeed? = nil) {
        self.text = text
        self.thinking = thinking
        self.context = context
        self.speed = speed
    }
}

/// What a model was sent for a reply, exactly as it got it.
public struct ReplyPrompt: Hashable, Sendable {
    /// What the model is told before the chat.
    public var instructions: String
    /// The chat's newest messages that fit, with the files sent with the
    /// last.
    public var prompt: String
    /// How many photos went with it.
    public var photos: Int
    /// The model it was sent to, by name, once it's looked up.
    public var model: String?
    /// How much of the model's window it took, once it replied.
    public var context: ContextUse?

    public init(
        instructions: String, prompt: String, photos: Int, model: String? = nil, context: ContextUse? = nil
    ) {
        self.instructions = instructions
        self.prompt = prompt
        self.photos = photos
        self.model = model
        self.context = context
    }
}

/// Part of a reply on its way: more of the text, or of what the model
/// thinks first.
public enum ReplyProgress: Hashable, Sendable {
    case text(String)
    case thinking(ThreadThinking)
}

/// The turn that asks for the reply to the last message of a chat.
struct ReplyTurn {
    var turn: ModelTurn

    /// The turn for `thread` on a model with `budget`: what the chat knows,
    /// then its newest messages that fit, and the files sent with the last.
    /// `photos` are the files of the photos sent with message `photosFrom`,
    /// which the model sees; a message's other photos are only named. The
    /// model thinks for `thinkingTokens` first, if set.
    init(
        thread: ChatThread, instructions: String, budget: ContextBudget, thinkingTokens: Int?, photos: [URL] = [],
        photosFrom: UUID? = nil
    ) {
        // About three characters a token, less what the photos take.
        let room = max(400, budget.room * 3 - instructions.count - 200 - photos.count * Self.photoCharacters)
        // The files sent with the new message, in at most half the room;
        // earlier ones were read with their own messages.
        let files = Self.files(of: thread.messages.last, fitting: room / 2)
        let transcript =
            thread.messages.newest(fitting: room - files.count)
            .map { message in
                let role = message.role == .person ? "User" : "Assistant"
                return "\(role): \(message.text)\(Self.photos(of: message, seen: photos.isEmpty ? nil : photosFrom))"
            }
            .joined(separator: "\n\n") + files
        turn = ModelTurn(
            instructions: instructions, prompt: transcript,
            attachments: photos.map { ModelAttachment(.image, file: $0) })
        if let thinkingTokens {
            turn.thinkingTokens = thinkingTokens
        }
    }

    /// What a photo takes of the model's window, in characters: models
    /// read one as up to about 280 tokens.
    static let photoCharacters = 280 * 3

    /// Says `message` was sent with photos, and when they aren't the ones
    /// the model sees, that it can't see them.
    static func photos(of message: ThreadMessage, seen: UUID?) -> String {
        let count = message.photos.count
        guard count > 0 else { return "" }
        let photos = count == 1 ? "a photo" : "\(count) photos"
        return message.id == seen ? " [Sent with \(photos)]" : " [Sent with \(photos), which you can't see now]"
    }

    /// The text of the files sent with `message`, each after its name, cut
    /// to `room` characters in all; empty when it has none.
    static func files(of message: ThreadMessage?, fitting room: Int) -> String {
        guard let message, message.role == .person, !message.files.isEmpty else { return "" }
        var left = room
        var written = ""
        for file in message.files {
            let text = String(file.text.prefix(max(0, left)))
            left -= text.count
            let cut = text.count < file.text.count ? ", cut short" : ""
            written += "\n\n[The file \(file.name)\(cut):\n\(text)\n]"
        }
        return written
    }

    /// What the model is sent.
    var prompt: ReplyPrompt {
        ReplyPrompt(instructions: turn.instructions, prompt: turn.prompt, photos: turn.attachments.count)
    }

    /// Asks `model`, sending the reply and what it thinks as they grow.
    /// `clock` times the reply when the model doesn't count its tokens.
    func ask(
        _ model: any ModelRuntime, thinks: Bool, clock: some Clock<Duration>,
        progress: @escaping @Sendable (ReplyProgress) -> Void
    ) async throws(ModelRuntimeError) -> ChatReply {
        var turn = turn
        let thought = LockIsolated<ThreadThinking?>(nil)
        let measured = LockIsolated<TurnSpeed?>(nil)
        let start = clock.now
        let first = LockIsolated<Duration?>(nil)
        let began = { @Sendable in first.withValue { $0 = $0 ?? start.duration(to: clock.now) } }
        if thinks {
            turn.thinking = { thinking in
                began()
                let kept = ThreadThinking(text: thinking.text, duration: thinking.duration)
                thought.setValue(kept)
                progress(.thinking(kept))
            }
        }
        turn.speed = { speed in measured.setValue(speed) }
        let answer = try await model.respond(to: turn) { value in
            began()
            if case .string(let text) = value { progress(.text(text)) }
        }
        guard case .string(let written) = answer else {
            throw .failed("the model answered with a value, not text")
        }
        let text = written.trimmingCharacters(in: .whitespacesAndNewlines)
        let thinking = thought.value.flatMap { $0.text.isEmpty ? nil : $0 }
        let read = turn.instructions.utf8.count + turn.prompt.utf8.count
        let writes = text.utf8.count + (thinking?.text.utf8.count ?? 0)
        let used = (read + writes) / 3
        let speed =
            measured.value.map {
                ThreadSpeed(read: $0.read, written: $0.written, reading: $0.reading, writing: $0.writing)
            }
            ?? {
                // Guessed for a model that doesn't count its tokens, about
                // three characters a token, from when it began to write.
                let whole = start.duration(to: clock.now)
                let reading = first.value ?? whole
                return ThreadSpeed(
                    read: read / 3 + turn.attachments.count * Self.photoCharacters / 3, written: writes / 3,
                    reading: reading, writing: whole - reading, estimated: true)
            }()
        return ChatReply(
            text: text, thinking: thinking, context: ContextUse(used: used, window: model.budget.window), speed: speed)
    }
}
