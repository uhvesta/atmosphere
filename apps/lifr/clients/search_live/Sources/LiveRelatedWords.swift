import LifrAPI
import LifrModelsClient
import LifrSearchClient

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
                if selected() != .builtIn {
                    let words = await ModelWords.related(to: query, on: runtime())
                    if !words.isEmpty { return words }
                }
                return BuiltInWords.related(to: query)
            })
    }
}
