import ComposableArchitecture
import SwiftUI

package struct BugReportsView: View {
    @Bindable package var store: StoreOf<BugReportsFeature>

    package init(store: StoreOf<BugReportsFeature>) {
        self.store = store
    }

    package var body: some View {
        List {
            if let error = store.error {
                Section {
                    Text(error.description).foregroundStyle(.red)
                    Button("Try Again") { store.send(.task) }
                }
            }
            if store.isLoaded, store.error == nil, store.saved.reports.isEmpty, store.saved.unreadable.isEmpty {
                Text(emptyText).foregroundStyle(.secondary)
            }
            ForEach(store.saved.reports) { report in
                Button {
                    store.send(.openTapped(report.id))
                } label: {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(report.happened.isEmpty ? String(localized: "No description") : report.happened)
                            .lineLimit(2)
                            .foregroundStyle(Color.primary)
                        Text(report.createdAt.formatted(date: .abbreviated, time: .shortened))
                            .font(.footnote)
                            .foregroundStyle(Color.secondary)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .swipeActions {
                    Button("Delete", role: .destructive) { store.send(.deleteTapped(report.id)) }
                }
                .contextMenu { actions(for: report) }
            }
            ForEach(store.saved.unreadable) { unreadable in
                VStack(alignment: .leading, spacing: 4) {
                    Text(unreadable.error.description).font(.footnote)
                    Button("Delete", role: .destructive) { store.send(.deleteTapped(unreadable.id)) }
                }
            }
        }
        .navigationTitle("Bug Reports")
        .navigationDestination(item: $store.opened) { report in
            BugReportDetail(report: report) {
                actions(for: report)
            }
        }
        .task { await store.send(.task).finish() }
    }

    private var emptyText: String {
        #if os(iOS)
            String(localized: "No bug reports. Shake your iPhone to report one.")
        #else
            String(localized: "No bug reports. Choose Help › Report a Bug to make one.")
        #endif
    }

    @ViewBuilder
    private func actions(for report: BugReport) -> some View {
        Button(store.copied == report.id ? "Copied" : "Copy") { store.send(.copyTapped(report.id)) }
        ShareLink(item: BugReportFile(report: report), preview: SharePreview(Text("Lifr bug report"))) {
            Text("Share")
        }
        Button("Delete", role: .destructive) { store.send(.deleteTapped(report.id)) }
    }
}

/// One saved report, as text.
private struct BugReportDetail<Actions: View>: View {
    let report: BugReport
    @ViewBuilder let actions: () -> Actions

    var body: some View {
        ScrollView {
            Text(report.text)
                .font(.system(.footnote, design: .monospaced))
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding()
        }
        .navigationTitle(report.createdAt.formatted(date: .abbreviated, time: .shortened))
        .toolbar {
            ToolbarItemGroup { actions() }
        }
    }
}
