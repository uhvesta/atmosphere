import ComposableArchitecture
import FlowKit
import LifrAssistantClient
import LifrSkills
import SnapshotTesting
import SwiftUI
import XCTest

/// The skill list with Undo off and one of the person's, and the editor
/// with the assistant's help open.
@MainActor
final class SkillsSnapshotTests: XCTestCase {
    private var mine: Skill {
        Skill(
            userNamed: "Weekly review", summary: "Fridays: what's done, what's next.",
            instructions: "Go through the week.", tools: [.overview], draftsPlans: false)!
    }

    private func list() -> some View {
        var undo = Skill.undo
        undo.isOn = false
        return NavigationStack {
            SkillsView(
                store: Store(initialState: SkillsFeature.State(skills: Skill.merged(stored: [undo, mine]))) {
                    EmptyReducer()
                })
        }
    }

    private func editor() -> some View {
        var state = SkillEditor.State(editing: mine)
        state.assists = true
        state.request = "Every Friday, show what I finished and what's overdue"
        return SkillEditorView(store: Store(initialState: state) { EmptyReducer() })
    }

    func testImages() {
        assertScreenImages(of: list(), named: "list")
        assertScreenImages(of: editor(), named: "editor", images: [.proMaxLight, .proMaxDark])
    }
}
