import ComposableArchitecture
import SwiftUI

struct RenameView: View {
    @Bindable var store: StoreOf<RenameFeature>

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("Name", text: $store.name, prompt: Text("The first message"))
                        .onSubmit { store.send(.saveTapped) }
                } footer: {
                    Text("Leave it empty to use the first message.")
                }
                Section {
                    Button {
                        store.send(.suggestTapped)
                    } label: {
                        HStack {
                            if let suggester = store.suggester {
                                Label(
                                    "Suggest with \(suggester.name)",
                                    systemImage: suggester.isApple ? "apple.intelligence" : "sparkles")
                            } else {
                                Label("Suggest a Name", systemImage: "sparkles")
                            }
                            if store.suggesting {
                                Spacer()
                                ProgressView()
                            }
                        }
                    }
                    .disabled(!store.canSuggest)
                } footer: {
                    if let problem = store.problem {
                        Text(problem)
                    }
                }
            }
            .navigationTitle("Rename")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { store.send(.cancelTapped) }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") { store.send(.saveTapped) }
                }
            }
        }
        .presentationDetents([.medium])
        .task { await store.send(.task).finish() }
    }
}
