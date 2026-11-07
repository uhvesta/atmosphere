import ChatUI
import ComposableArchitecture
import LifrModelsClient
import SwiftUI

/// What one conversation runs on: each choice starts at the default from
/// Settings, and Settings stays as it is.
struct ChatModelView: View {
    @Bindable var store: StoreOf<ChatModelFeature>

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Picker("Model", selection: $store.model) {
                        Text("Default (\(store.state.standardName ?? "…"))").tag(ModelID?.none)
                        ForEach(store.models) { model in
                            Text(model.name).tag(ModelID?.some(model.id))
                        }
                    }
                } footer: {
                    Text("Only for this chat. Download more models in Settings › Models.")
                }
                if store.state.current?.thinks ?? false {
                    Section {
                        Picker("Thinking", selection: $store.choice.thinking) {
                            Text("Default (On)").tag(ThinkingLevel?.none)
                            Text("Off").tag(ThinkingLevel?.some(.off))
                            Text("On").tag(ThinkingLevel?.some(.on))
                            Text("Long").tag(ThinkingLevel?.some(.long))
                        }
                    } footer: {
                        Text("Long thinks up to four times as long before it answers, for harder questions.")
                    }
                }
                if let current = store.state.current, !current.windows.isEmpty {
                    Section {
                        Picker("Context Window", selection: $store.choice.window) {
                            Text("Default (\(tokens(current.window)))").tag(Int?.none)
                            ForEach(current.windows, id: \.self) { window in
                                Text(tokens(window)).tag(Int?.some(window))
                            }
                        }
                    } footer: {
                        Text("A larger window remembers more of the chat and takes more memory.")
                    }
                }
                Section {
                    Button("Use the Defaults") { store.send(.resetTapped) }
                        .disabled(store.choice == .defaults)
                }
            }
            .formStyle(.grouped)
            .navigationTitle("This Chat's Model")
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { store.send(.doneTapped) }
                }
            }
        }
        .sheetLayout(height: 440)
        .task { await store.send(.task).finish() }
    }

    private func tokens(_ count: Int) -> String {
        String(localized: "\(count.formatted()) tokens")
    }
}
