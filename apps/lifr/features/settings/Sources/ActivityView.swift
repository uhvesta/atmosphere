import ChatUI
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
                    Text(store.author == nil && store.kind == nil && store.period == .all ? "No changes yet." : "No changes match.")
                        .foregroundStyle(.secondary)
                }
                ForEach(rows) { row in
                    if row.canRevert {
                        ActivityRowView(row: row)
                            .contextMenu { actions(row) }
                            .swipeActions(allowsFullSwipe: false) { actions(row) }
                    } else {
                        ActivityRowView(row: row)
                    }
                }
            } footer: {
                if store.archived == nil {
                    Button("Show changes older than 90 days") { store.send(.olderTapped) }
                        .accessibilityHint("Older changes can be looked at but not reverted")
                } else if store.archived?.isEmpty == true {
                    Text("No changes older than 90 days.")
                } else {
                    Text("Changes older than 90 days can be looked at but not reverted.")
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
        ActivityEntryRow(
            summary: row.summary, author: row.author, at: row.at, changes: row.changes, isReverted: row.isReverted)
    }
}
