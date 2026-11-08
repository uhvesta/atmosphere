import ComposableArchitecture
import LangReadrCore
import SwiftUI

/// The word tapped: how it's read, what it means, and what it means here.
struct WordCardView: View {
    let store: StoreOf<WordCardFeature>

    var body: some View {
        List {
            Section {
                HStack(alignment: .firstTextBaseline) {
                    VStack(alignment: .leading, spacing: 4) {
                        Text(store.word.text).font(.largeTitle)
                        if let reading = store.note?.transliteration ?? store.transliteration {
                            Text(reading).font(.title3).foregroundStyle(.secondary)
                        }
                    }
                    Spacer()
                    Button("Say it", systemImage: "speaker.wave.2.fill") { store.send(.sayTapped) }
                        .labelStyle(.iconOnly)
                        .font(.title2)
                        .buttonStyle(.borderless)
                }
            }
            if let note = store.note {
                Section("Meaning") {
                    Text(note.meaning)
                }
                if !note.inSentence.isEmpty {
                    Section("Here") {
                        Text(note.inSentence)
                    }
                }
            } else if let problem = store.problem {
                Section { Text(problem).foregroundStyle(.red) }
            } else {
                Section { Label("Looking it up…", systemImage: "ellipsis") }
            }
            Section {
                Text(store.sentence).foregroundStyle(.secondary)
                Button("Say the sentence", systemImage: "speaker.wave.2") { store.send(.saySentenceTapped) }
                Button("Ask about it", systemImage: "bubble.left") { store.send(.askTapped) }
                if let problem = store.speechProblem {
                    Text(problem).font(.caption).foregroundStyle(.red)
                }
            }
        }
        .navigationTitle(store.word.text)
        #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
        #endif
        .task { await store.send(.task).finish() }
        .toolbar {
            ToolbarItem(placement: .cancellationAction) { Button("Done") { store.send(.doneTapped) } }
        }
    }
}
