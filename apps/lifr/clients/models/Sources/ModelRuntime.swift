import Foundation
import LifrAPI

/// A language model Lifr can run: Apple's on-device model, or one the
/// person downloaded. The assistant talks to every model through this, so
/// what it sends and how it retries is the same for each.
package protocol ModelRuntime: Sendable {
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
package struct ModelTurn: Sendable {
    /// What the model is, and the rules it follows.
    package var instructions: String
    /// The conversation, as one prompt.
    package var prompt: String
    /// What the model may call while it answers.
    package var tools: [ModelTool]
    /// The shape of the answer, or nil for plain text.
    package var schema: ModelSchema?
    /// Whether the schema is written into the prompt as well as enforced.
    /// Spelling it out helps the model and costs tokens.
    package var schemaInPrompt: Bool
    /// Photos and recordings sent with the prompt, in order.
    package var attachments: [ModelAttachment]

    package init(
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
    package func unreadable(by inputs: Set<ModelInput>) -> ModelInput? {
        attachments.map(\.kind).first { !inputs.contains($0) }
    }
}

/// What a model can read besides text.
package enum ModelInput: String, Hashable, Codable, Sendable, CaseIterable {
    /// A photo, as JPEG or PNG.
    case image
    /// A recording, as 16-bit WAV.
    case audio
}

/// A photo or recording on this device, sent to the model with a prompt.
package struct ModelAttachment: Hashable, Sendable {
    package var kind: ModelInput
    /// The file, which the model reads while it answers.
    package var file: URL

    package init(_ kind: ModelInput, file: URL) {
        self.kind = kind
        self.file = file
    }
}

/// A function the model can call while it answers.
package struct ModelTool: Sendable {
    package var name: String
    package var description: String
    /// The arguments, as the fields of one object.
    package var parameters: [ModelProperty]
    /// Runs the tool with the arguments the model wrote, an object, and
    /// returns what the model reads back.
    package var call: @Sendable (_ arguments: JSONValue) async -> String

    package init(
        name: String, description: String, parameters: [ModelProperty],
        call: @escaping @Sendable (_ arguments: JSONValue) async -> String
    ) {
        self.name = name
        self.description = description
        self.parameters = parameters
        self.call = call
    }
}

/// The shape of a value a model writes. Only what Lifr's answers and
/// tools use, so every runtime can enforce it.
package indirect enum ModelSchema: Hashable, Sendable {
    case string
    case bool
    /// One of `choices`. `name` labels it for runtimes that need one.
    case choice(name: String, [String])
    case array(ModelSchema)
    /// An object with these fields, in the order the model writes them.
    case object(name: String, [ModelProperty])
}

/// One field of an object.
package struct ModelProperty: Hashable, Sendable {
    package var name: String
    package var description: String
    package var schema: ModelSchema
    package var isOptional: Bool

    package init(_ name: String, _ description: String, _ schema: ModelSchema = .string, optional: Bool = false) {
        self.name = name
        self.description = description
        self.schema = schema
        isOptional = optional
    }
}

/// The tokens one model has for a turn.
package struct ContextBudget: Hashable, Sendable {
    /// Everything the model reads and writes.
    package var window: Int
    /// Kept for the answer and the tools' results.
    package var reservedForAnswer: Int

    package init(window: Int, reservedForAnswer: Int) {
        self.window = window
        self.reservedForAnswer = reservedForAnswer
    }

    /// What the prompt, instructions, tools and schema may take.
    package var room: Int { window - reservedForAnswer }

    /// Apple's on-device model on iOS and macOS 26.
    package static let appleOnDevice = ContextBudget(window: 4_096, reservedForAnswer: 1_100)
}

/// Whether a model can answer now.
package enum ModelStatus: Hashable, Sendable {
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
    /// The model needs more memory than the system leaves Lifr now.
    case notEnoughMemory(needed: Int64, available: Int64)
}

/// Why Lifr stopped a model in the middle of an answer.
package enum ModelInterruption: Hashable, Sendable {
    /// The system ran low on memory.
    case memory
    /// Lifr went to the background.
    case background
}

/// Why a model didn't answer.
package enum ModelRuntimeError: Error, Hashable, Sendable, CustomStringConvertible {
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
    /// Lifr unloaded the model while it answered.
    case interrupted(ModelInterruption)
    /// The Simulator couldn't reach the Mac's model.
    case simulatorHost(String)
    /// Anything else the runtime reported.
    case failed(String)

    package var description: String {
        switch self {
        case .unavailable(let status):
            "The model can't run now (\(status)). Check Settings for what it needs."
        case .exceededContext:
            "The conversation doesn't fit the model. Start a new conversation, or send a shorter message."
        case .decodingFailure:
            "The model's answer didn't have the shape Lifr asked for. Send the message again."
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
            "The device ran low on memory, so Lifr stopped the model. Send the message again; if it keeps "
                + "happening, close other apps or pick a smaller model."
        case .interrupted(.background):
            "Lifr stopped the model when it went to the background. Send the message again."
        case .simulatorHost(let reason):
            "The Simulator couldn't reach the Mac's model (\(reason)). Use a Simulator runtime with the same "
                + "version as macOS, or a device."
        case .failed(let reason):
            "The model stopped: \(reason). Send the message again; if it keeps failing, restart Lifr."
        }
    }
}
