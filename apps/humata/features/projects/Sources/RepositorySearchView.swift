import ComposableArchitecture
import HumataDaemonClient
import SwiftUI

/// A repository row's field: typing searches the saved repositories, or
/// completes a folder from `/` or `~/`; a URL is taken as typed. Arrows
/// move through the list under it, Return or Tab picks, Escape closes it.
struct RepositoryField: View {
    @Binding var path: String
    let row: RepositoryRow.ID
    let search: StoreOf<RepositorySearchFeature>
    @FocusState private var focused: Bool

    var body: some View {
        TextField(
            "Repository", text: $path,
            prompt: Text("Search saved, /path/to/repository or https://github.com/owner/name")
        )
        .labelsHidden()
        .focused($focused)
        .onChange(of: path) { _, text in
            if focused { search.send(.typed(row: row, text)) }
        }
        .onChange(of: focused) { _, now in
            search.send(now ? .typed(row: row, path) : .left(row: row))
        }
        // Return and Tab pick only while a suggestion shows, so otherwise
        // Return still adds or saves the project and Tab moves on.
        .onKeyPress(keys: [.upArrow, .downArrow, .return, .tab, .escape]) { press in
            guard search.row == row else { return .ignored }
            switch press.key {
            case .escape:
                search.send(.closed)
            case .upArrow, .downArrow:
                guard !search.suggestions.isEmpty else { return .ignored }
                search.send(.moved(by: press.key == .upArrow ? -1 : 1))
            default:
                guard search.highlighted != nil else { return .ignored }
                search.send(.submitted)
            }
            return .handled
        }
    }
}

/// What the row being typed in is offered, under it.
struct RepositorySuggestionList: View {
    let row: RepositoryRow.ID
    let search: StoreOf<RepositorySearchFeature>

    var body: some View {
        if search.row == row {
            VStack(alignment: .leading, spacing: 2) {
                if search.loading, search.saved == nil {
                    HStack(spacing: 8) {
                        ProgressView().controlSize(.small)
                        Text("Reading the saved repositories…")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
                if let failure = search.failure {
                    Text(failure)
                        .font(.caption)
                        .foregroundStyle(.red)
                }
                ForEach(Array(search.suggestions.enumerated()), id: \.element.id) { index, suggestion in
                    Button {
                        search.send(.picked(suggestion))
                    } label: {
                        SuggestionRow(suggestion: suggestion)
                            .padding(.horizontal, 6)
                            .padding(.vertical, 3)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .background(
                                index == search.selected ? Color.accentColor.opacity(0.18) : Color.clear,
                                in: RoundedRectangle(cornerRadius: 5)
                            )
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                }
            }
            .accessibilityIdentifier("repository.suggestions")
        }
    }
}

private struct SuggestionRow: View {
    let suggestion: RepositorySuggestion

    var body: some View {
        switch suggestion {
        case .saved(let repository):
            VStack(alignment: .leading, spacing: 1) {
                HStack(spacing: 6) {
                    Text(repository.name)
                        .fontWeight(.medium)
                    Text(Self.source(repository))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                Text(repository.location)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
        case .folder(let path):
            Label(path, systemImage: "folder")
                .lineLimit(1)
                .truncationMode(.head)
        }
    }

    static func source(_ repository: SavedRepository) -> String {
        switch repository.source {
        case .github: String(localized: "GitHub · \(repository.owner)")
        case .used: String(localized: "Used before")
        }
    }
}
