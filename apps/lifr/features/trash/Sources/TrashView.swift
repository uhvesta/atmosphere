import ComposableArchitecture
import LifrUI
import SwiftUI

/// The trash: each task with the day it's deleted for good, swipe or
/// right-click to restore, and Empty Trash.
package struct TrashView: View {
    @Bindable package var store: StoreOf<TrashFeature>

    package init(store: StoreOf<TrashFeature>) {
        self.store = store
    }

    package var body: some View {
        Group {
            if store.rows.isEmpty {
                ContentUnavailableView {
                    Label("Trash Is Empty", systemImage: "trash")
                } description: {
                    Text("Tasks you move to the trash stay here for 30 days, then they're deleted.")
                }
            } else {
                List {
                    Section {
                        ForEach(store.rows) { row in
                            TrashRowView(row: row)
                                .swipeActions(edge: .leading, allowsFullSwipe: true) {
                                    Button("Restore") { store.send(.restoreTapped(row.id)) }
                                        .tint(.blue)
                                }
                                .contextMenu {
                                    Button("Restore") { store.send(.restoreTapped(row.id)) }
                                }
                                .accessibilityAction(named: "Restore") { store.send(.restoreTapped(row.id)) }
                        }
                    } footer: {
                        Text("Tasks are deleted 30 days after they're moved to the trash.")
                    }
                }
            }
        }
        .navigationTitle("Trash")
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button("Empty Trash", role: .destructive) { store.send(.emptyTapped) }
                    .disabled(store.rows.isEmpty)
            }
        }
        .confirmationDialog($store.scope(state: \.confirmation, action: \.confirmation))
        .alert($store.scope(state: \.alert, action: \.alert))
    }
}

private struct TrashRowView: View {
    let row: TrashRow

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(row.title)
            HStack(spacing: 8) {
                if let color = row.avenueColor {
                    Circle().fill(Color(avenue: color)).frame(width: 8, height: 8).accessibilityHidden(true)
                }
                Text(row.code).monospaced()
                if row.subtaskCount > 0 {
                    Text("^[\(row.subtaskCount) Sub task](inflect: true)")
                }
                Text(deleted)
            }
            .font(.caption)
            .foregroundStyle(.secondary)
        }
        .accessibilityElement(children: .combine)
    }

    private var deleted: String {
        switch row.daysLeft {
        case 0: String(localized: "Deleted today")
        case 1: String(localized: "Deleted tomorrow")
        default: String(localized: "Deleted in \(row.daysLeft) days")
        }
    }
}
