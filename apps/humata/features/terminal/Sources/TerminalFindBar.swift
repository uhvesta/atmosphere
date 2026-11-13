// The search state and its debounce are adapted from Ghostty's macOS app,
// MIT, Copyright (c) 2024 Mitchell Hashimoto, Ghostty contributors.

import AppKit
import SwiftUI

/// Finding text in one terminal, with Ghostty's own search: ⌘F in the
/// terminal starts it, Ghostty marks the matches in what's drawn and in
/// the scrollback, and says how many there are and which is selected.
@MainActor
@Observable
final class TerminalSearch {
    /// Whether the find bar is up.
    private(set) var shown = false
    var needle = ""
    /// How many matches there are, once Ghostty has counted.
    private(set) var total: Int?
    /// Which match is selected, from 0; nil when none is.
    private(set) var selected: Int?
    /// Goes up each time ⌘F asks for the field, so it takes the keys again.
    private(set) var focusRequests = 0

    /// The surface searched; nil once it's gone.
    @ObservationIgnored weak var view: GhosttySurfaceView?
    @ObservationIgnored private var pending: Task<Void, Never>?

    /// Ghostty started a search, with `needle` when it has one already.
    func started(needle: String?) {
        if let needle, !needle.isEmpty { self.needle = needle }
        let wasShown = shown
        shown = true
        focusRequests += 1
        // A needle kept from last time is searched for again.
        if !wasShown, !self.needle.isEmpty { search(self.needle) }
    }

    /// Ghostty ended the search, or its surface went.
    func ended() {
        pending?.cancel()
        pending = nil
        shown = false
        total = nil
        selected = nil
    }

    /// Ghostty counted the matches; nil while it can't say.
    func counted(_ total: Int?) { self.total = total }
    /// Ghostty selected a match; nil when none is.
    func moved(to index: Int?) { selected = index }

    /// The needle was typed over: searches for it, after a moment when it's
    /// only a letter or two, which match almost everything.
    func needleChanged() {
        pending?.cancel()
        pending = nil
        let needle = needle
        guard !needle.isEmpty, needle.count < 3 else {
            search(needle)
            return
        }
        pending = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(300))
            guard !Task.isCancelled else { return }
            self?.search(needle)
        }
    }

    func next() { view?.perform("navigate_search:next") }
    func previous() { view?.perform("navigate_search:previous") }

    /// Takes the find bar down and gives the terminal the keys back.
    func close() {
        view?.perform("end_search")
        ended()
        if let view { view.window?.makeFirstResponder(view) }
    }

    private func search(_ needle: String) {
        if needle.isEmpty {
            total = nil
            selected = nil
        }
        // An empty needle cancels the search and keeps the bar.
        view?.perform("search:\(needle)")
    }
}

/// The find bar over a terminal: what to find, how many matches there are,
/// the one before and after, and Done.
struct TerminalFindBar: View {
    @Bindable var search: TerminalSearch
    @FocusState private var focused: Bool

    var body: some View {
        HStack(spacing: 6) {
            TextField("Find", text: $search.needle)
                .textFieldStyle(.roundedBorder)
                .frame(width: 180)
                .focused($focused)
                .onSubmit { search.next() }
                .accessibilityLabel("Find in Terminal")
            if let count {
                Text(count)
                    .font(.caption)
                    .monospacedDigit()
                    .foregroundStyle(.secondary)
            }
            Button {
                search.previous()
            } label: {
                Image(systemName: "chevron.up")
            }
            .help("The match before")
            .accessibilityLabel("Previous Match")
            Button {
                search.next()
            } label: {
                Image(systemName: "chevron.down")
            }
            .help("The match after")
            .accessibilityLabel("Next Match")
            Button("Done") { search.close() }
        }
        .buttonStyle(.borderless)
        .padding(6)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 8))
        .padding(8)
        .onChange(of: search.needle) { search.needleChanged() }
        .onChange(of: search.focusRequests, initial: true) { focused = true }
        .onExitCommand { search.close() }
    }

    /// "2 of 7", or how many when none is selected; nothing before Ghostty
    /// has counted.
    private var count: String? {
        guard !search.needle.isEmpty, let total = search.total else { return nil }
        if total == 0 { return "No matches" }
        if let selected = search.selected { return "\(selected + 1) of \(total)" }
        return "\(total) found"
    }
}
