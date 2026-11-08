import ComposableArchitecture
import Foundation
import InferenceKit
import LangReadrCore
import LangReadrModels
import LangReadrServices

/// One book as it's read: the switches for what's written under each
/// paragraph, the model writing it one paragraph at a time, words to tap,
/// and passages to pick and ask about.
@Reducer
package struct ReaderFeature {
    @ObservableState
    package struct State: Equatable, Identifiable {
        package var book: Book
        package var showsTranslation = false
        package var showsTransliteration = false
        /// Whether taps pick a passage instead of looking words up.
        package var isHighlighting = false
        /// Each paragraph's words, found when the book opens.
        package var words: [UUID: [WordSpan]] = [:]
        /// Each paragraph in Latin letters by the transform, where it has a
        /// rule for the script.
        package var transforms: [UUID: String] = [:]
        /// Where the passage being picked starts.
        package var anchor: Anchor?
        package var selection: Passage?
        /// The paragraphs the model is to write for, the one it's on first.
        package var writing: [UUID] = []
        /// Why the model couldn't write for a paragraph.
        package var problems: [UUID: String] = [:]
        @Presents package var destination: Destination.State?

        package var id: UUID { book.id }

        package init(book: Book) { self.book = book }

        /// The passage picked so far, which may be one word.
        package var picked: Passage? {
            if let selection { return selection }
            guard let anchor, let paragraph = book.paragraph(anchor.paragraph) else { return nil }
            return Passage(in: paragraph, from: anchor.word.start, to: anchor.word.end)
        }

        /// The paragraph in Latin letters: the model's once written, or the
        /// transform's.
        package func transliteration(of paragraph: UUID) -> String? {
            book.annotations[paragraph]?.transliteration ?? transforms[paragraph]
        }

        /// Whether the book is in a script with letters other than Latin.
        package var canTransliterate: Bool { !Script.isLatin(book.paragraphs.first?.text ?? "") }

        /// The chat about a passage, open over the text.
        fileprivate func chatting(_ chat: AnchoredChat, compact: Bool, reader: Language) -> Destination.State {
            .chat(
                PassageChatFeature.State(
                    chat: chat, book: ChatBook(book, chat: chat), compact: compact, reader: reader))
        }

        fileprivate func needsTranslation(_ paragraph: UUID) -> Bool {
            showsTranslation && book.annotations[paragraph]?.translation == nil
        }

        fileprivate func needsReading(_ paragraph: Paragraph) -> Bool {
            showsTransliteration && book.annotations[paragraph.id]?.transliteration == nil
                && Script.transliteration(for: book.language) == .model && !Script.isLatin(paragraph.text)
        }

        fileprivate func needsModel(_ paragraph: Paragraph) -> Bool {
            needsTranslation(paragraph.id) || needsReading(paragraph)
        }
    }

    /// The word a passage being picked starts at.
    package struct Anchor: Hashable, Sendable {
        package var paragraph: UUID
        package var word: WordSpan
    }

    @Reducer
    package enum Destination {
        case word(WordCardFeature)
        case chat(PassageChatFeature)
        case chats(ChatsFeature)
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case task
        case paragraphAppeared(UUID)
        case retryTapped(UUID)
        case annotated(UUID, Result<Annotation, ModelRuntimeError>)
        case languagePicked(Language)
        case wordTapped(paragraph: UUID, WordSpan)
        case askTapped
        case cancelHighlightTapped
        case chatsTapped
        case chatBubbleTapped(paragraph: UUID)
        case destination(PresentationAction<Destination.Action>)
    }

    @Dependency(\.tutor) private var tutor
    @Dependency(\.readerServices) private var services
    @Dependency(\.speech) private var speech
    @Dependency(\.date.now) private var now

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .binding(\.showsTranslation):
                if state.showsTranslation { state.book.translate(into: tutor.reader) }
                return .none
            case .binding(\.isHighlighting):
                if !state.isHighlighting { clearSelection(&state) }
                return .none
            case .binding:
                return .none
            case .task:
                findWords(&state)
                return .none
            case .paragraphAppeared(let id):
                state.book.lastRead = id
                guard let paragraph = state.book.paragraph(id), state.needsModel(paragraph),
                    state.problems[id] == nil, !state.writing.contains(id)
                else { return .none }
                state.writing.append(id)
                return state.writing.count == 1 ? annotate(paragraph, state) : .none
            case .retryTapped(let id):
                state.problems[id] = nil
                return .send(.paragraphAppeared(id))
            case .annotated(let id, .success(let annotation)):
                state.book.annotate(
                    id, translation: annotation.translation, transliteration: annotation.transliteration)
                return next(after: id, &state)
            case .annotated(let id, .failure(let error)):
                state.problems[id] = error.description
                return next(after: id, &state)
            case .languagePicked(let language):
                guard language != state.book.language else { return .none }
                state.book.language = language
                state.book.annotations = [:]
                state.book.words = [:]
                state.problems = [:]
                state.writing = []
                findWords(&state)
                return .none
            case .wordTapped(let id, let word):
                guard let paragraph = state.book.paragraph(id) else { return .none }
                if state.isHighlighting {
                    pick(word, in: paragraph, &state)
                    return .none
                }
                let key = WordKey(paragraph: id, word: word.text)
                state.destination = .word(
                    WordCardFeature.State(
                        key: key, word: word, language: state.book.language,
                        sentence: Self.sentence(around: word, in: paragraph.text),
                        transliteration: Script.transliteration(for: state.book.language) == .transform
                            ? services.transliterate(word.text) : nil,
                        note: state.book.words[key]))
                // The card says why if it can't; here it just doesn't.
                return .run { [language = state.book.language] _ in
                    try? await speech.speak(word.text, language.rawValue)
                }
            case .askTapped:
                guard let passage = state.picked else { return .none }
                state.isHighlighting = false
                clearSelection(&state)
                open(passage, &state)
                return .none
            case .cancelHighlightTapped:
                state.isHighlighting = false
                clearSelection(&state)
                return .none
            case .chatsTapped:
                state.destination = .chats(ChatsFeature.State(chats: state.book.chats))
                return .none
            case .chatBubbleTapped(let paragraph):
                guard let chat = state.book.chats(in: paragraph).last else { return .none }
                state.destination = state.chatting(chat, compact: true, reader: tutor.reader)
                return .none
            case .destination(.presented(.word(.delegate(.lookedUp(let note))))):
                guard case .word(let card)? = state.destination else { return .none }
                state.book.words[card.key] = note
                return .none
            case .destination(.presented(.word(.delegate(.ask)))):
                guard case .word(let card)? = state.destination,
                    let paragraph = state.book.paragraph(card.key.paragraph),
                    let passage = Passage(in: paragraph, from: card.word.start, to: card.word.end)
                else { return .none }
                open(passage, &state)
                return .none
            case .destination(.presented(.chat(.delegate(.changed(let chat))))):
                state.book.save(chat)
                return .none
            case .destination(.presented(.chats(.chatTapped(let id)))):
                guard let chat = state.book.chats.first(where: { $0.id == id }) else { return .none }
                state.destination = state.chatting(chat, compact: false, reader: tutor.reader)
                return .none
            case .destination:
                return .none
            }
        }
        .ifLet(\.$destination, action: \.destination)
    }

    private func findWords(_ state: inout State) {
        let language = state.book.language
        let transforms = Script.transliteration(for: language) == .transform
        state.words = [:]
        state.transforms = [:]
        for paragraph in state.book.paragraphs {
            state.words[paragraph.id] = services.words(paragraph.text, language)
            if transforms, let reading = services.transliterate(paragraph.text) {
                state.transforms[paragraph.id] = reading
            }
        }
    }

    /// Has the model write what `paragraph` is missing.
    private func annotate(_ paragraph: Paragraph, _ state: State) -> Effect<Action> {
        let translate = state.needsTranslation(paragraph.id)
        let read = state.needsReading(paragraph)
        return .run { [tutor, language = state.book.language] send in
            let written = await outcome { () throws(ModelRuntimeError) in
                Annotation(
                    translation: translate ? try await tutor.translate(paragraph.text, from: language) : nil,
                    transliteration: read ? try await tutor.romanize(paragraph.text, from: language) : nil)
            }
            await send(.annotated(paragraph.id, written))
        }
    }

    /// Moves on from `id` to the next paragraph that still needs the model.
    private func next(after id: UUID, _ state: inout State) -> Effect<Action> {
        state.writing.removeAll { $0 == id }
        while let first = state.writing.first {
            if let paragraph = state.book.paragraph(first), state.needsModel(paragraph) {
                return annotate(paragraph, state)
            }
            state.writing.removeFirst()
        }
        return .none
    }

    /// The first tap picks one word; the next, in the same paragraph,
    /// stretches the passage to there, and the one after starts again.
    private func pick(_ word: WordSpan, in paragraph: Paragraph, _ state: inout State) {
        if let anchor = state.anchor, anchor.paragraph == paragraph.id, state.selection == nil {
            state.selection = Passage(
                in: paragraph, from: min(anchor.word.start, word.start), to: max(anchor.word.end, word.end))
        } else {
            state.anchor = Anchor(paragraph: paragraph.id, word: word)
            state.selection = nil
        }
    }

    private func clearSelection(_ state: inout State) {
        state.anchor = nil
        state.selection = nil
    }

    private func open(_ passage: Passage, _ state: inout State) {
        let chat = state.book.chat(about: passage, at: now)
        state.destination = state.chatting(chat, compact: true, reader: tutor.reader)
    }

    /// The sentence of `text` that `word` is in.
    static func sentence(around word: WordSpan, in text: String) -> String {
        let characters = Array(text)
        let ends: Set<Character> = [".", "!", "?", "。", "！", "？", "\n", "।", "؟"]
        var start = min(word.start, characters.count)
        while start > 0, !ends.contains(characters[start - 1]) { start -= 1 }
        var end = min(word.end, characters.count)
        while end < characters.count, !ends.contains(characters[end]) { end += 1 }
        if end < characters.count { end += 1 }
        return String(characters[start..<end]).trimmingCharacters(in: .whitespacesAndNewlines)
    }
}

extension ReaderFeature.Destination.State: Equatable {}
