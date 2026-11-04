import ComposableArchitecture
import LifrCore
import SwiftUI

package struct ActivityView: View {
    @Bindable package var store: StoreOf<ActivityFeature>

    package init(store: StoreOf<ActivityFeature>) {
        self.store = store
    }

    package var body: some View {
        let rows = store.state.rows(now: Date(), calendar: .current)
        List {
            Section {
                Picker("Made by", selection: $store.author) {
                    Text("Anyone").tag(ActivityAuthor?.none)
                    ForEach(ActivityAuthor.all, id: \.self) { Text($0.title).tag(ActivityAuthor?.some($0)) }
                }
                Picker("Changed", selection: $store.kind) {
                    Text("Anything").tag(RecordKind?.none)
                    ForEach(RecordKind.allCases, id: \.self) { Text($0.title).tag(RecordKind?.some($0)) }
                }
                Picker("When", selection: $store.period) {
                    ForEach(ActivityPeriod.allCases, id: \.self) { Text($0.title).tag($0) }
                }
            }
            Section {
                if rows.isEmpty {
                    Text("No changes match.").foregroundStyle(.secondary)
                }
                ForEach(rows) { row in
                    ActivityRowView(row: row)
                        .contextMenu { actions(row) }
                        .swipeActions(allowsFullSwipe: false) { actions(row) }
                }
            }
        }
        .navigationTitle("Activity")
        .confirmationDialog($store.scope(state: \.confirm, action: \.confirm))
        .alert($store.scope(state: \.alert, action: \.alert))
    }

    @ViewBuilder
    private func actions(_ row: ActivityRow) -> some View {
        Button("Revert") { store.send(.revertTapped(row.id)) }
            .tint(.orange)
        Button("Revert everything since") { store.send(.revertSinceTapped(row.id)) }
    }
}

private struct ActivityRowView: View {
    let row: ActivityRow

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(alignment: .firstTextBaseline) {
                Text(row.summary).font(.headline)
                if row.isReverted {
                    Text("Reverted")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                Spacer()
                Text(row.at, format: .dateTime.month().day().hour().minute())
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Text(row.author).font(.subheadline).foregroundStyle(.secondary)
            ForEach(Array(row.changes.enumerated()), id: \.offset) { _, change in
                Text(Self.describe(change, showsRecord: row.changes.count > 1))
                    .font(.caption)
            }
        }
        .accessibilityElement(children: .combine)
    }

    /// "Priority: None → High".
    private static func describe(_ change: ActivityRow.Change, showsRecord: Bool) -> String {
        let name = showsRecord ? "\(change.record) · \(change.field)" : change.field
        return "\(name): \(change.before ?? "—") → \(change.after ?? "—")"
    }
}
