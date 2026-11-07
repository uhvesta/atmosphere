import Foundation
import LifrCore

/// One change the assistant asks to make to records that exist. As data,
/// so a proposal keeps with its conversation and can be edited, then made
/// again on the records as they are when the person applies it.
package enum AssistantAction: Hashable, Codable, Sendable {
    case task(TaskChange)
    case project(ProjectChange)
    case comment(code: String, text: String)
    /// Puts back what these journal entries changed.
    case revert(entries: [UUID])
}

/// What the assistant asked to change in one reply. Nothing of it is
/// saved until the person applies it, as one change Undo reverts.
package struct Proposal: Hashable, Codable, Sendable {
    package var actions: [AssistantAction]
    /// What the actions change, record by record, as the records were when
    /// they were proposed or last edited.
    package var steps: [JournalStep]
    package var decision: Decision?

    package enum Decision: String, Hashable, Codable, Sendable {
        case applied, declined
    }

    package init(actions: [AssistantAction], steps: [JournalStep], decision: Decision? = nil) {
        self.actions = actions
        self.steps = steps
        self.decision = decision
    }

    /// How many of the actions revert earlier changes.
    package var reverts: Int {
        actions.reduce(0) { count, action in
            if case .revert(let entries) = action { count + entries.count } else { count }
        }
    }

    /// The actions made on `workspace` without saving anything: what they
    /// would change now, and the operation that saves them as the assistant
    /// in `conversation`. Throws the first that can't be made any more,
    /// such as for a task deleted since.
    package static func make(
        _ actions: [AssistantAction], on workspace: Workspace, now: Date, timeZone: TimeZone, conversation: UUID
    ) async throws(ActionError) -> (proposal: Proposal, operation: AssistantOperation) {
        let tools = AssistantTools(
            workspace: workspace, now: now, timeZone: timeZone, writes: AssistantWrites(conversation: conversation))
        for action in actions {
            try await tools.run(action)
        }
        guard let proposal = tools.proposal, let operation = tools.operation else { throw .nothingToChange }
        return (proposal, operation)
    }
}

/// Why a change the assistant asked for can't be made. Nothing of it is
/// saved.
package enum ActionError: Error, Hashable, Sendable, CustomStringConvertible {
    case task(TaskChangeError)
    case project(ProjectChangeError)
    /// The task to comment on isn't there.
    case noTask(code: String)
    /// A change to revert isn't in the journal, such as one from a
    /// device that hasn't synced.
    case noChange(ref: String)
    /// The actions change nothing, as the records are now.
    case nothingToChange
    /// The conversation can't change anything, such as with a skill that
    /// only reads.
    case readOnly
    /// A Core rule refused it.
    case refused(LifrError)

    package var description: String {
        switch self {
        case .task(let error): error.description
        case .project(let error): error.description
        case .noTask(let code): "There's no task \(code), so the comment wasn't added. Look up the task's code."
        case .noChange(let ref): "There's no change \(ref) to revert. Look up its ref in the recent activity."
        case .nothingToChange: "The records already are as asked, so there's nothing to change."
        case .readOnly: TaskChangeError.readOnly.description
        case .refused(let error): "\(error) Nothing changed."
        }
    }
}
