import Foundation

/// The voice the person picked: one for every language, and one for each
/// language they picked apart, keyed by base subtag, saved as JSON in
/// `file`.
public final class VoiceSelection: Sendable {
    private struct Picks: Codable, Equatable {
        var standard: VoiceChoice = .system
        var languages: [String: VoiceChoice] = [:]
    }

    private let file: URL?
    private let picks: Locked<Picks>

    /// - Parameter file: Where the picks are kept; nil keeps them in memory.
    ///   A missing or unreadable file starts with the system voice.
    public init(file: URL?) {
        self.file = file
        let saved = file.flatMap { try? JSONDecoder().decode(Picks.self, from: Data(contentsOf: $0)) }
        picks = Locked(saved ?? Picks())
    }

    /// The voice for `language`: its own pick, else the one for every
    /// language. Nil asks for the one for every language.
    public func choice(for language: String? = nil) -> VoiceChoice {
        let picks = picks.value
        guard let language else { return picks.standard }
        return picks.languages[VoiceModel.base(of: language)] ?? picks.standard
    }

    /// Whether `language` has a pick of its own.
    public func hasOwnChoice(for language: String) -> Bool {
        picks.value.languages[VoiceModel.base(of: language)] != nil
    }

    /// Speaks `language` in `choice` from now on, or every language when
    /// nil. Nil as the choice for a language forgets its pick, so it
    /// follows the one for every language again.
    public func choose(_ choice: VoiceChoice?, for language: String? = nil) {
        save(
            picks.withValue { picks in
                if let language {
                    picks.languages[VoiceModel.base(of: language)] = choice
                } else {
                    picks.standard = choice ?? .system
                }
                return picks
            })
    }

    /// Forgets every pick of `id`, such as once it's deleted: those go back
    /// to the system voice, or to the one for every language.
    public func forget(_ id: ModelID) {
        save(
            picks.withValue { picks in
                if picks.standard.model == id { picks.standard = .system }
                picks.languages = picks.languages.filter { $0.value.model != id }
                return picks
            })
    }

    private func save(_ picks: Picks) {
        guard let file else { return }
        try? FileManager.default.createDirectory(
            at: file.deletingLastPathComponent(), withIntermediateDirectories: true)
        try? JSONEncoder().encode(picks).write(to: file, options: .atomic)
    }
}
