import ChatKit
import SwiftUI

/// Every point a conversation can go to, on every branch, oldest first:
/// tap one to go there, or swipe to fork from it.
public struct CheckpointList<Message: ChatEntry, Context: ConversationContext>: View {
    public typealias Checkpoint = Conversation<Message, Context>.Checkpoint

    let checkpoints: [Checkpoint]
    let canGo: Bool
    let canFork: (UUID) -> Bool
    let go: (UUID) -> Void
    let fork: (UUID) -> Void

    /// `go` and `fork` get the checkpoint's end.
    public init(
        _ checkpoints: [Checkpoint], canGo: Bool, canFork: @escaping (UUID) -> Bool, go: @escaping (UUID) -> Void,
        fork: @escaping (UUID) -> Void
    ) {
        self.checkpoints = checkpoints
        self.canGo = canGo
        self.canFork = canFork
        self.go = go
        self.fork = fork
    }

    public var body: some View {
        List(checkpoints) { checkpoint in
            Button {
                go(checkpoint.end)
            } label: {
                HStack(alignment: .firstTextBaseline) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(title(of: checkpoint))
                            .lineLimit(2)
                            .foregroundStyle(checkpoint.isShown ? .primary : .secondary)
                        HStack(spacing: 6) {
                            Text(checkpoint.message.at, format: .dateTime.hour().minute())
                            if checkpoint.changes > 0 {
                                Text("^[\(checkpoint.changes) change](inflect: true)")
                            }
                            if !checkpoint.isShown {
                                Text("Not shown")
                            }
                        }
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    }
                    Spacer()
                    if checkpoint.isCurrent {
                        Image(systemName: "checkmark")
                            .foregroundStyle(.tint)
                            .accessibilityLabel(Text("Where the conversation is"))
                    }
                }
                .contentShape(.rect)
            }
            .buttonStyle(.plain)
            .disabled(!canGo)
            .accessibilityAddTraits(checkpoint.isCurrent ? .isSelected : [])
            .swipeActions {
                Button("Fork", systemImage: "arrow.triangle.branch") { fork(checkpoint.end) }
                    .tint(.indigo)
                    .disabled(!canFork(checkpoint.end))
            }
            .contextMenu {
                Button("Fork From Here", systemImage: "arrow.triangle.branch") { fork(checkpoint.end) }
                    .disabled(!canFork(checkpoint.end))
            }
        }
    }

    private func title(of checkpoint: Checkpoint) -> String {
        checkpoint.message.text.isEmpty ? String(localized: "Attachment") : checkpoint.message.text
    }
}
