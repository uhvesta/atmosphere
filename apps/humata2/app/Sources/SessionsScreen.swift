import Humata2Core
import Humata2Models
import SwiftUI

/// The sessions, newest first, and the one picked.
struct SessionsScreen: View {
    let model: SessionsModel

    var body: some View {
        if let startError = model.startError {
            ContentUnavailableView(
                "Humata 2 Couldn't Start", systemImage: "exclamationmark.triangle",
                description: Text(startError))
        } else {
            NavigationSplitView {
                sidebar
                    .navigationSplitViewColumnWidth(min: 240, ideal: 300)
            } detail: {
                detail
            }
            .onAppear { model.send(.appeared) }
            .onDisappear { model.send(.disappeared) }
        }
    }

    private var selection: Binding<String?> {
        Binding(
            get: { model.list.selected },
            set: { model.send(.select(id: $0)) })
    }

    private var sidebar: some View {
        List(selection: selection) {
            if let error = model.list.error {
                Label(error, systemImage: "exclamationmark.triangle")
                    .foregroundStyle(.secondary)
                    .font(.callout)
            }
            ForEach(model.list.rows, id: \.id) { session in
                SessionRow(session: session).tag(session.id)
            }
        }
        .overlay {
            if model.list.loading {
                ProgressView("Loading sessions…")
            } else if model.list.rows.isEmpty, model.list.error == nil {
                ContentUnavailableView("No Sessions", systemImage: "bubble.left.and.bubble.right")
            }
        }
        .toolbar {
            Button("Refresh", systemImage: "arrow.clockwise") { model.send(.refresh) }
                .keyboardShortcut("r")
        }
    }

    @ViewBuilder private var detail: some View {
        if let id = model.list.selected, let session = model.list.rows.first(where: { $0.id == id }) {
            ChatScreen(chat: model.chat)
                .navigationTitle(session.name)
                .navigationSubtitle(Text(session.state.title))
        } else {
            ContentUnavailableView("Pick a Session", systemImage: "sidebar.left")
        }
    }
}

private struct SessionRow: View {
    let session: Session

    var body: some View {
        HStack(spacing: 8) {
            Circle()
                .fill(session.unread ? Color.accentColor : .clear)
                .frame(width: 7, height: 7)
            VStack(alignment: .leading, spacing: 2) {
                Text(session.name).lineLimit(1)
                HStack(spacing: 4) {
                    Text(session.state.title)
                    if !session.host.isEmpty {
                        Text("· \(session.host)")
                    }
                    if session.lastActiveUnixMs > 0 {
                        Text("· ")
                            + Text(
                                Date(timeIntervalSince1970: Double(session.lastActiveUnixMs) / 1000),
                                style: .relative)
                    }
                }
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(1)
            }
            Spacer(minLength: 0)
            if session.state == .working {
                ProgressView().controlSize(.small)
            }
        }
    }
}

extension SessionState {
    fileprivate var title: LocalizedStringKey {
        switch self {
        case .idle: "Idle"
        case .working: "Working"
        case .waitingForApproval: "Waiting for approval"
        case .failed: "Failed"
        }
    }
}
