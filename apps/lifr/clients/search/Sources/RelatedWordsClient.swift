import ComposableArchitecture

/// Words and short phrases a conversation about what's searched for would
/// likely have, to find it by what it means as well as what it says.
package struct RelatedWordsClient: Sendable {
    /// Where the words come from, which says how soon to ask for them.
    package var source: @Sendable () -> RelatedWordsSource = { .builtIn }
    /// Words related to `query`, without its own; empty when there are
    /// none.
    package var related: @Sendable (_ query: String) async -> [String] = { _ in [] }

    package init(
        source: @escaping @Sendable () -> RelatedWordsSource = { .builtIn },
        related: @escaping @Sendable (_ query: String) async -> [String] = { _ in [] }
    ) {
        self.source = source
        self.related = related
    }
}

/// Where related words come from.
package enum RelatedWordsSource: Hashable, Sendable {
    /// The system's word lists: at once, on any device, with no model.
    case builtIn
    /// A model the person picked, which takes a while.
    case model
}

extension RelatedWordsClient: TestDependencyKey {
    package static let testValue = RelatedWordsClient()
    package static let previewValue = RelatedWordsClient.scripted(["milk", "eggs", "bread", "supermarket"])
}

extension DependencyValues {
    package var relatedWords: RelatedWordsClient {
        get { self[RelatedWordsClient.self] }
        set { self[RelatedWordsClient.self] = newValue }
    }
}

extension RelatedWordsClient {
    /// Answers every query with `words`, from `source`: for previews and
    /// tests.
    package static func scripted(_ words: [String], from source: RelatedWordsSource = .builtIn) -> Self {
        RelatedWordsClient(source: { source }, related: { _ in words })
    }
}
