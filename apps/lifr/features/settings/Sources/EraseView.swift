import ComposableArchitecture
import SwiftUI

/// Says what deleting all data does here, on other devices and in iCloud,
/// and deletes only once the confirming word is typed.
package struct EraseView: View {
    @Bindable package var store: StoreOf<EraseFeature>
    @FocusState private var focused: Bool

    package init(store: StoreOf<EraseFeature>) {
        self.store = store
    }

    package var body: some View {
        NavigationStack {
            Form {
                Section {
                    Text(
                        """
                        Every Avenue, Project, task, label, comment and reminder is deleted from this device \
                        and from your iCloud, and your settings go back to their defaults. It can't be undone.
                        """)
                    Text(
                        """
                        Your other devices keep their copies until they next sync, then delete them too. One \
                        that's offline deletes them when it's back. Past changes in Activity stay, since that \
                        record is never rewritten. Conversations aren't affected.
                        """
                    )
                    .foregroundStyle(.secondary)
                }
                Section {
                    TextField(EraseFeature.State.word, text: $store.typed)
                        .focused($focused)
                        .autocorrectionDisabled()
                        .onSubmit { store.send(.eraseTapped) }
                        #if os(iOS)
                            .textInputAutocapitalization(.never)
                        #endif
                } header: {
                    Text("Type \(EraseFeature.State.word) to confirm")
                } footer: {
                    if let error = store.error {
                        Text("Nothing was deleted: \(error). Try again; if it keeps failing, restart Lifr.")
                            .foregroundStyle(.red)
                    }
                }
                Section {
                    Button(role: .destructive) {
                        store.send(.eraseTapped)
                    } label: {
                        if store.isErasing {
                            ProgressView()
                        } else {
                            Text("Delete All Data")
                        }
                    }
                    .disabled(!store.canErase)
                }
            }
            .formStyle(.grouped)
            .navigationTitle("Delete All Data")
            #if os(iOS)
                .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { store.send(.cancelTapped) }
                        .disabled(store.isErasing)
                }
            }
            .onAppear { focused = true }
        }
        .interactiveDismissDisabled(store.isErasing)
    }
}
