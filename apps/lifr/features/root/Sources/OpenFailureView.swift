import ComposableArchitecture
import Foundation
import LifrBackupClient
import LifrBugReport
import SwiftUI

/// Why Lifr couldn't open its records, with a way to report it and, when
/// there's a daily backup, a copy of the newest one to keep somewhere safe.
package struct OpenFailureView<Retry: View>: View {
    @Bindable private var store: StoreOf<RootFeature>
    private let reason: String
    private let retry: Retry
    @State private var backup: URL?

    package init(store: StoreOf<RootFeature>, reason: String, @ViewBuilder retry: () -> Retry) {
        self.store = store
        self.reason = reason
        self.retry = retry()
    }

    package var body: some View {
        ContentUnavailableView {
            Label("Lifr can't open its data", systemImage: "exclamationmark.triangle")
        } description: {
            Text(reason)
            Text("If this keeps happening, report a bug with these details.")
        } actions: {
            retry
            Button("Report a Bug") { store.send(.openFailureReported(reason)) }
            if let backup {
                ShareLink("Export What Can Be Read", item: backup)
                    .help("The newest daily backup, as a file to keep or import later")
            }
            ShareLink("Share the Details", item: reason)
        }
        .task { backup = await Self.newestBackup() }
        .sheet(item: $store.scope(state: \.bugReport, action: \.bugReport)) { store in
            NavigationStack { BugReportView(store: store) }
                .interactiveDismissDisabled()
        }
    }

    /// The newest daily backup copied to a file named for its day, or nil
    /// when there's none or it can't be read.
    private static func newestBackup() async -> URL? {
        // The records didn't open, so there's no store to take it from.
        let backups = BackupClient.liveValue
        guard let newest = await backups.list().first, let data = try? await backups.read(newest.day) else {
            return nil
        }
        let file = FileManager.default.temporaryDirectory.appending(path: "Lifr-\(newest.day.description).json")
        do {
            try data.write(to: file, options: .atomic)
        } catch {
            return nil
        }
        return file
    }
}
