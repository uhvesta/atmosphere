import Foundation
@_exported import ModelsClient

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

    /// Chats are chats; reading's lookups are quick tasks.
    package var purpose: TunePurpose {
        switch self {
        case .chat: .chat
        case .reading: .quick
        }
    }

    package var title: String {
        switch self {
        case .chat: String(localized: "Chats")
        case .reading: String(localized: "Translating and words")
        }
    }
}

extension ModelUseKey {
    package static let chat = Self(ModelUse.chat)
    package static let reading = Self(ModelUse.reading)

    /// The title of LangReadr's use this is.
    package var title: String { ModelUse(rawValue: rawValue)?.title ?? rawValue }
}

extension InMemoryModelLibrary {
    /// A library for LangReadr's uses.
    package convenience init(
        catalog: [ModelSpec] = ModelCatalog.all, installed: [ModelSpec] = [],
        links: [String: Result<ModelSpec, ModelResolveError>] = [:],
        now: Date = Date(timeIntervalSince1970: 1_793_700_000)
    ) {
        self.init(uses: ModelUse.self, catalog: catalog, installed: installed, links: links, now: now)
    }
}
