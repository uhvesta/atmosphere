import ComposableArchitecture
import LifrCore
import LifrSyncClient
import LifrUI
import SwiftUI

/// The review sheet: each record that couldn't merge, with a choice per
/// field, or Keep and Delete for a record one device deleted.
package struct SyncReviewView: View {
    let store: StoreOf<SyncReviewFeature>

    package init(store: StoreOf<SyncReviewFeature>) {
        self.store = store
    }

    package var body: some View {
        Form {
            Section {
                Text(
                    """
                    These changed here and on another device in ways that can't both stand. Choose what \
                    to keep; you can undo it like any change.
                    """
                )
                .foregroundStyle(.secondary)
                if let error = store.error {
                    Text(error).foregroundStyle(.red)
                }
            }
            ForEach(store.rows) { row in
                Section(row.title) {
                    switch row.kind {
                    case .fields(let questions):
                        ForEach(questions) { question in
                            Picker(question.title, selection: choice(row.id, question.field)) {
                                Text("\(store.thisDevice): \(question.mine)").tag(SyncSide.mine)
                                Text("\(row.device): \(question.theirs)").tag(SyncSide.theirs)
                            }
                            .pickerStyle(.inline)
                        }
                    case .removed(let side):
                        Text(removedText(by: side, device: row.device))
                        Picker("Keep it?", selection: choice(row.id, .deletedAt)) {
                            // The side that deleted it is Delete.
                            Text("Keep").tag(side == .theirs ? SyncSide.mine : .theirs)
                            Text("Delete").tag(side)
                        }
                        .pickerStyle(.segmented)
                    }
                }
            }
        }
        .sheetLayout(width: 520, height: 480)
        .navigationTitle("Review Changes")
        #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
        #endif
        .disabled(store.isResolving)
        .toolbar {
            ToolbarItem(placement: .cancellationAction) {
                Button("Later") { store.send(.laterTapped) }
            }
            ToolbarItem(placement: .confirmationAction) {
                Button("Done") { store.send(.doneTapped) }
            }
        }
    }

    private func removedText(by side: SyncSide, device: String) -> String {
        switch side {
        case .theirs: String(localized: "\(device) deleted it, and it was changed here.")
        case .mine: String(localized: "It was deleted here, and \(device) changed it.")
        }
    }

    private func choice(_ record: RecordID, _ field: Field) -> Binding<SyncSide> {
        Binding(
            get: { store.state.choice(record, field) },
            set: { store.send(.chose(record, field, $0)) })
    }
}
