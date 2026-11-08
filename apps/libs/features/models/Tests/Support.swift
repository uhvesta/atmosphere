import ModelsClient

/// What a test app asks models to do: chatting, and two uses that follow
/// the chat's model unless they're picked.
enum TestUse: String, ModelUseCase {
    case assistant, skills, titles

    func model(in selection: [TestUse: ModelID]) -> ModelID {
        if let picked = selection[self] { return picked }
        return self == .assistant ? .apple : TestUse.assistant.model(in: selection)
    }

    var standard: ModelID? { self == .assistant ? .apple : nil }
}

extension ModelUseKey {
    static let assistant = Self(TestUse.assistant)
    static let skills = Self(TestUse.skills)
    static let titles = Self(TestUse.titles)
}

extension InMemoryModelLibrary {
    convenience init(
        catalog: [ModelSpec] = ModelCatalog.all, installed: [ModelSpec] = [],
        links: [String: Result<ModelSpec, ModelResolveError>] = [:]
    ) {
        self.init(uses: TestUse.self, catalog: catalog, installed: installed, links: links)
    }
}
