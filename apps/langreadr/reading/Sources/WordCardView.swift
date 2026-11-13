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
                        if let reading {
                            Text(reading).font(.title3).foregroundStyle(.secondary)
                        }
                    }
                    Spacer()
                    Button("Say it slowly", systemImage: "tortoise") { store.send(.saySlowlyTapped) }
                        .labelStyle(.iconOnly)
                        .font(.title2)
                        .buttonStyle(.borderless)
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
            } else if let known = store.known {
                Section("Meaning") {
                    Text(known.meaning)
                }
                Section("Here") {
                    if store.isLookingUp {
                        lookingUp
                    } else if let problem = store.problem {
                        failed(problem)
                    } else {
                        Button("What it means here", systemImage: "text.magnifyingglass") {
                            store.send(.lookUpTapped)
                        }
                    }
                }
            } else if let problem = store.problem {
                Section { failed(problem) }
            } else {
                Section { lookingUp }
            }
            Section {
                Text(store.sentence).foregroundStyle(.secondary)
                Button("Say the sentence", systemImage: "speaker.wave.2") { store.send(.saySentenceTapped) }
                Button("Say the sentence slowly", systemImage: "tortoise") { store.send(.saySentenceSlowlyTapped) }
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

    /// How it's read: the model's, or else the transform's.
    private var reading: String? {
        store.note?.transliteration ?? store.known?.transliteration ?? store.transliteration
    }

    private var lookingUp: some View {
        Label("Looking it up…", systemImage: "ellipsis")
    }

    private func failed(_ problem: String) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(problem).foregroundStyle(.red)
            Button("Try again", systemImage: "arrow.clockwise") { store.send(.lookUpTapped) }
                .buttonStyle(.borderless)
        }
    }
}
