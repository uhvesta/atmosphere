import ComposableArchitecture
import Foundation
@_exported import ModelsClient

extension ModelID {
    /// No model: Lifr's own word lists, built into the system. Only
    /// searching runs on it, and does unless the person picks a model.
    package static let builtIn: ModelID = "built-in"
}

/// What Lifr asks a model to do. The person can pick a model for each.
package enum ModelUse: String, ModelUseCase {
    /// Conversations with the assistant.
    case assistant
    /// Writing skills from what the person describes.
    case skills
    /// Naming conversations.
    case titles
    /// Finding words related to what's searched for in conversations.
    case search

    /// The model this use runs on, given what the person picked: for the
    /// assistant, Apple's; for searching, the built-in word lists; for the
    /// rest, the assistant's model. Unless they picked another for this use.
    package func model(in selection: [ModelUse: ModelID]) -> ModelID {
        if let picked = selection[self] { return picked }
        switch self {
        case .assistant: return .apple
        case .search: return .builtIn
        case .skills, .titles: return ModelUse.assistant.model(in: selection)
        }
    }

    /// The model this use runs on when the person hasn't picked one, if it
    /// isn't the assistant's.
    package var standard: ModelID? {
        switch self {
        case .assistant: .apple
        case .search: .builtIn
        case .skills, .titles: nil
        }
    }

    /// Talking with the assistant and writing skills are chats; naming
    /// conversations and searching are quick tasks.
    package var purpose: TunePurpose {
        switch self {
        case .assistant, .skills: .chat
        case .titles, .search: .quick
        }
    }
}

extension ModelUse {
    package var title: String {
        switch self {
        case .assistant: String(localized: "Assistant")
        case .skills: String(localized: "Writing skills")
        case .titles: String(localized: "Naming conversations")
        case .search: String(localized: "Searching conversations")
        }
    }
}

extension ModelUseKey {
    package static let assistant = Self(ModelUse.assistant)
    package static let skills = Self(ModelUse.skills)
    package static let titles = Self(ModelUse.titles)
    package static let search = Self(ModelUse.search)

    /// The title of Lifr's use this is.
    package var title: String { ModelUse(rawValue: rawValue)?.title ?? rawValue }
}

extension InMemoryModelLibrary {
    /// A library for Lifr's uses.
    package convenience init(
        catalog: [ModelSpec] = ModelCatalog.all, installed: [ModelSpec] = [],
        links: [String: Result<ModelSpec, ModelResolveError>] = [:],
        now: Date = Date(timeIntervalSince1970: 1_793_700_000)
    ) {
        self.init(uses: ModelUse.self, catalog: catalog, installed: installed, links: links, now: now)
    }
}

extension SharedKey where Self == AppStorageKey<Bool>.Default {
    /// Whether a downloaded model keeps answering on the CPU while Lifr is in
    /// the background on iPhone, instead of waiting for Lifr to be back on
    /// screen. Off unless the person turns it on.
    package static var answersInBackground: Self {
        Self[.appStorage("lifrAnswersInBackground"), default: false]
    }
}
