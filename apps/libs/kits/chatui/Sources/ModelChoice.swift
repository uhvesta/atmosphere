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

/// What one chat runs on, picked in a sheet: its models in one card,
/// each with what it's good for and a check on the one it runs on, and
/// under them how long it thinks and its window, each saying which is
/// recommended. Picking the default of each goes back to following it.
public struct ModelSheet: View {
    @Binding var model: ModelID?
    @Binding var thinking: ThinkingLevel?
    @Binding var window: Int?
    let models: [RunnableModel]
    let standard: ModelID?
    let done: () -> Void

    /// `models` are those it can run on, `standard` the default model.
    public init(
        model: Binding<ModelID?>, thinking: Binding<ThinkingLevel?>, window: Binding<Int?>, models: [RunnableModel],
        standard: ModelID?, done: @escaping () -> Void
    ) {
        _model = model
        _thinking = thinking
        _window = window
        self.models = models
        self.standard = standard
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
                    ForEach(models) { choice in
                        Button {
                            model = choice.id == standard ? nil : choice.id
                        } label: {
                            ModelChoiceRow(
                                model: choice, isDefault: choice.id == standard, isPicked: choice.id == current?.id)
                        }
                        .buttonStyle(.plain)
                    }
                } footer: {
                    Text("Only for this chat. Download more models in Settings › Models.")
                }
                if current?.thinks ?? false {
                    Section {
                        OptionRow(
                            "Thinking", selection: $thinking, options: ThinkingLevel.allCases, recommended: .on,
                            title: \.title)
                    } footer: {
                        Text("Long thinks up to four times as long before it answers, for harder questions.")
                    }
                }
                if let current, !current.windows.isEmpty {
                    Section {
                        OptionRow(
                            "Context Window", selection: $window, options: current.windows,
                            recommended: current.window, title: Self.tokens)
                    } footer: {
                        Text("A larger window remembers more of the chat and takes more memory.")
                    }
                }
            }
            .formStyle(.grouped)
            .navigationTitle("Select Model")
            #if os(iOS)
                .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Close", systemImage: "xmark", action: done)
                        .labelStyle(.iconOnly)
                        .keyboardShortcut(.cancelAction)
                }
            }
        }
        .sheetLayout(height: 520)
        #if os(iOS)
            .presentationDetents([.medium, .large])
        #endif
    }

    private static func tokens(_ count: Int) -> String {
        String(localized: "\(count.formatted()) tokens")
    }
}

/// One model of the sheet: its name, what it's good for, and a check when
/// the chat runs on it.
private struct ModelChoiceRow: View {
    let model: RunnableModel
    let isDefault: Bool
    let isPicked: Bool

    var body: some View {
        HStack(spacing: 12) {
            VStack(alignment: .leading, spacing: 2) {
                // The tag goes under the name when both don't fit on a line.
                ViewThatFits(in: .horizontal) {
                    HStack(spacing: 6) { name }
                    VStack(alignment: .leading, spacing: 4) { name }
                }
                if !model.summary.isEmpty {
                    Text(model.summary)
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                }
            }
            Spacer(minLength: 0)
            if isPicked {
                Image(systemName: "checkmark")
                    .font(.body.weight(.semibold))
                    .foregroundStyle(.tint)
            }
        }
        .contentShape(.rect)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(isPicked ? .isSelected : [])
    }

    @ViewBuilder private var name: some View {
        Text(model.name)
            .fixedSize(horizontal: false, vertical: true)
        if isDefault {
            Tag(Text("Default"))
        }
    }
}

/// A choice of the sheet, such as how long the model thinks: its name, a
/// capsule when the recommended one is picked, and the one picked in gray,
/// which opens a menu of the others. nil is the recommended one, which
/// follows the default.
private struct OptionRow<Option: Hashable>: View {
    let name: LocalizedStringKey
    @Binding var selection: Option?
    let options: [Option]
    let recommended: Option
    let title: (Option) -> String

    init(
        _ name: LocalizedStringKey, selection: Binding<Option?>, options: [Option], recommended: Option,
        title: @escaping (Option) -> String
    ) {
        self.name = name
        _selection = selection
        self.options = options
        self.recommended = recommended
        self.title = title
    }

    private var picked: Option { selection ?? recommended }

    var body: some View {
        Menu {
            Picker(name, selection: Binding(get: { picked }, set: { selection = $0 == recommended ? nil : $0 })) {
                ForEach(options, id: \.self) { option in
                    Text(option == recommended ? "\(title(option)) (Recommended)" : "\(title(option))")
                        .tag(option)
                }
            }
            .pickerStyle(.inline)
        } label: {
            // The tag goes under the name when the row doesn't fit on a line.
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 8) {
                    Text(name).fixedSize()
                    tag
                    Spacer(minLength: 8)
                    value.fixedSize()
                }
                VStack(alignment: .leading, spacing: 4) {
                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        Text(name)
                        Spacer(minLength: 8)
                        value
                    }
                    tag
                }
            }
            .foregroundStyle(.primary)
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .menuIndicator(.hidden)
        .accessibilityLabel(Text(name))
        .accessibilityValue(Text(title(picked)))
    }

    @ViewBuilder private var tag: some View {
        if picked == recommended {
            Tag(Text("Recommended"))
        }
    }

    private var value: some View {
        HStack(spacing: 6) {
            Text(title(picked))
            Image(systemName: "chevron.up.chevron.down")
                .font(.footnote.weight(.medium))
        }
        .foregroundStyle(.secondary)
    }
}

/// A small gray label in a capsule, such as Recommended.
private struct Tag: View {
    let text: Text

    init(_ text: Text) {
        self.text = text
    }

    var body: some View {
        text
            .font(.caption.weight(.medium))
            .foregroundStyle(.secondary)
            .padding(.horizontal, 8)
            .padding(.vertical, 2)
            .background(.quaternary, in: .capsule)
            .fixedSize()
    }
}

extension ThinkingLevel {
    /// Off, On or Long.
    public var title: String {
        switch self {
        case .off: String(localized: "Off")
        case .on: String(localized: "On")
        case .long: String(localized: "Long")
        }
    }
}
