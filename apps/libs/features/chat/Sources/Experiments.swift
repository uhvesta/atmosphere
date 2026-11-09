import ChatKit
import ChatUI
import ComposableArchitecture
import InferenceKit
import SwiftUI

/// Under a reply: the model that wrote it and how fast, when the person
/// asked to see it, and a button to see what the model was sent.
struct ReplyDetails: View {
    let model: String?
    let speed: ThreadSpeed?
    var prompt: (() -> Void)?

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            if let line = Self.line(model: model, speed: speed) {
                Text(line)
                    .help(Self.help(speed))
            }
            if let prompt {
                Button("Prompt", systemImage: "text.magnifyingglass", action: prompt)
                    .labelStyle(.titleAndIcon)
                    .buttonStyle(.borderless)
                    .help("See exactly what the model was sent for this reply")
            }
        }
        .font(.caption)
        .foregroundStyle(.secondary)
        .monospacedDigit()
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// "Gemma 4 E4B · 23.1 tok/s · 0.4 s to first token · 412 read", with
    /// ≈ before what's guessed.
    static func line(model: String?, speed: ThreadSpeed?) -> String? {
        guard let speed else { return model }
        let about = speed.estimated ? "≈" : ""
        var parts = model.map { [$0] } ?? []
        if let rate = speed.writtenPerSecond {
            let rate = rate.formatted(.number.precision(.fractionLength(1)))
            parts.append(String(localized: "\(about)\(rate) tok/s"))
        }
        let first = speed.reading.formatted(.units(allowed: [.seconds], fractionalPart: .show(length: 1)))
        parts.append(String(localized: "\(first) to first token"))
        parts.append(String(localized: "\(about)\(speed.read.formatted()) read"))
        parts.append(String(localized: "\(about)\(speed.written.formatted()) written"))
        return parts.joined(separator: " · ")
    }

    private static func help(_ speed: ThreadSpeed?) -> String {
        guard let speed else { return "" }
        let reading = speed.readPerSecond.map { $0.formatted(.number.precision(.fractionLength(0))) } ?? "–"
        let note =
            speed.estimated
            ? String(
                localized: "This model doesn't count its tokens, so the counts are guessed from the text's length.")
            : String(localized: "Counted by the model as it ran.")
        return String(localized: "Read the chat at \(reading) tokens a second. \(note)")
    }
}

/// Exactly what a model was sent for a reply: what it was told, the chat as
/// cut to fit its window, and how much of the window it took.
struct PromptSheet: View {
    let prompt: ReplyPrompt
    let close: () -> Void

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    if let model = prompt.model {
                        LabeledContent("Model", value: model)
                    }
                    LabeledContent("Window", value: ContextRing.description(of: prompt.context))
                    if prompt.photos > 0 {
                        LabeledContent("Photos", value: prompt.photos.formatted())
                    }
                }
                Section("Instructions") {
                    sent(prompt.instructions)
                }
                Section {
                    sent(prompt.prompt)
                } header: {
                    Text("Chat")
                } footer: {
                    Text("The newest messages that fit the model's window, with the files sent with the last.")
                }
            }
            .formStyle(.grouped)
            .navigationTitle("Prompt")
            #if os(iOS)
                .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done", action: close)
                }
            }
        }
        #if os(macOS)
            .frame(minWidth: 520, minHeight: 480)
        #endif
    }

    private func sent(_ text: String) -> some View {
        Text(text.isEmpty ? String(localized: "Nothing") : text)
            .font(.callout.monospaced())
            .foregroundStyle(text.isEmpty ? .secondary : .primary)
            .textSelection(.enabled)
            .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// Two models' answers to the same message, side by side on the Mac and
/// one above the other on the iPhone, each with a button to keep it.
struct Comparison: View {
    let first: ThreadMessage
    let rival: RivalReply
    let showsSpeed: Bool
    let select: (String) -> Void
    let keep: (_ rival: Bool) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Keep the answer to go on from.")
                .font(.footnote)
                .foregroundStyle(.secondary)
            layout {
                answer(
                    model: first.model, text: first.text, thinking: first.thinking, speed: first.speed, done: true,
                    keepsRival: false)
                answer(
                    model: rival.model, text: rival.text, thinking: rival.thinking, speed: rival.speed,
                    done: rival.done, keepsRival: true)
            }
        }
    }

    private var layout: AnyLayout {
        #if os(macOS)
            AnyLayout(HStackLayout(alignment: .top, spacing: 12))
        #else
            AnyLayout(VStackLayout(alignment: .leading, spacing: 12))
        #endif
    }

    private func answer(
        model: String?, text: String, thinking: ThreadThinking?, speed: ThreadSpeed?, done: Bool, keepsRival: Bool
    ) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(model ?? String(localized: "Model"))
                .font(.caption.weight(.semibold))
                .foregroundStyle(.secondary)
            if text.isEmpty, !done {
                ProgressView().controlSize(.small)
            } else {
                MessageView(role: .assistant, text: text, thinking: thinking?.model, select: select)
            }
            if showsSpeed, let line = ReplyDetails.line(model: nil, speed: speed) {
                Text(line)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .monospacedDigit()
            }
            Button("Keep This Answer", systemImage: "checkmark") { keep(keepsRival) }
                .buttonStyle(.bordered)
                .controlSize(.small)
                .disabled(!rival.done)
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .topLeading)
        .background(.quaternary.opacity(0.5), in: .rect(cornerRadius: 14))
    }
}

/// Above the field when comparing: the second model each reply is
/// compared with, to change or stop.
struct RivalBar: View {
    let model: String?
    let pick: () -> Void
    let remove: () -> Void

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "square.split.2x1")
                .foregroundStyle(.secondary)
            if let model {
                Button(action: pick) {
                    Text("Compared with \(model)")
                }
                .buttonStyle(.borderless)
                Spacer()
                Button("Stop Comparing", systemImage: "xmark.circle.fill", action: remove)
                    .labelStyle(.iconOnly)
                    .buttonStyle(.borderless)
                    .foregroundStyle(.secondary)
            } else {
                Button("Compare with Another Model…", action: pick)
                    .buttonStyle(.borderless)
                Spacer()
            }
        }
        .font(.footnote)
        .padding(.horizontal, 4)
    }
}
