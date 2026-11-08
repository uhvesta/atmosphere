import InferenceKit

/// A sample app's uses: chatting runs on Apple's model unless picked;
/// naming chats and writing skills follow the chat's model.
enum Use: String, ModelUseCase {
    case assistant, skills, titles

    func model(in selection: [Use: ModelID]) -> ModelID {
        if let picked = selection[self] { return picked }
        return self == .assistant ? .apple : Use.assistant.model(in: selection)
    }

    var standard: ModelID? { self == .assistant ? .apple : nil }

    var purpose: TunePurpose { self == .titles ? .quick : .chat }
}
