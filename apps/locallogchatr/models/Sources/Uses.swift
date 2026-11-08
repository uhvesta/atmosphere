import Foundation
@_exported import ModelsClient

/// What LocalLogChatr asks a model to do: chat. The person picks the model
/// in Settings, and each chat can pick its own.
package enum ModelUse: String, ModelUseCase {
    case chat

    package func model(in selection: [ModelUse: ModelID]) -> ModelID {
        selection[self] ?? .apple
    }

    package var standard: ModelID? { .apple }

    package var purpose: TunePurpose { .chat }

    package var title: String { String(localized: "Chats") }
}

extension ModelUseKey {
    package static let chat = Self(ModelUse.chat)

    /// The title of LocalLogChatr's use this is.
    package var title: String { ModelUse(rawValue: rawValue)?.title ?? rawValue }
}

extension InMemoryModelLibrary {
    /// A library for LocalLogChatr's uses.
    package convenience init(
        catalog: [ModelSpec] = ModelCatalog.all, installed: [ModelSpec] = [],
        links: [String: Result<ModelSpec, ModelResolveError>] = [:],
        now: Date = Date(timeIntervalSince1970: 1_793_700_000)
    ) {
        self.init(uses: ModelUse.self, catalog: catalog, installed: installed, links: links, now: now)
    }
}
