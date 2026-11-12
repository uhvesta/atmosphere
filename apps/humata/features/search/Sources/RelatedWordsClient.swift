import ComposableArchitecture
import InferenceKit
import InferenceKitLive
import WordsKit

/// Words that mean much the same as a search's, so it finds what a session
/// said in other words: from Apple's on-device model when it's there,
/// otherwise from the system's word lists.
package struct RelatedWordsClient: Sendable {
    package var related: @Sendable (_ query: String) async -> [String]

    package init(related: @escaping @Sendable (_ query: String) async -> [String]) {
        self.related = related
    }
}

extension RelatedWordsClient: DependencyKey {
    package static let liveValue = RelatedWordsClient { query in
        let model = AppleRuntime()
        return await RelatedWords.related(
            to: query, searching: "conversations with coding agents: messages, replies, commands and their output",
            on: model.status == .available ? model : nil)
    }

    package static let testValue = RelatedWordsClient { _ in [] }
    package static let previewValue = RelatedWordsClient { _ in [] }
}

extension DependencyValues {
    package var relatedWords: RelatedWordsClient {
        get { self[RelatedWordsClient.self] }
        set { self[RelatedWordsClient.self] = newValue }
    }
}
