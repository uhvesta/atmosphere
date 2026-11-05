import ComposableArchitecture
import LifrAssistantClient
import SwiftUI

package struct InstructionsView: View {
    @Bindable package var store: StoreOf<InstructionsFeature>

    package init(store: StoreOf<InstructionsFeature>) {
        self.store = store
    }

    package var body: some View {
        Form {
            Section {
                TextEditor(text: $store.draft)
                    .frame(minHeight: 160)
                    .disabled(!store.isLoaded)
                    .accessibilityLabel("Instructions")
            } header: {
                Text("How the assistant should behave")
            } footer: {
                VStack(alignment: .leading, spacing: 4) {
                    Text("\(store.count) of \(Guidance.limit) characters")
                        .foregroundStyle(store.isTooLong ? .red : .secondary)
                    Text(
                        "Lifr also tells the assistant about your records, the tools it can use and today's date, "
                            + "so drafts and changes keep working whatever you write.")
                }
            }
            if let error = store.error {
                Section { Text(error.description).foregroundStyle(.red) }
            }
            Section {
                row(String(localized: "Lifr's default"), detail: nil, isCurrent: store.guidance.isDefault) {
                    store.send(.useTapped(nil))
                }
                ForEach(store.guidance.history) { version in
                    row(
                        version.text, detail: version.savedAt.formatted(date: .abbreviated, time: .shortened),
                        isCurrent: version.id == store.guidance.current
                    ) {
                        store.send(.useTapped(version.id))
                    }
                }
            } header: {
                Text("History")
            } footer: {
                Text("Every version you save is kept on this device only. Tap one to use it again.")
            }
        }
        .navigationTitle("Instructions")
        .toolbar {
            ToolbarItem(placement: .confirmationAction) {
                Button("Save") { store.send(.saveTapped) }
                    .disabled(!store.canSave)
            }
        }
        .task { await store.send(.task).finish() }
    }

    private func row(
        _ text: String, detail: String?, isCurrent: Bool, action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            HStack(alignment: .firstTextBaseline) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(text).lineLimit(2).foregroundStyle(Color.primary)
                    if let detail {
                        Text(detail).font(.footnote).foregroundStyle(Color.secondary)
                    }
                }
                Spacer()
                if isCurrent {
                    Image(systemName: "checkmark").foregroundStyle(Color.accentColor)
                        .accessibilityLabel("In use")
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(store.isSaving)
        .accessibilityAddTraits(isCurrent ? .isSelected : [])
    }
}
