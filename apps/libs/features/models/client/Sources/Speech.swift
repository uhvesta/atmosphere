import ComposableArchitecture
import Foundation

/// Reading aloud: the voice picked for every language and for each one
/// picked apart, and speaking in it. The app gives the live one, on the
/// device's speaker; tests and previews give one that keeps picks in
/// memory and plays nothing.
@DependencyClient
public struct SpeechClient: Sendable {
    /// The voice picked for `language`, nil if it follows the one for
    /// every language, or the one for every language when nil.
    public var choice: @Sendable (_ language: String?) -> VoiceChoice? = { _ in nil }
    /// Picks `choice` for `language`, or for every language when nil. A nil
    /// choice has the language follow the one for every language.
    public var choose: @Sendable (_ choice: VoiceChoice?, _ language: String?) async -> Void
    /// Forgets every pick of a removed voice model.
    public var forget: @Sendable (_ id: ModelID) async -> Void
    // A typed throw needs its closure's parameters spelled out, and the
    // formatter puts them on their own line.
    // swiftlint:disable closure_parameter_position
    /// Reads `text` aloud in `language`, a BCP 47 tag, after stopping what's
    /// speaking, and returns once it's played or been cut off.
    public var speak: @Sendable (_ text: String, _ language: String) async throws(SpeechError) -> Void = {
        _, _ throws(SpeechError) in
    }
    // swiftlint:enable closure_parameter_position
    /// Stops what's speaking.
    public var stop: @Sendable () async -> Void
}

extension SpeechClient {
    /// Picks kept in `selection`, and `speak` for what's said.
    public static func picks(
        _ selection: VoiceSelection,
        speak: @escaping @Sendable (String, String) async throws(SpeechError) -> Void = { _, _ throws(SpeechError) in
        }, stop: @escaping @Sendable () async -> Void = {}
    ) -> Self {
        SpeechClient(
            choice: { language in
                guard let language else { return selection.choice() }
                return selection.hasOwnChoice(for: language) ? selection.choice(for: language) : nil
            },
            choose: { choice, language in selection.choose(choice, for: language) },
            forget: { id in selection.forget(id) },
            speak: speak, stop: stop)
    }

    /// Picks in memory, saying nothing.
    public static var inMemory: Self { .picks(VoiceSelection(file: nil)) }
}

extension SpeechClient: TestDependencyKey {
    public static let testValue = SpeechClient()
    public static var previewValue: SpeechClient { .inMemory }
}

extension DependencyValues {
    public var speech: SpeechClient {
        get { self[SpeechClient.self] }
        set { self[SpeechClient.self] = newValue }
    }
}
