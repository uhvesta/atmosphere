import Foundation
import JSONKit

/// A language model the app can run: Apple's on-device model, or one the
/// person downloaded. The assistant talks to every model through this, so
/// what it sends and how it retries is the same for each.
public protocol ModelRuntime: Sendable {
    /// Whether the model can answer now, and if not, why.
    var status: ModelStatus { get }
    /// The tokens the model reads and writes in all, and what's kept for
    /// its answer and the tools' results.
    var budget: ContextBudget { get }
    /// What the model can read besides text.
    var inputs: Set<ModelInput> { get }
    /// Loads the model ahead of the first turn, so the first answer starts
    /// sooner. Does nothing when it's loaded or can't run.
    func prewarm()
    /// The model's answer to `turn`. `partial` gets the answer as far as
    /// it's written, each time it grows: a JSON value with the turn's
    /// schema, or a string without one. Returns the whole answer.
    func respond(to turn: ModelTurn, partial: @escaping @Sendable (JSONValue) -> Void)
        async throws(ModelRuntimeError) -> JSONValue
}

/// What a model gets for one answer.
public struct ModelTurn: Sendable {
    /// What the model is, and the rules it follows.
    public var instructions: String
    /// The conversation, as one prompt.
    public var prompt: String
    /// What the model may call while it answers.
    public var tools: [ModelTool]
    /// The shape of the answer, or nil for plain text.
    public var schema: ModelSchema?
    /// Whether the schema is written into the prompt as well as enforced.
    /// Spelling it out helps the model and costs tokens.
    public var schemaInPrompt: Bool
    /// Photos and recordings sent with the prompt, in order.
    public var attachments: [ModelAttachment]
    /// Gets what the model thinks before it answers, each time it grows,
    /// and how long it took once it's done. Nil when the turn isn't worth
    /// the wait, such as naming a conversation, and the model answers
    /// straight away.
    public var thinking: (@Sendable (ModelThinking) -> Void)?

    public init(
        instructions: String, prompt: String, tools: [ModelTool] = [], schema: ModelSchema? = nil,
        schemaInPrompt: Bool = true, attachments: [ModelAttachment] = []
    ) {
        self.instructions = instructions
        self.prompt = prompt
        self.tools = tools
        self.schema = schema
        self.schemaInPrompt = schemaInPrompt
        self.attachments = attachments
    }

    /// The first kind of attachment `inputs` doesn't include, if any.
    public func unreadable(by inputs: Set<ModelInput>) -> ModelInput? {
        attachments.map(\.kind).first { !inputs.contains($0) }
    }
}

/// What a model thought on its way to an answer, and how long it worked
/// on it.
public struct ModelThinking: Hashable, Codable, Sendable {
    /// Its reasoning, as it wrote it.
    public var text: String
    /// How long the model spent reading the turn and writing, not counting
    /// loading it or waiting for tools. Nil until the answer is done, or
    /// when the model doesn't say.
    public var duration: Duration?
    /// What it ran on.
    public var processor: ModelBackend?

    public init(text: String, duration: Duration? = nil, processor: ModelBackend? = nil) {
        self.text = text
        self.duration = duration
        self.processor = processor
    }

    /// This, then `next`: as when a turn asks the model again after a tool.
    public func then(_ next: ModelThinking) -> ModelThinking {
        let joined = [text, next.text].filter { !$0.isEmpty }.joined(separator: "\n\n")
        let total: Duration? =
            switch (duration, next.duration) {
            case (nil, nil): nil
            case (let one?, nil), (nil, let one?): one
            case (let one?, let other?): one + other
            }
        return ModelThinking(text: joined, duration: total, processor: next.processor ?? processor)
    }
}

/// What a model can read besides text.
public enum ModelInput: String, Hashable, Codable, Sendable, CaseIterable {
    /// A photo, as JPEG or PNG.
    case image
    /// A recording, as 16-bit WAV.
    case audio
}

/// A photo or recording on this device, sent to the model with a prompt.
public struct ModelAttachment: Hashable, Sendable {
    public var kind: ModelInput
    /// The file, which the model reads while it answers.
    public var file: URL

    public init(_ kind: ModelInput, file: URL) {
        self.kind = kind
        self.file = file
    }
}

/// A function the model can call while it answers.
public struct ModelTool: Sendable {
    public var name: String
    public var description: String
    /// The arguments, as the fields of one object.
    public var parameters: [ModelProperty]
    /// Runs the tool with the arguments the model wrote, an object, and
    /// returns what the model reads back.
    public var call: @Sendable (_ arguments: JSONValue) async -> String

    public init(
        name: String, description: String, parameters: [ModelProperty],
        call: @escaping @Sendable (_ arguments: JSONValue) async -> String
    ) {
        self.name = name
        self.description = description
        self.parameters = parameters
        self.call = call
    }
}

/// The shape of a value a model writes. Only what the app's answers and
/// tools use, so every runtime can enforce it.
public indirect enum ModelSchema: Hashable, Sendable {
    case string
    case bool
    /// One of `choices`. `name` labels it for runtimes that need one.
    case choice(name: String, [String])
    case array(ModelSchema)
    /// An object with these fields, in the order the model writes them.
    case object(name: String, [ModelProperty])
}

/// One field of an object.
public struct ModelProperty: Hashable, Sendable {
    public var name: String
    public var description: String
    public var schema: ModelSchema
    public var isOptional: Bool

    public init(_ name: String, _ description: String, _ schema: ModelSchema = .string, optional: Bool = false) {
        self.name = name
        self.description = description
        self.schema = schema
        isOptional = optional
    }
}

/// The tokens one model has for a turn.
public struct ContextBudget: Hashable, Sendable {
    /// Everything the model reads and writes.
    public var window: Int
    /// Kept for the answer and the tools' results.
    public var reservedForAnswer: Int

    public init(window: Int, reservedForAnswer: Int) {
        self.window = window
        self.reservedForAnswer = reservedForAnswer
    }

    /// What the prompt, instructions, tools and schema may take.
    public var room: Int { window - reservedForAnswer }

    /// Apple's on-device model on iOS and macOS 26.
    public static let appleOnDevice = ContextBudget(window: 4_096, reservedForAnswer: 1_100)
}

/// Whether a model can answer now.
public enum ModelStatus: Hashable, Sendable {
    case available
    /// Older than iOS 26 or macOS 26, which Apple's model needs.
    case systemTooOld
    /// The device can't run Apple Intelligence.
    case deviceNotEligible
    /// Apple Intelligence is turned off.
    case notEnabled
    /// Apple's model is still downloading.
    case modelNotReady
    /// The model picked isn't on the device: its download didn't finish,
    /// or it was deleted.
    case notDownloaded
    /// The model needs more memory than the system leaves the app now: the
    /// bytes it takes with what's kept free besides, and the bytes free.
    case notEnoughMemory(needed: Int64, available: Int64)
}

/// Why the app stopped a model in the middle of an answer.
public enum ModelInterruption: Hashable, Sendable {
    /// The system ran low on memory.
    case memory
    /// The app went to the background.
    case background
}

/// Where a downloaded model runs. iOS lets an app use the GPU only while
/// it's on screen, so a model that keeps answering in the background moves
/// to the CPU, which is slower and uses more battery.
public enum ModelBackend: String, Hashable, Codable, Sendable {
    case gpu
    case cpu
}

/// Why a model didn't answer.
public enum ModelRuntimeError: Error, Hashable, Sendable, CustomStringConvertible {
    /// The model can't run now.
    case unavailable(ModelStatus)
    /// The turn and its answer don't fit the model's context window.
    case exceededContext
    /// The model wrote something that doesn't fit the turn's schema.
    case decodingFailure
    /// The model declined, such as for content it won't write about.
    case declined
    /// The model is busy with another request, or the system is limiting it.
    case busy
    /// The turn has a photo or recording the model can't read.
    case unsupportedInput(ModelInput)
    /// The turn was canceled before the model finished.
    case cancelled
    /// The app unloaded the model while it answered.
    case interrupted(ModelInterruption)
    /// The Simulator couldn't reach the Mac's model.
    case simulatorHost(String)
    /// The model needs an engine this build of the app doesn't have, as for
    /// one downloaded by a build that had it.
    case engineMissing(RuntimeKind)
    /// Anything else the runtime reported.
    case failed(String)

    public var description: String {
        switch self {
        case .unavailable(let status):
            "The model can't run now (\(status)). Check Settings for what it needs."
        case .exceededContext:
            "The conversation doesn't fit the model. Start a new conversation, or send a shorter message."
        case .decodingFailure:
            "The model's answer didn't have the shape the app asked for. Send the message again."
        case .declined:
            "The model declined to answer. Rephrase the message."
        case .busy:
            "The model is busy. Wait a moment and send the message again."
        case .unsupportedInput(.image):
            "This model can't read photos. Pick one that can, such as Gemma 4 E4B, or remove the photo."
        case .unsupportedInput(.audio):
            "This model can't listen to recordings. Pick one that can, such as Gemma 4 E4B, or remove the recording."
        case .cancelled:
            "The answer was stopped before it finished. Send the message again."
        case .interrupted(.memory):
            "The device ran low on memory, so the app stopped the model. Send the message again; if it keeps "
                + "happening, close other apps or pick a smaller model."
        case .interrupted(.background):
            "The app stopped the model when it went to the background. Send the message again."
        case .simulatorHost(let reason):
            "The Simulator couldn't reach the Mac's model (\(reason)). Use a Simulator runtime with the same "
                + "version as macOS, or a device."
        case .engineMissing(let engine):
            "This version of the app can't run \(engine.name) models. Pick another model in Settings, or delete "
                + "this one and download it again in a version of the app that runs it."
        case .failed(let reason):
            "The model stopped: \(reason). Send the message again; if it keeps failing, restart the app."
        }
    }
}
