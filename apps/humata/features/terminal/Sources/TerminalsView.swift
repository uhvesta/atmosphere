import AppKit
import ComposableArchitecture
import HumataDaemonClient
import SwiftUI

/// The terminals as tabs above the selected one, with + for another and ×
/// to close one; the shell keeps running on its host when the view goes.
package struct TerminalsView: View {
    @Bindable var store: StoreOf<TerminalsFeature>

    package init(store: StoreOf<TerminalsFeature>) {
        self.store = store
    }

    package var body: some View {
        VStack(spacing: 0) {
            tabBar
            Divider()
            if let id = store.selected, let terminal = store.scope(state: \.tabs[id: id], action: \.terminals[id: id]) {
                TerminalPane(store: terminal)
                    .id(id)
            } else if let failure = store.listFailure {
                ContentUnavailableView {
                    Label("Can't Read the Terminals", systemImage: "terminal")
                } description: {
                    Text(failure)
                } actions: {
                    Button("Try Again") { store.send(.task) }
                }
            } else if store.listed, !store.opening {
                ContentUnavailableView {
                    Label("No Terminals", systemImage: "terminal")
                } description: {
                    Text(
                        store.place.host.map { "Open a shell on \($0); it keeps running when you close Humata." }
                            ?? "Open a shell on this Mac; it keeps running when you close Humata.")
                } actions: {
                    Button("New Terminal") { store.send(.newTapped) }
                }
            } else {
                ProgressView()
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .overlay(alignment: .bottom) {
            if let failure = store.failure, store.listFailure == nil {
                Label(failure, systemImage: "exclamationmark.triangle")
                    .font(.callout)
                    .padding(8)
                    .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 8))
                    .padding()
            }
        }
        .alert($store.scope(state: \.confirm, action: \.confirm))
        // Not awaited: leaving the tab stops neither the read nor the first
        // open, which lands in the tabs whenever it comes.
        .task { store.send(.task) }
    }

    private var tabBar: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 4) {
                ForEach(store.tabs) { tab in
                    TerminalTab(
                        title: tab.terminal.title, exited: tab.exited, selected: tab.id == store.selected,
                        closing: store.closing.contains(tab.id),
                        select: { store.send(.selected(tab.id)) }, close: { store.send(.closeTapped(tab.id)) })
                }
                Button {
                    store.send(.newTapped)
                } label: {
                    Image(systemName: "plus")
                }
                .buttonStyle(.borderless)
                .disabled(store.opening)
                .help("Open another terminal here")
                .accessibilityLabel("New Terminal")
                // With a terminal showing, nothing else says another is on its way.
                if store.opening, store.selected != nil {
                    ProgressView()
                        .controlSize(.small)
                        .accessibilityLabel("Opening a terminal")
                }
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
        }
    }
}

private struct TerminalTab: View {
    let title: String
    let exited: Bool
    let selected: Bool
    /// Its host is ending its shell and hasn't answered yet.
    let closing: Bool
    let select: () -> Void
    let close: () -> Void

    var body: some View {
        HStack(spacing: 4) {
            Button(action: select) {
                Label(title, systemImage: exited ? "terminal.fill" : "terminal")
                    .foregroundStyle(exited ? .secondary : .primary)
            }
            .buttonStyle(.plain)
            if closing {
                ProgressView()
                    .controlSize(.mini)
                    .help("Ending this terminal's shell…")
                    .accessibilityLabel("Closing \(title)")
            } else {
                Button(action: close) {
                    Image(systemName: "xmark")
                        .imageScale(.small)
                }
                .buttonStyle(.borderless)
                .help(exited ? "Forget this terminal" : "End this terminal's shell…")
                .accessibilityLabel("Close \(title)")
            }
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 3)
        .background(selected ? Color.accentColor.opacity(0.18) : .clear, in: RoundedRectangle(cornerRadius: 6))
    }
}

/// One terminal, drawn by Ghostty, with a note once its shell exited or it
/// was closed elsewhere, and Reattach when only the pipe to it ended.
private struct TerminalPane: View {
    let store: StoreOf<TerminalFeature>
    /// The find bar's, which only this view and its surface share.
    @State private var search = TerminalSearch()

    var body: some View {
        VStack(spacing: 0) {
            if let command = store.command {
                TerminalSurface(command: command, generation: store.generation, search: search) { _ in
                    store.send(.surfaceExited)
                }
                .overlay(alignment: .topTrailing) {
                    if search.shown { TerminalFindBar(search: search) }
                }
            } else {
                Color.clear
            }
            if let code = store.terminal.exitCode {
                notice(
                    code == 0 ? "The shell exited." : "The shell exited with code \(code).",
                    systemImage: "stop.circle")
            } else if store.gone {
                notice("This terminal was closed elsewhere.", systemImage: "xmark.circle")
            } else if store.detached {
                HStack(spacing: 0) {
                    notice(
                        store.failure ?? "The connection to the terminal ended. Its shell is still running.",
                        systemImage: "bolt.horizontal.circle")
                    Button("Reattach") { store.send(.reattachTapped) }
                        .padding(.trailing, 8)
                }
            }
        }
        .task { await store.send(.attach).finish() }
    }

    private func notice(_ text: String, systemImage: String) -> some View {
        Label(text, systemImage: systemImage)
            .font(.callout)
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(8)
    }
}
