import ComposableArchitecture
import HumataDaemonClient
import SwiftUI

/// The search box, how far back and in which project to search, and what
/// was found, each hit with where it is and the words found in bold.
package struct SearchView: View {
    @Bindable var store: StoreOf<SearchFeature>
    @FocusState private var focused: Bool

    package init(store: StoreOf<SearchFeature>) {
        self.store = store
    }

    package var body: some View {
        VStack(spacing: 0) {
            bar
            Divider()
            results
        }
        .navigationTitle("Search Sessions")
        .onAppear { focused = true }
    }

    private var bar: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 8) {
                Image(systemName: "magnifyingglass")
                    .foregroundStyle(.secondary)
                TextField("Search what sessions said and ran, as \"cargo test last week\"", text: $store.text)
                    .textFieldStyle(.plain)
                    .font(.title3)
                    .focused($focused)
                    .onSubmit { store.send(.submitted) }
                if store.isSearching {
                    ProgressView().controlSize(.small)
                }
            }
            HStack(spacing: 12) {
                Picker("Time", selection: $store.period) {
                    Text("Any Time").tag(SearchFeature.Period.anyTime)
                    Text("Today").tag(SearchFeature.Period.today)
                    Text("Past Week").tag(SearchFeature.Period.pastWeek)
                    Text("Past Month").tag(SearchFeature.Period.pastMonth)
                }
                .fixedSize()
                .disabled(store.namedSince != nil)
                .help(
                    store.namedSince != nil
                        ? "The search names its own time" : "How far back to search")
                Picker("Project", selection: $store.projectID) {
                    Text("All Projects").tag(String?.none)
                    ForEach(store.projects) { project in
                        Text(project.name).tag(Optional(project.id))
                    }
                }
                .fixedSize()
                Spacer()
                if !store.related.isEmpty {
                    Text("Also: " + store.related.joined(separator: ", "))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .help("Related words, found as well")
                }
            }
            .controlSize(.small)
            if let since = store.namedSince {
                Text(named(since: since, until: store.namedUntil))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .padding(12)
    }

    @ViewBuilder
    private var results: some View {
        if let failure = store.failure {
            ContentUnavailableView(
                "Couldn't Search", systemImage: "exclamationmark.magnifyingglass", description: Text(failure))
        } else if store.text.trimmingCharacters(in: .whitespaces).isEmpty {
            ContentUnavailableView(
                "Search Every Session", systemImage: "text.magnifyingglass",
                description: Text(
                    "Find what was said, run or answered in any session, on this Mac and every host. "
                        + "Name a time, as \"yesterday\" or \"past 3 days\", to search only then."))
        } else if store.hits.isEmpty, store.searched {
            ContentUnavailableView.search(text: store.words.isEmpty ? store.text : store.words)
        } else {
            List(store.hits) { hit in
                Button {
                    store.send(.hitTapped(hit))
                } label: {
                    HitRow(hit: hit)
                }
                .buttonStyle(.plain)
            }
            .listStyle(.inset)
        }
    }

    private func named(since: Date, until: Date?) -> String {
        let format = Date.FormatStyle(date: .abbreviated, time: .shortened)
        guard let until else { return "Searching since \(since.formatted(format))" }
        return "Searching from \(since.formatted(format)) to \(until.formatted(format))"
    }
}

/// A hit: where it is, when, and the text around what was found.
private struct HitRow: View {
    let hit: SearchHit

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 6) {
                Image(systemName: symbol)
                    .foregroundStyle(.secondary)
                    .frame(width: 16)
                Text(place)
                    .font(.headline)
                    .lineLimit(1)
                if hit.related {
                    Text("Related")
                        .font(.caption2.weight(.medium))
                        .padding(.horizontal, 5)
                        .padding(.vertical, 1)
                        .background(.quaternary, in: Capsule())
                        .help("Found by a related word, not the search's own")
                }
                Spacer()
                Text(hit.time, format: .relative(presentation: .named))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            snippet
                .font(hit.kind == .toolCall || hit.kind == .toolOutput ? .callout.monospaced() : .callout)
                .foregroundStyle(.secondary)
                .lineLimit(3)
        }
        .padding(.vertical, 4)
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
    }

    private var snippet: Text {
        hit.snippet.reduce(Text(verbatim: "")) { text, run in
            let piece = Text(verbatim: run.text.replacingOccurrences(of: "\n", with: " "))
            return text + (run.found ? piece.bold().foregroundColor(.primary) : piece)
        }
    }

    private var place: String {
        switch hit.kind {
        case .spec: "Spec"
        case .comment: "\(hit.sessionName) · review comment"
        case .message: "\(hit.sessionName) · turn \(hit.turn) · your message"
        case .reply: "\(hit.sessionName) · turn \(hit.turn) · reply"
        case .toolCall: "\(hit.sessionName) · turn \(hit.turn) · ran"
        case .toolOutput: "\(hit.sessionName) · turn \(hit.turn) · output"
        }
    }

    private var symbol: String {
        switch hit.kind {
        case .message: "person"
        case .reply: "sparkles"
        case .toolCall: "terminal"
        case .toolOutput: "text.alignleft"
        case .comment: "text.bubble"
        case .spec: "doc.text"
        }
    }
}
