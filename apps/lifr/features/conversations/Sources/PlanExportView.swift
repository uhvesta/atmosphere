import ComposableArchitecture
import LifrAPI
import SwiftUI

/// Makes a prompt to plan with another assistant: what it's for, which
/// tasks it lists, and the prompt itself to copy or share.
struct PlanExportView: View {
    @Bindable var store: StoreOf<PlanExportFeature>

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("Goal", text: $store.goal, prompt: Text("Move house by March"), axis: .vertical)
                        .lineLimit(1...4)
                } footer: {
                    Text("What the other assistant plans. Leave it empty to say it there instead.")
                }
                Section {
                    Picker("Tasks Listed", selection: $store.listed) {
                        Text("None").tag(PlanPrompt.Scope.names)
                        Text("All Avenues").tag(PlanPrompt.Scope.everything)
                        ForEach(store.avenues) { avenue in
                            Text(avenue.name).tag(PlanPrompt.Scope.avenue(avenue.id))
                        }
                        ForEach(store.projects, id: \.project.id) { pair in
                            Text("\(pair.avenue.name) › \(pair.project.name)")
                                .tag(PlanPrompt.Scope.project(pair.project.id))
                        }
                    }
                    Toggle("Open Tasks Only", isOn: $store.openOnly)
                        .disabled(!store.listsTasks)
                    Toggle("Descriptions", isOn: $store.descriptions)
                        .disabled(!store.listsTasks)
                } footer: {
                    if store.listsTasks {
                        Text(
                            """
                            Listing tasks keeps the assistant from planning them again, but copies their titles \
                            out of Lifr, and their descriptions when that's on.
                            """)
                    } else {
                        Text("The prompt names your Avenues, Projects and labels only.")
                    }
                }
                Section {
                    Text(store.prompt)
                        .font(.footnote.monospaced())
                        .textSelection(.enabled)
                        .lineLimit(12)
                    Button {
                        store.send(.copyTapped)
                    } label: {
                        Label(store.copied ? "Copied" : "Copy Prompt", systemImage: "doc.on.doc")
                    }
                    ShareLink(item: store.prompt) {
                        Label("Share Prompt…", systemImage: "square.and.arrow.up")
                    }
                } header: {
                    Text("Prompt")
                } footer: {
                    if let preset = store.preset {
                        Text(
                            """
                            It carries the instructions of your skill \(preset.title). Paste it into any chat \
                            assistant, then copy its whole reply and choose Paste Plan Reply here to review it.
                            """)
                    } else {
                        Text(
                            """
                            Paste it into any chat assistant, then copy its whole reply and choose Paste Plan Reply \
                            here to review it.
                            """)
                    }
                }
            }
            .formStyle(.grouped)
            .navigationTitle("Plan with Another Assistant")
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { store.send(.doneTapped) }
                }
            }
        }
    }
}
