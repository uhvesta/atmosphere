import AssistantKit
import ComposableArchitecture
import Foundation

/// A tool the assistant can call while it answers.
package enum AssistantTool: String, Hashable, Codable, Sendable, CaseIterable {
    case findTasks, getTask, overview, recentActivity, updateTasks, updateProjects, addComment, revert

    /// Whether the tool saves changes, which needs `AssistantRequest.writes`.
    package var writes: Bool {
        switch self {
        case .findTasks, .getTask, .overview, .recentActivity: false
        case .updateTasks, .updateProjects, .addComment, .revert: true
        }
    }
}

extension AssistantTool: SkillTool {
    package static var builtInSkills: [Skill] { Skill.builtIn }
}

/// What the assistant does in a conversation: AssistantKit's skill, over
/// Lifr's tools. Lifr has a few built in; the person can write their own,
/// whose IDs start with `user:`.
package typealias Skill = AssistantKit.Skill<AssistantTool>

/// A skill the assistant wrote from the person's description, for the
/// editor to fill in.
package typealias SkillDraft = AssistantKit.SkillDraft<AssistantTool>

extension AssistantKit.Skill where Tool == AssistantTool {
    // MARK: - Built in

    package static var query: Skill {
        Skill(
            id: "query", summary: String(localized: "Answer questions about your tasks."),
            instructions: """
                Answer the person's questions about their tasks from what the tools return. Don't propose a plan.
                """,
            tools: [.findTasks, .getTask, .overview, .recentActivity], draftsPlans: false, autoPicks: true,
            useWhen: "the person asks about their tasks, Projects or Avenues: what's due, what's left, what's in one")
    }

    package static var add: Skill {
        Skill(
            id: "add", summary: String(localized: "Plan new tasks, Projects and Avenues."),
            instructions: """
                Help the person plan what to add. Ask what's missing, then put the whole plan in "plan".
                """,
            tools: [.findTasks, .getTask], draftsPlans: true, autoPicks: true,
            useWhen: "the person names one or a few new things to add, or answers about a plan being drafted")
    }

    package static var change: Skill {
        Skill(
            id: "change", summary: String(localized: "Rename, move, reschedule or finish tasks and Projects."),
            instructions: """
                Change the tasks or Projects the person means: put each change in "changes", or in \
                "projectChanges" for a Project, and say in a sentence what you'd change.
                """,
            tools: [.findTasks, .getTask, .updateTasks, .updateProjects], draftsPlans: false, autoPicks: true,
            useWhen: """
                the person wants tasks or Projects they already have renamed, moved, rescheduled or reprioritised, \
                or marked done, such as "I did…" or "mark it done"
                """)
    }

    package static var breakdown: Skill {
        Skill(
            id: "breakdown", summary: String(localized: "Turn a brain dump into a plan."),
            instructions: """
                The person is dumping what's on their mind. First understand it: ask short questions, one or two at \
                a time, about what's unclear: what each thing is, which Avenue or Project it belongs in, what has to \
                come first, and any deadlines. Leave "plan" out while you're still asking. Once you understand, plan \
                it in "plan": tasks in the order to do them, with steps as Sub tasks, and a Project where several \
                tasks serve one goal.
                """,
            tools: [.findTasks], draftsPlans: true, autoPicks: true,
            useWhen: "the person dumps many things they have to do, or a big goal that needs breaking into steps")
    }

    package static var tidy: Skill {
        Skill(
            id: "tidy", summary: String(localized: "Clean up titles, priorities, dates and labels."),
            instructions: """
                Help tidy existing tasks: clearer titles, missing priorities and due dates, consistent labels. \
                Say what you'd change first, and put it in "changes" once the person agrees.
                """,
            tools: [.findTasks, .getTask, .overview, .updateTasks, .updateProjects], draftsPlans: false,
            useWhen: "the person asks to clean up or tidy their tasks")
    }

    package static var review: Skill {
        Skill(
            id: "review", summary: String(localized: "Look back at what's done, overdue and stalled."),
            instructions: """
                Review the person's week with them: what got done, what's overdue, and what has stalled. Suggest \
                what to drop or move, and add a comment to a task when they ask.
                """,
            tools: [.findTasks, .getTask, .overview, .recentActivity, .addComment], draftsPlans: false,
            useWhen: "the person asks to look back over their week")
    }

    package static var undo: Skill {
        Skill(
            id: "undo", summary: String(localized: "Put back recent changes."),
            instructions: """
                Help the person put back changes. Find them with recentActivity, say which you'd revert, and \
                revert them once the person agrees.
                """,
            tools: [.recentActivity, .revert], draftsPlans: false,
            useWhen: "the person wants a recent change put back")
    }

    package static var builtIn: [Skill] { [.query, .add, .change, .breakdown, .tidy, .review, .undo] }

    /// Whether every answer must hold a plan: the Add skill.
    package var requiresPlan: Bool { id == Self.add.id }
    /// Whether answers must hold changes to tasks or Projects: the Change skill.
    package var requiresChange: Bool { id == Self.change.id }
    /// Whether the skill plans Projects itself, so the person needn't ask
    /// for one by name: the Break down skill.
    package var suggestsProjects: Bool { id == Self.breakdown.id }
}

extension AssistantKit.SkillDraft where Tool == AssistantTool {
    package static var example: SkillDraft {
        SkillDraft(
            name: "Weekly review", summary: "Fridays: what got done and what's next.",
            instructions:
                "Go through what got done this week and what's overdue, then suggest the three tasks to do next.",
            tools: [.findTasks, .overview, .recentActivity], draftsPlans: false)
    }
}

/// Keeps the person's skills, and which skills are on, with conversations:
/// on this device, or in iCloud when conversations sync.
@DependencyClient
package struct SkillsClient: Sendable {
    /// The built-in skills, then the person's, each as on or off as it was
    /// left.
    package var load: @Sendable () async throws -> [Skill]
    /// Adds or replaces a skill of the person's, or turns a built-in one on
    /// or off. A built-in's instructions can't change.
    package var save: @Sendable (_ skill: Skill) async throws -> Void
    /// Removes skills of the person's. Messages that used one still show
    /// its ID.
    package var delete: @Sendable (_ ids: [String]) async throws -> Void
}

extension SkillsClient {
    /// Keeps skills in memory, for previews and tests.
    package static func inMemory(_ skills: [Skill] = []) -> Self {
        let stored = LockIsolated(Dictionary(skills.map { ($0.id, $0) }) { $1 })
        return SkillsClient(
            load: { Skill.merged(stored: Array(stored.value.values)) },
            save: { skill in stored.withValue { $0[skill.id] = skill } },
            delete: { ids in stored.withValue { for id in ids { $0[id] = nil } } })
    }
}

extension SkillsClient: TestDependencyKey {
    package static let testValue = SkillsClient()
    package static let previewValue = SkillsClient.inMemory()
}

extension DependencyValues {
    package var skills: SkillsClient {
        get { self[SkillsClient.self] }
        set { self[SkillsClient.self] = newValue }
    }
}
