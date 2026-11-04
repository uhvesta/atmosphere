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
                            Label("Suggest with Apple Intelligence", systemImage: "apple.intelligence")
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
    }
}
