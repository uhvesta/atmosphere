import Foundation
import LifrCore

/// A change to one existing Project, as the assistant's answer gives it.
/// Fields left nil stay as they are.
package struct ProjectChange: Hashable, Sendable {
    /// "KEY/Project name", as the prompt lists it, or just its name.
    package var project: String
    package var name: String?
    package var details: String?
    /// active, completed or archived.
    package var status: String?
    /// A date as `CalendarDate.resolving` reads it, or "none" to clear it.
    package var due: String?

    package init(
        project: String, name: String? = nil, details: String? = nil, status: String? = nil, due: String? = nil
    ) {
        self.project = project
        self.name = name
        self.details = details
        self.status = status
        self.due = due
    }
}

/// Why a change to a Project wasn't made. Nothing of that change is saved.
package enum ProjectChangeError: Error, Hashable, Sendable, CustomStringConvertible {
    case noProject(name: String, suggestion: String?)
    case unknownStatus(String)
    case invalidDate(String)
    case nothingToChange(project: String)
    /// The conversation can't change anything, such as with a skill that
    /// only reads.
    case readOnly
    /// A Core rule refused the change, such as completing a Project with
    /// open tasks.
    case refused(project: String, LifrError)
    /// Saving failed, for a reason outside Lifr's rules.
    case notSaved(project: String, reason: String)

    package var description: String {
        switch self {
        case .noProject(let name, let suggestion):
            "There's no Project “\(name)”, so nothing was changed."
                + (suggestion.map { " Did you mean “\($0)”?" } ?? " Ask again with its name from the sidebar.")
        case .unknownStatus(let status):
            "“\(status)” isn't something a Project can be. Ask to complete, archive or reopen it."
        case .invalidDate(let text):
            "“\(text)” isn't a date Lifr reads, so the Project's due date wasn't changed. " + Plan.dateHelp
        case .nothingToChange(let project):
            "The change to “\(project)” didn't say what to change."
        case .readOnly:
            "Nothing can be changed in this conversation. Change the Project yourself, or ask without the skill."
        case .refused(let project, let error):
            "“\(project)” wasn't changed: \(error)"
        case .notSaved(let project, let reason):
            "“\(project)” wasn't changed because saving failed: \(reason) Try again; if it keeps failing, "
                + "restart Lifr."
        }
    }
}

extension AssistantTools {
    /// The words for each status the assistant can set.
    private static let projectStatuses: [String: ProjectStatus] = [
        "active": .active, "reopen": .active, "reopened": .active, "restore": .active, "restored": .active,
        "completed": .completed, "complete": .completed, "done": .completed,
        "archived": .archived, "archive": .archived,
    ]

    /// Makes `change` to its Project, all of it or none, and returns the
    /// Project after it. Fields that say what it already has change
    /// nothing. Archiving moves its open tasks out to its Avenue, so none
    /// is closed without the person asking; completing needs every task
    /// closed.
    package func change(_ change: ProjectChange) async throws(ProjectChangeError) -> Project {
        let project = try project(named: change.project)
        let newStatus = try change.status.nonEmpty.map { (status) throws(ProjectChangeError) in
            guard let match = Self.projectStatuses[status.lowercased()] else { throw .unknownStatus(status) }
            return match
        }.flatMap { $0 == project.status ? nil : $0 }
        let newDue = try change.due.nonEmpty.map { (due) throws(ProjectChangeError) -> CalendarDate? in
            if due.lowercased() == "none" { return nil }
            guard let date = CalendarDate.resolving(due, today: today) else { throw .invalidDate(due) }
            return date
        }.flatMap { $0 == project.dueDate ? nil : .some($0) }
        let newName = change.name.nonEmpty.flatMap { $0 == project.name ? nil : $0 }
        let newDetails = change.details.nonEmpty.flatMap { $0 == project.details ? nil : $0 }
        guard newStatus != nil || newDue != nil || newName != nil || newDetails != nil else {
            let given = [change.name, change.details, change.status, change.due]
            if given.contains(where: { $0.nonEmpty != nil }) { return project }
            throw .nothingToChange(project: project.name)
        }
        let id = project.id
        let saved = await apply { workspace, context throws(LifrError) in
            var saved = Change()
            if newName != nil || newDetails != nil || newDue != nil {
                try workspace.edit(
                    project: id, Workspace.ProjectEdit(name: newName, details: newDetails, dueDate: newDue))
            }
            switch newStatus {
            case .archived?: saved.merge(try workspace.archive(project: id, openTasks: .moveToAvenue, in: context))
            case let status?: try workspace.setStatus(status, ofProject: id, in: context)
            case nil: break
            }
            return saved
        }
        switch saved {
        case .success: return workspace.projects[id] ?? project
        case .readOnly: throw .readOnly
        case .refused(let error): throw .refused(project: project.name, error)
        case .failed(let reason): throw .notSaved(project: project.name, reason: reason)
        }
    }

    /// The Project "KEY/Name" or "Name" names, in any case, in an Avenue
    /// that isn't archived.
    private func project(named text: String) throws(ProjectChangeError) -> Project {
        let text = text.trimmingCharacters(in: .whitespaces)
        let parts = text.split(separator: "/", maxSplits: 1).map { $0.trimmingCharacters(in: .whitespaces) }
        let avenues = workspace.avenues.values.filter { $0.archivedAt == nil }
        let candidates: [Project]
        let name: String
        if parts.count == 2,
            let avenue = avenues.first(where: {
                $0.key.rawValue.caseInsensitiveCompare(parts[0]) == .orderedSame
                    || $0.name.caseInsensitiveCompare(parts[0]) == .orderedSame
            })
        {
            candidates = workspace.projects.values.filter { $0.avenueID == avenue.id }
            name = parts[1]
        } else {
            let shown = Set(avenues.map(\.id))
            candidates = workspace.projects.values.filter { shown.contains($0.avenueID) }
            name = parts.last ?? text
        }
        let matches = candidates.filter { $0.name.caseInsensitiveCompare(name) == .orderedSame }
        guard matches.count == 1, let project = matches.first else {
            throw .noProject(name: name, suggestion: Workspace.closest(to: name, in: candidates.map(\.name)))
        }
        return project
    }
}
