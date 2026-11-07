@_exported import InferenceKit

/// What LangReadr asks a model to do. The person can pick a model for each.
package enum ModelUse: String, ModelUseCase {
    /// Chats about passages.
    case chat
    /// Translating, writing in Latin letters and looking words up.
    case reading

    /// The model this use runs on, given what the person picked: for chats,
    /// Apple's; for reading, the chats' model. Unless they picked another
    /// for this use.
    package func model(in selection: [ModelUse: ModelID]) -> ModelID {
        if let picked = selection[self] { return picked }
        switch self {
        case .chat: return .apple
        case .reading: return ModelUse.chat.model(in: selection)
        }
    }

    package var standard: ModelID? {
        switch self {
        case .chat: .apple
        case .reading: nil
        }
    }

    package var title: String {
        switch self {
        case .chat: String(localized: "Chats")
        case .reading: String(localized: "Translating and words")
        }
    }
}

extension ModelLibraryClient {
    /// Apple's model, then each one downloaded, by name.
    package func runnable() async -> [RunnableModel] {
        RunnableModel.list(await installed())
    }
}
