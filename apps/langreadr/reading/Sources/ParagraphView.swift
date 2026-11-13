import ComposableArchitecture
import LangReadrCore
import LangReadrServices
import SwiftUI

/// A paragraph as written, each word a button, with what's switched on
/// under it and its chats in the margin.
struct ParagraphView: View {
    let paragraph: Paragraph
    let store: StoreOf<ReaderFeature>

    private enum Piece: Identifiable {
        case word(WordSpan)
        case gap(String, at: Int)

        var id: Int {
            switch self {
            case .word(let word): word.start
            case .gap(_, let at): at
            }
        }
    }

    private var pieces: [Piece] {
        let text = Array(paragraph.text)
        var pieces: [Piece] = []
        var at = 0
        for word in store.words[paragraph.id] ?? [] {
            if word.start > at { pieces.append(.gap(String(text[at..<word.start]), at: at)) }
            pieces.append(.word(word))
            at = word.end
        }
        if at < text.count { pieces.append(.gap(String(text[at...]), at: at)) }
        return pieces
    }

    var body: some View {
        let book = store.book
        let annotation = book.annotations[paragraph.id]
        let picked = store.picked.flatMap { $0.paragraph == paragraph.id ? $0 : nil }
        let chats = book.chats(in: paragraph.id)
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .top, spacing: 8) {
                WordFlow {
                    ForEach(pieces) { piece in
                        switch piece {
                        case .gap(let text, _):
                            Text(text).accessibilityHidden(true)
                        case .word(let word):
                            Button {
                                store.send(.wordTapped(paragraph: paragraph.id, word))
                            } label: {
                                // Underlined once looked up here.
                                Text(word.text).underline(
                                    book.words[WordKey(paragraph: paragraph.id, word: word.text)] != nil,
                                    pattern: .dot)
                            }
                            .buttonStyle(.plain)
                            .background(highlight(word, picked: picked, chats: chats))
                        }
                    }
                }
                .font(.title3)
                .frame(maxWidth: .infinity, alignment: .leading)
                if !chats.isEmpty {
                    Button {
                        store.send(.chatBubbleTapped(paragraph: paragraph.id))
                    } label: {
                        Image(systemName: "bubble.left.fill")
                            .foregroundStyle(.tint)
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(
                        chats.count == 1 ? "1 chat about this paragraph" : "\(chats.count) chats about this paragraph")
                }
            }
            if store.showsTransliteration, let reading = store.state.transliteration(of: paragraph.id) {
                Text(reading)
                    .font(.callout)
                    .italic()
                    .foregroundStyle(.secondary)
            }
            if store.showsTranslation, let translation = annotation?.translation {
                Text(translation)
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
            if let problem = store.problems[paragraph.id] {
                HStack {
                    Text(problem).font(.caption).foregroundStyle(.red)
                    Button("Try again") { store.send(.retryTapped(paragraph.id)) }.font(.caption)
                }
            } else if store.writing.contains(paragraph.id) {
                Label(store.showsTranslation ? "Translating…" : "Writing the reading…", systemImage: "ellipsis")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        // Asks for what's missing again when a switch or the language changes.
        .task(id: Writing(store.showsTranslation, store.showsTransliteration, store.book.language)) {
            store.send(.paragraphAppeared(paragraph.id))
        }
    }

    private func highlight(_ word: WordSpan, picked: Passage?, chats: [AnchoredChat]) -> some View {
        let isPicked = picked.map { $0.contains(word.start) } ?? false
        let isAsked = chats.contains { $0.passage.contains(word.start) }
        return RoundedRectangle(cornerRadius: 3)
            .fill(isPicked ? Color.accentColor.opacity(0.3) : isAsked ? Color.yellow.opacity(0.25) : .clear)
    }
}

/// What the model is asked to write under a paragraph.
private struct Writing: Hashable {
    let translation: Bool
    let transliteration: Bool
    let language: Language

    init(_ translation: Bool, _ transliteration: Bool, _ language: Language) {
        self.translation = translation
        self.transliteration = transliteration
        self.language = language
    }
}
