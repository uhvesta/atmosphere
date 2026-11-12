import LifrAPI
import LifrModelsClient
import LifrSearchClient
import WordsKit

extension RelatedWordsClient {
    /// Related words from the system's word lists, or from the model
    /// `selected` names when the person picked one for searching. When that
    /// model can't answer, or finds nothing, the word lists do instead, so
    /// a search never depends on a model.
    package static func live(
        selected: @escaping @Sendable () -> ModelID, runtime: @escaping @Sendable () -> any ModelRuntime
    ) -> Self {
        RelatedWordsClient(
            source: { selected() == .builtIn ? .builtIn : .model },
            related: { query in
                await RelatedWords.related(
                    to: query, searching: "their conversations with the assistant of Lifr, a task app",
                    on: selected() == .builtIn ? nil : runtime())
            })
    }
}
