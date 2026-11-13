import LifrCore
import SwiftUI

/// Somewhere a task or Project can go, as a change names it: "KEY", or
/// "KEY/Project name".
struct PlaceChoice: Hashable {
    let value: String
    let title: String

    /// The Avenues the assistant may add to, in sidebar order, each followed
    /// by its active Projects when `projects`; then the Avenues `plan` adds.
    static func all(in workspace: Workspace, adding new: [Plan.AvenueDraft] = [], projects: Bool) -> [PlaceChoice] {
        let projectsByAvenue = projects ? Dictionary(grouping: workspace.projects.values, by: \.avenueID) : [:]
        var choices: [PlaceChoice] = []
        let avenues = workspace.avenues.values
            .filter { $0.archivedAt == nil && $0.agentAccess == .full }
            .sorted { ($0.sortOrder, $0.id.uuidString) < ($1.sortOrder, $1.id.uuidString) }
        for avenue in avenues {
            let key = avenue.key.rawValue
            choices.append(PlaceChoice(value: key, title: avenue.name))
            let active = (projectsByAvenue[avenue.id] ?? [])
                .filter { $0.status == .active }
                .sorted { ($0.sortOrder, $0.id.uuidString) < ($1.sortOrder, $1.id.uuidString) }
            for project in active {
                choices.append(
                    PlaceChoice(value: "\(key)/\(project.name)", title: "\(avenue.name) › \(project.name)"))
            }
        }
        for avenue in new where !avenue.key.isEmpty && !choices.contains(where: { $0.matches(avenue.key) }) {
            let name = avenue.name.isEmpty ? avenue.key : avenue.name
            choices.append(PlaceChoice(value: avenue.key, title: String(localized: "\(name) (new)")))
        }
        return choices
    }

    /// Whether `text` names it, as Lifr reads a place: without regard to
    /// case or the spaces around the slash.
    func matches(_ text: String) -> Bool {
        Self.normalized(text) == Self.normalized(value)
    }

    private static func normalized(_ text: String) -> String {
        text.split(separator: "/", maxSplits: 1)
            .map { $0.trimmingCharacters(in: .whitespaces).lowercased() }
            .joined(separator: "/")
    }
}

/// A menu of places for a field that holds one as text. What the field holds
/// that names none of them, such as a place a model made up or one archived
/// since, stays as written and listed, so opening the editor changes nothing.
struct PlacePicker: View {
    let title: LocalizedStringKey
    @Binding var place: String?
    let choices: [PlaceChoice]
    /// The choice for no place, such as "Unchanged".
    let none: LocalizedStringKey

    var body: some View {
        Picker(title, selection: selection) {
            Text(none).tag(String?.none)
            if let unlisted {
                Text(unlisted).tag(String?.some(unlisted))
            }
            ForEach(choices, id: \.value) { choice in
                Text(choice.title).tag(String?.some(choice.value))
            }
        }
    }

    /// The field's text, when it names no choice.
    private var unlisted: String? {
        guard let place, !place.isEmpty, !choices.contains(where: { $0.matches(place) }) else { return nil }
        return place
    }

    private var selection: Binding<String?> {
        Binding {
            guard let place, !place.isEmpty else { return nil }
            return choices.first { $0.matches(place) }?.value ?? place
        } set: {
            place = $0
        }
    }
}

extension Binding<String> {
    /// The text, with an empty one as nil, for a field that must hold some.
    var optional: Binding<String?> {
        Binding<String?> {
            wrappedValue.isEmpty ? nil : wrappedValue
        } set: {
            wrappedValue = $0 ?? ""
        }
    }
}
