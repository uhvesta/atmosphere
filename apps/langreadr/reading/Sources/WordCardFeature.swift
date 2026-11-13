import ComposableArchitecture
import Foundation
import InferenceKit
import LangReadrCore
import LangReadrModels
import LangReadrServices

/// A word the person tapped: how it's read and what it means, there and
/// in general, looked up by the model the first time.
@Reducer
package struct WordCardFeature {
    @ObservableState
    package struct State: Equatable, Identifiable {
        package var key: WordKey
        package var word: WordSpan
        package var language: Language
        /// The sentence it was read in.
        package var sentence: String
        /// Its reading by the transform, shown until the model's.
        package var transliteration: String?
        package var note: WordNote?
        /// What the model said of the word in another sentence of the
        /// book, shown instead of looking it up again until the person
        /// asks what it means here.
        package var known: WordNote?
        package var problem: String?
        /// Whether the model is looking it up now.
        package var isLookingUp = false
        /// Why it couldn't be read aloud.
        package var speechProblem: String?

        package var id: WordKey { key }

        package init(
            key: WordKey, word: WordSpan, language: Language, sentence: String, transliteration: String? = nil,
            note: WordNote? = nil, known: WordNote? = nil
        ) {
            self.key = key
            self.word = word
            self.language = language
            self.sentence = sentence
            self.transliteration = transliteration
            self.note = note
            self.known = note == nil ? known : nil
        }
    }

    package enum Action: Equatable {
        case task
        /// Looks it up, for its sentence here, after a failure or when only
        /// another sentence's meaning is known.
        case lookUpTapped
        case sayTapped
        case saySlowlyTapped
        case saySentenceTapped
        case saySentenceSlowlyTapped
        case spoke(SpeechError?)
        case askTapped
        case doneTapped
        case lookedUp(Result<WordNote, ModelRuntimeError>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate: Equatable {
            /// The model said what it means, for the book to keep.
            case lookedUp(WordNote)
            /// The person wants to ask about it.
            case ask
        }
    }

    @Dependency(\.tutor) private var tutor
    @Dependency(\.speech) private var speech
    @Dependency(\.dismiss) private var dismiss

    package init() {}

    package var body: some ReducerOf<Self> {
        Reduce { state, action in
            switch action {
            case .task:
                guard state.note == nil, state.known == nil else { return .none }
                return lookUp(&state)
            case .lookUpTapped:
                guard state.note == nil else { return .none }
                return lookUp(&state)
            case .sayTapped:
                return say(state.word.text, slowly: false, &state)
            case .saySlowlyTapped:
                return say(state.word.text, slowly: true, &state)
            case .saySentenceTapped:
                return say(state.sentence, slowly: false, &state)
            case .saySentenceSlowlyTapped:
                return say(state.sentence, slowly: true, &state)
            case .spoke(let problem):
                state.speechProblem = problem?.description
                return .none
            case .askTapped:
                return .send(.delegate(.ask))
            case .doneTapped:
                return .run { _ in await dismiss() }
            case .lookedUp(.success(let note)):
                state.isLookingUp = false
                state.note = note
                state.known = nil
                return .send(.delegate(.lookedUp(note)))
            case .lookedUp(.failure(let error)):
                state.isLookingUp = false
                state.problem = error.reading
                return .none
            case .delegate:
                return .none
            }
        }
    }

    private func lookUp(_ state: inout State) -> Effect<Action> {
        state.problem = nil
        state.isLookingUp = true
        return .run { [word = state.word.text, sentence = state.sentence, language = state.language] send in
            await send(
                .lookedUp(
                    outcome { () throws(ModelRuntimeError) in
                        try await tutor.lookUp(word, in: sentence, language: language)
                    }))
        }
    }

    /// Reads `text` aloud in the card's language, in its voice.
    private func say(_ text: String, slowly: Bool, _ state: inout State) -> Effect<Action> {
        state.speechProblem = nil
        return .run { [language = state.language] send in
            do throws(SpeechError) {
                if slowly {
                    try await speech.speakSlowly(text, language.rawValue)
                } else {
                    try await speech.speak(text, language.rawValue)
                }
            } catch {
                await send(.spoke(error))
            }
        }
    }
}
