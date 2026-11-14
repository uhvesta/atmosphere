import Humata2Core
import Humata2Models
import SwiftUI

/// A session's chat: its rows, the reply streaming, what's queued, and the
/// message field.
struct ChatScreen: View {
    let chat: ChatModel
    @State private var draft = ""
    @State private var position = ScrollPosition(edge: .bottom)
    /// The newest is in view, so what comes next is scrolled to; scrolling
    /// up to read stops that until the bottom is reached again.
    @State private var following = true

    var body: some View {
        VStack(spacing: 0) {
            if let reading = chat.status.reading {
                ReadingView(reading: reading)
            } else {
                transcript
            }
            Divider()
            footer
        }
        .onChange(of: chat.status.unsent, initial: true) { _, unsent in
            guard let unsent else { return }
            draft = draft.isEmpty ? unsent : "\(unsent)\n\n\(draft)"
            chat.send(.tookUnsent)
        }
    }

    private var transcript: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 12) {
                ForEach(chat.rows, id: \.id) { row in
                    RowView(row: row)
                }
                if let reply = chat.reply {
                    Text(reply.text)
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                if chat.status.working {
                    HStack(spacing: 6) {
                        ProgressView().controlSize(.small)
                        Text(chat.status.waiting ?? "Working…")
                            .foregroundStyle(.secondary)
                    }
                }
            }
            .padding()
        }
        .scrollPosition($position)
        .defaultScrollAnchor(.bottom)
        .onScrollGeometryChange(for: ScrollGeometry.self, of: { $0 }, action: scrolled)
        .overlay {
            if chat.rows.isEmpty, chat.reply == nil, !chat.status.working {
                ContentUnavailableView("No Messages Yet", systemImage: "bubble.left")
            }
        }
    }

    /// Grown content is followed while the newest was in view; otherwise
    /// the person scrolled, and following is whether they reached the end.
    private func scrolled(from old: ScrollGeometry, to new: ScrollGeometry) {
        if new.contentSize.height != old.contentSize.height {
            if following { position.scrollTo(edge: .bottom) }
        } else {
            following = new.visibleRect.maxY >= new.contentSize.height - 24
        }
    }

    private var footer: some View {
        VStack(alignment: .leading, spacing: 6) {
            ForEach(chat.status.queued, id: \.id) { queued in
                Label(queued.text, systemImage: "clock")
                    .lineLimit(1)
                    .foregroundStyle(.secondary)
            }
            if let error = chat.status.error {
                Label(error, systemImage: "exclamationmark.triangle")
                    .foregroundStyle(.secondary)
                    .font(.callout)
            }
            HStack(alignment: .bottom, spacing: 8) {
                TextField("Message", text: $draft, axis: .vertical)
                    .lineLimit(1...8)
                    .textFieldStyle(.roundedBorder)
                    .onSubmit(send)
                if chat.status.working {
                    Button(chat.status.stopping ? "Stopping…" : "Stop", systemImage: "stop.fill") {
                        chat.send(.stop)
                    }
                    .disabled(chat.status.stopping)
                    .keyboardShortcut(".", modifiers: .command)
                }
                Button("Send", systemImage: "arrow.up", action: send)
                    .disabled(draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    .keyboardShortcut(.return, modifiers: .command)
            }
        }
        .padding()
    }

    private func send() {
        let text = draft
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        draft = ""
        chat.send(.send(text: text))
    }
}

/// The history being read, with how much of it is.
private struct ReadingView: View {
    let reading: ReadingProgress

    var body: some View {
        VStack {
            if reading.total > 0 {
                ProgressView(
                    "Reading the chat…", value: Double(min(reading.read, reading.total)),
                    total: Double(reading.total))
            } else {
                ProgressView("Reading the chat…")
            }
        }
        .frame(maxWidth: 320)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

private struct RowView: View {
    let row: Row

    var body: some View {
        switch row.body {
        case .asked(let text, let images):
            VStack(alignment: .trailing, spacing: 4) {
                Text(text)
                    .textSelection(.enabled)
                    .padding(10)
                    .background(.tint.opacity(0.15), in: .rect(cornerRadius: 10))
                if images > 0 {
                    Label("\(images) image\(images == 1 ? "" : "s")", systemImage: "photo")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            .frame(maxWidth: .infinity, alignment: .trailing)
        case .replied(let text):
            Text(text)
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
        case .thought(let text):
            DisclosureGroup("Thought") {
                Text(text).foregroundStyle(.secondary).textSelection(.enabled)
            }
        case .tool(_, let tool, let arguments, let outcome):
            ToolView(tool: tool, arguments: arguments, outcome: outcome)
        case .permission(_, let summary, let answer):
            Label {
                Text(summary)
            } icon: {
                Image(systemName: answer.map { $0.approved ? "checkmark.shield" : "xmark.shield" } ?? "shield")
            }
            .foregroundStyle(answer == nil ? .primary : .secondary)
        case .notice(let text):
            Label(text, systemImage: "info.circle").foregroundStyle(.secondary)
        case .failed(let message):
            Label(message, systemImage: "exclamationmark.octagon").foregroundStyle(.red)
        case .stopped:
            Label("Stopped", systemImage: "stop.circle").foregroundStyle(.secondary)
        }
    }
}

private struct ToolView: View {
    let tool: String
    let arguments: String
    let outcome: ToolOutcome?

    var body: some View {
        DisclosureGroup {
            VStack(alignment: .leading, spacing: 6) {
                Text(arguments).font(.caption.monospaced()).textSelection(.enabled)
                if let outcome {
                    Text(outcome.output)
                        .font(.caption.monospaced())
                        .lineLimit(40)
                        .textSelection(.enabled)
                }
            }
        } label: {
            HStack(spacing: 6) {
                if let outcome {
                    Image(systemName: outcome.success ? "checkmark.circle" : "xmark.circle")
                        .foregroundStyle(outcome.success ? .green : .red)
                } else {
                    ProgressView().controlSize(.small)
                }
                Text(tool).font(.callout.monospaced())
            }
        }
    }
}
