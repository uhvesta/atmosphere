import ComposableArchitecture
import FlowKit
import LifrAPI
import LifrAssistantClient
import LifrCore
import LifrSessionClient
import LifrSettings
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

    /// Two saved versions, the older one in use.
    private var instructions: InstructionsFeature.State {
        var guidance = Guidance()
        try? guidance.save("Keep answers short. Ask before adding more than five tasks.", id: UUID(1), at: now - 86400)
        try? guidance.save("Answer in Spanish.", id: UUID(2), at: now - 3600)
        guidance.use(UUID(1))
        return InstructionsFeature.State(guidance: guidance, isLoaded: true)
    }

    private func instructionsScreen() -> some View {
        NavigationStack {
            InstructionsView(store: Store(initialState: instructions) { EmptyReducer() })
        }
        .environment(\.timeZone, .gmt)
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
        let instructions = instructions
        assertSnapshot(
            of: (guidance: instructions.guidance, draft: instructions.draft, canSave: instructions.canSave),
            as: Snapshotting<Any, String>.dump.pullback { $0 }, named: "instructions")
    }

    func testAccessibilityTree() {
        assertSnapshot(
            of: settingsScreen(), as: .accessibilityTree(size: CGSize(width: 402, height: 874)), named: "settings")
        assertSnapshot(
            of: approvalScreen(), as: .accessibilityTree(size: CGSize(width: 402, height: 874)), named: "approval")
        assertSnapshot(
            of: instructionsScreen(), as: .accessibilityTree(size: CGSize(width: 402, height: 874)),
            named: "instructions")
    }

    func testImages() {
        assertScreenImages(of: settingsScreen(), named: "settings")
        assertScreenImages(of: approvalScreen(), named: "approval")
        assertScreenImages(of: instructionsScreen(), named: "instructions")
    }
}
