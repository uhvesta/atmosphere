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
        package var problem: String?
        /// Why it couldn't be read aloud.
        package var speechProblem: String?

        package var id: WordKey { key }

        package init(
            key: WordKey, word: WordSpan, language: Language, sentence: String, transliteration: String? = nil,
            note: WordNote? = nil
        ) {
            self.key = key
            self.word = word
            self.language = language
            self.sentence = sentence
            self.transliteration = transliteration
            self.note = note
        }
    }

    package enum Action: Equatable {
        case task
        case sayTapped
        case saySentenceTapped
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
                guard state.note == nil else { return .none }
                state.problem = nil
                return .run { [word = state.word.text, sentence = state.sentence, language = state.language] send in
                    await send(
                        .lookedUp(
                            outcome { () throws(ModelRuntimeError) in
                                try await tutor.lookUp(word, in: sentence, language: language)
                            }))
                }
            case .sayTapped:
                return say(state.word.text, &state)
            case .saySentenceTapped:
                return say(state.sentence, &state)
            case .spoke(let problem):
                state.speechProblem = problem?.description
                return .none
            case .askTapped:
                return .send(.delegate(.ask))
            case .doneTapped:
                return .run { _ in await dismiss() }
            case .lookedUp(.success(let note)):
                state.note = note
                return .send(.delegate(.lookedUp(note)))
            case .lookedUp(.failure(let error)):
                state.problem = error.description
                return .none
            case .delegate:
                return .none
            }
        }
    }

    /// Reads `text` aloud in the card's language, in its voice.
    private func say(_ text: String, _ state: inout State) -> Effect<Action> {
        state.speechProblem = nil
        return .run { [language = state.language] send in
            do throws(SpeechError) {
                try await speech.speak(text, language.rawValue)
            } catch {
                await send(.spoke(error))
            }
        }
    }
}
