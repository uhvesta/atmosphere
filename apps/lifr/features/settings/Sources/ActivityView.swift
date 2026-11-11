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
                    let filtered = store.author != nil || store.kind != nil || store.period != .all
                    Text(filtered ? "No changes match." : "No changes yet.")
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
            } header: {
                if rows.contains(where: \.canRevert) {
                    Text(revertHint)
                        .font(.footnote)
                        .fontWeight(.regular)
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

    /// How to revert a change, in the gesture the platform uses for it.
    private var revertHint: LocalizedStringKey {
        #if os(macOS)
            "Control-click a change to revert it."
        #else
            "Swipe left on a change, or touch and hold it, to revert it."
        #endif
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
