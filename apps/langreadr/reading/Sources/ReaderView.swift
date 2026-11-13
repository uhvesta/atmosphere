import ComposableArchitecture
import LangReadrCore
import SwiftUI

/// A book to read: its chapters, each paragraph's words to tap, and the
/// switches for what's written under them.
struct ReaderView: View {
    @Bindable var store: StoreOf<ReaderFeature>

    /// Where it opens: the paragraph the person read last.
    @State private var position = ScrollPosition(idType: UUID.self)

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 18) {
                ForEach(store.book.chapters) { chapter in
                    if let title = chapter.title {
                        Text(title).font(.title2.bold()).padding(.top, 6)
                    }
                    ForEach(chapter.paragraphs) { paragraph in
                        ParagraphView(paragraph: paragraph, store: store)
                    }
                }
            }
            .scrollTargetLayout()
            .padding()
            .frame(maxWidth: 720, alignment: .leading)
            .frame(maxWidth: .infinity)
        }
        .scrollPosition($position, anchor: .top)
        .onScrollTargetVisibilityChange(idType: UUID.self, threshold: 0.5) { shown in
            store.send(.paragraphsShown(shown))
        }
        .onAppear {
            if let place = store.book.lastRead { position.scrollTo(id: place, anchor: .top) }
        }
        .navigationTitle(store.book.title)
        .task { await store.send(.task).finish() }
        .toolbar { toolbar }
        .safeAreaInset(edge: .bottom) {
            if store.isHighlighting { highlightBar }
        }
        .alert($store.scope(state: \.alert, action: \.alert))
        .sheet(item: $store.scope(state: \.destination, action: \.destination)) { destination in
            ReaderSheetView(store: destination)
        }
    }

    @ToolbarContentBuilder private var toolbar: some ToolbarContent {
        ToolbarItemGroup(placement: .primaryAction) {
            Toggle(isOn: $store.showsTranslation) { Label("Translate", systemImage: "character.book.closed") }
            if store.canTransliterate {
                Toggle(isOn: $store.showsTransliteration) { Label("Transliterate", systemImage: "textformat.abc") }
            }
            Toggle(isOn: $store.isHighlighting) { Label("Highlight", systemImage: "highlighter") }
            Menu {
                Picker(
                    "Written in",
                    selection: Binding(get: { store.book.language }, set: { store.send(.languagePicked($0)) })
                ) {
                    ForEach(Self.languages(including: store.book.language), id: \.self) { language in
                        Text(language.name()).tag(language)
                    }
                }
                let lookedUp = store.state.lookedUp
                if !lookedUp.isEmpty {
                    Menu("Words (\(lookedUp.count))", systemImage: "character.book.closed") {
                        ForEach(lookedUp, id: \.key) { word in
                            Button(word.word.text) { store.send(.lookedUpWordTapped(word.key)) }
                        }
                    }
                }
                if !store.book.chats.isEmpty {
                    Button("Chats (\(store.book.chats.count))", systemImage: "bubble.left.and.bubble.right") {
                        store.send(.chatsTapped)
                    }
                }
            } label: {
                Label("More", systemImage: "ellipsis.circle")
            }
        }
    }

    private var highlightBar: some View {
        HStack {
            if let passage = store.picked {
                Text("“\(passage.text)”").lineLimit(1).truncationMode(.middle)
                Spacer()
                Button("Ask") { store.send(.askTapped) }.buttonStyle(.borderedProminent)
                Button("Copy") { copy(passage.text) }
            } else {
                Text("Tap where the passage starts, then where it ends.").foregroundStyle(.secondary)
                Spacer()
            }
            Button("Cancel") { store.send(.cancelHighlightTapped) }
        }
        .padding()
        .background(.bar)
    }

    private func copy(_ text: String) {
        #if os(macOS)
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(text, forType: .string)
        #else
            UIPasteboard.general.string = text
        #endif
    }

    /// The languages to pick from: those people most often learn, and the
    /// book's own.
    static func languages(including language: Language) -> [Language] {
        let common = [
            "ar", "de", "en", "es", "fa", "fr", "he", "hi", "it", "ja", "ko", "nl", "pl", "pt", "ru", "sv", "th",
            "tr", "uk", "vi", "zh-Hans", "zh-Hant",
        ].map { Language($0) }
        let all = common.contains(language) ? common : common + [language]
        return all.sorted { $0.name() < $1.name() }
    }
}

/// What opens over the text.
struct ReaderSheetView: View {
    let store: StoreOf<ReaderFeature.Destination>

    var body: some View {
        NavigationStack {
            switch store.case {
            case .word(let card): WordCardView(store: card)
            case .chat(let chat): PassageChatView(store: chat)
            case .chats(let chats): ChatsView(store: chats)
            }
        }
        #if os(iOS)
            .presentationDetents(isCompact ? [.medium, .large] : [.large])
        #else
            .frame(minWidth: 420, minHeight: isCompact ? 360 : 560)
        #endif
    }

    private var isCompact: Bool {
        switch store.state {
        case .word: true
        case .chat(let chat): chat.compact
        case .chats: false
        }
    }
}

extension ReaderFeature.Destination.State: Identifiable {
    package var id: String {
        switch self {
        case .word(let card): "word \(card.id)"
        case .chat(let chat): "chat \(chat.id)"
        case .chats: "chats"
        }
    }
}
