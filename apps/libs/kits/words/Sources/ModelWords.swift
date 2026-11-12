import Foundation
import InferenceKit
import JSONKit

/// Related words from a model on the device, such as Apple's or one the
/// person picked for searching.
public enum ModelWords {
    /// What the model is told, for a search of `searched`, such as "their
    /// conversations with the assistant of Lifr, a task app".
    public static func instructions(searching searched: String) -> String {
        """
        You help a person search \(searched). Given what they searched for, answer with up to 12 words or short \
        phrases a conversation about it would likely have: other words for it, things that belong to it and \
        examples. For "groceries": milk, eggs, bread, supermarket, shopping. One per line, in lowercase, without \
        numbers or any other text.
        """
    }

    /// What `model` relates to `query` in a search of `searched`; empty
    /// when it can't answer.
    public static func related(to query: String, searching searched: String, on model: any ModelRuntime) async
        -> [String]
    {
        guard model.status == .available else { return [] }
        let turn = ModelTurn(instructions: instructions(searching: searched), prompt: "Search: \(query)")
        guard case .string(let answer)? = try? await model.respond(to: turn, partial: { _ in }) else { return [] }
        return words(in: answer, besides: query)
    }

    /// The words and phrases in the model's answer: one per line or comma,
    /// without list marks or a line leading into them, of at most three
    /// words, leaving out what was searched for and repeats. At most 12.
    static func words(in answer: String, besides query: String) -> [String] {
        let searched = query.lowercased()
        var seen: Set<String> = []
        return answer.split(whereSeparator: \.isNewline)
            // A line leading into the list, such as "Here you go:".
            .filter { !$0.trimmingCharacters(in: .whitespaces).hasSuffix(":") }
            .flatMap { $0.split(separator: ",") }
            .map { line in
                line.trimmingCharacters(in: .whitespaces.union(.punctuationCharacters).union(.decimalDigits))
                    .lowercased()
            }
            .filter { phrase in
                !phrase.isEmpty && phrase.split(separator: " ").count <= 3 && phrase != searched
                    && seen.insert(phrase).inserted
            }
            .prefix(12).map { $0 }
    }
}
