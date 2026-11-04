import ComposableArchitecture
import LifrAPI
import LifrCore
import LifrSessionClient
import LifrSettings
import LifrSnapshotTesting
import LifrStoreClient
import SnapshotTesting
import SwiftUI
import XCTest

/// Settings for the sample records with Claude Code connected, and the sheet
/// that approves it, as of 2026-11-03 10:00 UTC.
@MainActor
final class SettingsSnapshotTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_793_700_000)

    private var settings: SettingsFeature.State {
        var state = SettingsFeature.State(showsAgents: true)
        state.show(.sample(now: now, timeZone: .gmt))
        state.apps = [
            ConnectedApp(
                id: "com.anthropic.claude-code", grant: .agentDefault(client: "Claude Code", now: now),
                lastUsedAt: now - 3600, isConnected: true)
        ]
        state.cli = .installed(path: "/usr/local/bin/lifr")
        return state
    }

    private var approval: ApprovalFeature.State {
        ApprovalFeature.State(
            request: ApprovalRequest(
                id: UUID(0), identity: "com.anthropic.claude-code", client: "Claude Code", method: .searchTasks,
                requestedAt: now),
            workspace: .sample(now: now, timeZone: .gmt))
    }

    private func settingsScreen() -> some View {
        NavigationStack {
            SettingsView(store: Store(initialState: settings) { EmptyReducer() })
        }
        .environment(\.timeZone, .gmt)
    }

    private func approvalScreen() -> some View {
        NavigationStack {
            ApprovalView(store: Store(initialState: approval) { BindingReducer() })
        }
        .environment(\.timeZone, .gmt)
    }

    func testDisplayData() {
        let settings = settings
        assertSnapshot(
            of: (
                procrastinatedAfterDays: settings.procrastinatedAfterDays, avenues: settings.avenues,
                apps: settings.apps, cli: settings.cli
            ),
            as: Snapshotting<Any, String>.dump.pullback { $0 }, named: "settings")
        let approval = approval
        assertSnapshot(
            of: (avenues: approval.avenues, canApprove: approval.canApprove, grant: approval.grant(now: now)),
            as: Snapshotting<Any, String>.dump.pullback { $0 }, named: "approval")
    }

    func testAccessibilityTree() {
        assertSnapshot(
            of: settingsScreen(), as: .accessibilityTree(size: CGSize(width: 402, height: 874)), named: "settings")
        assertSnapshot(
            of: approvalScreen(), as: .accessibilityTree(size: CGSize(width: 402, height: 874)), named: "approval")
    }

    func testImages() {
        assertScreenImages(of: settingsScreen(), named: "settings")
        assertScreenImages(of: approvalScreen(), named: "approval")
    }
}
