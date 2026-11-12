import InferenceKit

/// Words and short phrases a text about what's searched for would likely
/// have, to find it by what it means as well as what it says.
public enum RelatedWords {
    /// Words related to `query`, without its own: from `model` when it's
    /// given and answers, otherwise from the system's word lists, so a
    /// search never depends on a model. Empty when neither has any.
    public static func related(
        to query: String, searching searched: String, on model: (any ModelRuntime)?
    ) async -> [String] {
        if let model {
            let words = await ModelWords.related(to: query, searching: searched, on: model)
            if !words.isEmpty { return words }
        }
        return BuiltInWords.related(to: query)
    }
}
