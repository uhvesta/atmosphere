import InferenceKit
import SwiftUI

/// Picks a model for one thing, such as a chat or a sheet, from Apple's
/// and the downloaded ones, or the default: nil.
public struct ModelPicker: View {
    @Binding var selection: ModelID?
    let models: [RunnableModel]
    /// The default's name, or nil while it's looked up.
    let standard: String?

    public init(selection: Binding<ModelID?>, models: [RunnableModel], standard: String?) {
        _selection = selection
        self.models = models
        self.standard = standard
    }

    public var body: some View {
        Picker("Model", selection: $selection) {
            Text("Default (\(standard ?? "…"))").tag(ModelID?.none)
            ForEach(models) { model in
                Text(model.name).tag(ModelID?.some(model.id))
            }
        }
    }
}

/// What one chat runs on: its model, how long that thinks and its window,
/// each the default until picked, in a sheet of its own.
public struct ModelChoiceForm: View {
    @Binding var model: ModelID?
    @Binding var thinking: ThinkingLevel?
    @Binding var window: Int?
    let models: [RunnableModel]
    let standard: ModelID?
    let reset: () -> Void
    let done: () -> Void

    /// `models` are those it can run on, `standard` the default model;
    /// `reset` goes back to every default.
    public init(
        model: Binding<ModelID?>, thinking: Binding<ThinkingLevel?>, window: Binding<Int?>, models: [RunnableModel],
        standard: ModelID?, reset: @escaping () -> Void, done: @escaping () -> Void
    ) {
        _model = model
        _thinking = thinking
        _window = window
        self.models = models
        self.standard = standard
        self.reset = reset
        self.done = done
    }

    /// The model the chat runs on, once it's looked up.
    private var current: RunnableModel? {
        let id = model ?? standard
        return models.first { $0.id == id }
    }

    public var body: some View {
        NavigationStack {
            Form {
                Section {
                    ModelPicker(
                        selection: $model, models: models, standard: models.first { $0.id == standard }?.name)
                } footer: {
                    Text("Only for this chat. Download more models in Settings › Models.")
                }
                if current?.thinks ?? false {
                    Section {
                        Picker("Thinking", selection: $thinking) {
                            Text("Default (On)").tag(ThinkingLevel?.none)
                            Text("Off").tag(ThinkingLevel?.some(.off))
                            Text("On").tag(ThinkingLevel?.some(.on))
                            Text("Long").tag(ThinkingLevel?.some(.long))
                        }
                    } footer: {
                        Text("Long thinks up to four times as long before it answers, for harder questions.")
                    }
                }
                if let current, !current.windows.isEmpty {
                    Section {
                        Picker("Context Window", selection: $window) {
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
                    Button("Use the Defaults", action: reset)
                        .disabled(model == nil && thinking == nil && window == nil)
                }
            }
            .formStyle(.grouped)
            .navigationTitle("This Chat's Model")
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done", action: done)
                }
            }
        }
        .sheetLayout(height: 440)
    }

    private func tokens(_ count: Int) -> String {
        String(localized: "\(count.formatted()) tokens")
    }
}
