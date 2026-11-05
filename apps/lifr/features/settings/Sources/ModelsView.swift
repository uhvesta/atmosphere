import ComposableArchitecture
import LifrModelsClient
import SwiftUI

package struct ModelsView: View {
    @Bindable package var store: StoreOf<ModelsFeature>
    @Shared(.answersInBackground) private var answersInBackground

    package init(store: StoreOf<ModelsFeature>) {
        self.store = store
    }

    package var body: some View {
        Form {
            Section {
                ForEach(ModelUse.allCases, id: \.self) { use in
                    Picker(
                        use.title,
                        selection: Binding(
                            get: { store.state.selected(for: use) }, set: { store.send(.picked($0, for: use)) })
                    ) {
                        ForEach(store.choices) { Text($0.name).tag($0.id) }
                    }
                }
            } header: {
                Text("Runs on")
            } footer: {
                Text("Downloaded models run on this device, like Apple's. Nothing you write leaves it.")
            }
            #if os(iOS)
                // The Mac doesn't suspend apps, so a model there always
                // goes on answering.
                Section {
                    Toggle("Answer in the background", isOn: Binding($answersInBackground))
                } footer: {
                    Text(
                        "When you leave Lifr mid-answer, a downloaded model keeps going on the iPhone's CPU, "
                            + "which is slower and uses more battery, and Lifr shows its progress on the Lock "
                            + "Screen. Off, the model stops and carries on where it was when you come back.")
                }
            #endif
            Section {
                ForEach(store.downloadable) { model in
                    ModelRow(model: model, store: store)
                }
            } header: {
                Text("Models")
            }
            Section {
                TextField("Hugging Face link", text: $store.link)
                    .textContentType(.URL)
                    .autocorrectionDisabled()
                    #if os(iOS)
                        .keyboardType(.URL)
                        .textInputAutocapitalization(.never)
                    #endif
                TextField("SHA-256 (optional)", text: $store.sha256)
                    .autocorrectionDisabled()
                    #if os(iOS)
                        .textInputAutocapitalization(.never)
                    #endif
                if let error = store.addError {
                    Text(error).foregroundStyle(.red)
                }
                Button {
                    store.send(.addTapped)
                } label: {
                    if store.isAdding {
                        ProgressView()
                    } else {
                        Text("Download model")
                    }
                }
                .disabled(!store.canAdd)
            } header: {
                Text("From Hugging Face")
            } footer: {
                Text(
                    "Paste the link to a .litertlm model, such as one from litert-community. Lifr pins it to the "
                        + "current commit and checks every file. Give the SHA-256 its publisher lists to be sure "
                        + "it's theirs.")
            }
        }
        .navigationTitle("Models")
        .task { await store.send(.task).finish() }
        .confirmationDialog($store.scope(state: \.remove, action: \.remove))
        .alert($store.scope(state: \.alert, action: \.alert))
    }
}

private struct ModelRow: View {
    let model: ModelSpec
    let store: StoreOf<ModelsFeature>

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .firstTextBaseline) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(model.name).font(.headline)
                    Text(details).font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                action
            }
            progress
        }
        .buttonStyle(.borderless)
    }

    private var details: String {
        "\(model.summary) · \(Self.bytes(model.downloadSize)) · \(model.license)"
    }

    private static func bytes(_ count: Int64) -> String {
        ByteCountFormatter.string(fromByteCount: count, countStyle: .file)
    }

    private var isInstalled: Bool { store.installed.contains(model.id) }

    @ViewBuilder private var action: some View {
        switch store.downloads[model.id] {
        case .downloading:
            Button("Pause") { store.send(.pauseTapped(model.id)) }
        case .verifying:
            Text("Checking").foregroundStyle(.secondary)
        case .paused:
            Button("Resume") { store.send(.downloadTapped(model.id)) }
        case .failed:
            Button("Download model") { store.send(.downloadTapped(model.id)) }
        case nil:
            if isInstalled {
                Button("Remove", role: .destructive) { store.send(.removeTapped(model.id)) }
            } else {
                Button("Download model") { store.send(.downloadTapped(model.id)) }
            }
        }
    }

    @ViewBuilder private var progress: some View {
        switch store.downloads[model.id] {
        case .downloading(let received, let total), .paused(let received, let total):
            ProgressView(value: store.downloads[model.id]?.fraction ?? 0) {
                EmptyView()
            } currentValueLabel: {
                Text("\(Self.bytes(received)) of \(Self.bytes(total))")
            }
        case .verifying:
            ProgressView(value: 1)
        case .failed(let error):
            Text(error.description).font(.caption).foregroundStyle(.red)
        case nil:
            if isInstalled {
                Text("Downloaded").font(.caption).foregroundStyle(.secondary)
            }
        }
    }
}

#Preview {
    NavigationStack {
        ModelsView(store: Store(initialState: ModelsFeature.State()) { ModelsFeature() })
    }
}
