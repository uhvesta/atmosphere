import AppKit
import ComposableArchitecture
import HumataDaemonClient
import SwiftTerm
import SwiftUI

/// The terminals as tabs above the selected one, with + for another and ×
/// to close one; the shell keeps running on its host when the view goes.
package struct TerminalsView: View {
    let store: StoreOf<TerminalsFeature>

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
            if let failure = store.failure {
                Label(failure, systemImage: "exclamationmark.triangle")
                    .font(.callout)
                    .padding(8)
                    .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 8))
                    .padding()
            }
        }
        .task { await store.send(.task).finish() }
    }

    private var tabBar: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 4) {
                ForEach(store.tabs) { tab in
                    TerminalTab(
                        title: tab.terminal.title, exited: tab.exited, selected: tab.id == store.selected,
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
    let select: () -> Void
    let close: () -> Void

    var body: some View {
        HStack(spacing: 4) {
            Button(action: select) {
                Label(title, systemImage: exited ? "terminal.fill" : "terminal")
                    .foregroundStyle(exited ? .secondary : .primary)
            }
            .buttonStyle(.plain)
            Button(action: close) {
                Image(systemName: "xmark")
                    .imageScale(.small)
            }
            .buttonStyle(.borderless)
            .help(exited ? "Forget this terminal" : "End this terminal's shell")
            .accessibilityLabel("Close \(title)")
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 3)
        .background(selected ? Color.accentColor.opacity(0.18) : .clear, in: RoundedRectangle(cornerRadius: 6))
    }
}

/// One terminal, drawn as it printed, with a note once it exited or while
/// it's reconnecting.
private struct TerminalPane: View {
    let store: StoreOf<TerminalFeature>

    var body: some View {
        VStack(spacing: 0) {
            TerminalScreenView(
                screen: store.screen, typed: { store.send(.typed($0)) }, resized: { store.send(.resized($0)) })
            if let code = store.terminal.exitCode {
                notice(
                    code == 0 ? "The shell exited." : "The shell exited with code \(code).",
                    systemImage: "stop.circle")
            } else if store.detached {
                notice("Lost the connection to the terminal. Reconnecting…", systemImage: "bolt.horizontal.circle")
            } else if let failure = store.failure {
                notice(failure, systemImage: "exclamationmark.triangle")
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

/// SwiftTerm's view, fed the screen's bytes as they come: from the start
/// when they were replayed, otherwise only what's new.
private struct TerminalScreenView: NSViewRepresentable {
    let screen: TerminalFeature.Screen
    let typed: (Data) -> Void
    let resized: (TerminalSize) -> Void

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> TerminalView {
        let view = TerminalView(frame: NSRect(x: 0, y: 0, width: 640, height: 400))
        view.font = .monospacedSystemFont(ofSize: 12, weight: .regular)
        view.terminalDelegate = context.coordinator
        return view
    }

    func updateNSView(_ view: TerminalView, context: Context) {
        let coordinator = context.coordinator
        coordinator.typed = typed
        coordinator.resized = resized
        if coordinator.generation != screen.generation || coordinator.fed < screen.start {
            // RIS: clears the screen and scrollback to draw it again.
            view.feed(text: "\u{1b}c")
            coordinator.generation = screen.generation
            coordinator.fed = screen.start
        }
        let new = coordinator.fed - screen.start
        guard new < screen.data.count else { return }
        view.feed(byteArray: ArraySlice(screen.data.dropFirst(new)))
        coordinator.fed = screen.end
    }

    final class Coordinator: NSObject, TerminalViewDelegate {
        var generation = 0
        /// How many of the screen's bytes the view was given.
        var fed = 0
        var typed: (Data) -> Void = { _ in }
        var resized: (TerminalSize) -> Void = { _ in }

        func send(source: TerminalView, data: ArraySlice<UInt8>) { typed(Data(data)) }

        func sizeChanged(source: TerminalView, newCols: Int, newRows: Int) {
            resized(TerminalSize(rows: newRows, columns: newCols))
        }

        func setTerminalTitle(source: TerminalView, title: String) {}
        func hostCurrentDirectoryUpdate(source: TerminalView, directory: String?) {}
        func scrolled(source: TerminalView, position: Double) {}
        func rangeChanged(source: TerminalView, startY: Int, endY: Int) {}
    }
}
