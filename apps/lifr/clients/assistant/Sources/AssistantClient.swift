import ComposableArchitecture
import Foundation
import LifrAPI
import LifrCore

/// Talks with the person about what to add and drafts it as a `Plan`. The
/// live version runs Apple's on-device model, so nothing leaves the device.
@DependencyClient
package struct AssistantClient: Sendable {
    /// Whether this device can run the assistant now.
    package var availability: @Sendable () -> AssistantAvailability = { .deviceNotEligible }
    /// The assistant's answer to the last message in `request`, as it's
    /// written: each element is the whole answer so far, the last one complete.
    package var reply: @Sendable (_ request: AssistantRequest) -> AssistantReplies = { _ in
        AssistantReplies { $0.finish(throwing: AssistantError.unavailable(.deviceNotEligible)) }
    }
}

/// Each reply as it's written, the last one complete.
package typealias AssistantReplies = AsyncThrowingStream<AssistantReply, any Error>

/// Why the assistant can or can't run, with what the person can do about it.
package enum AssistantAvailability: Hashable, Sendable, CustomStringConvertible {
    case available
    /// Older than iOS 26 or macOS 26.
    case systemTooOld
    /// The device can't run Apple Intelligence.
    case deviceNotEligible
    /// Apple Intelligence is turned off.
    case notEnabled
    /// The model is still downloading.
    case modelNotReady

    package var description: String {
        switch self {
        case .available:
            "The assistant is ready."
        case .systemTooOld:
            "The assistant needs iOS 26 or macOS 26. Update in Settings > General > Software Update."
        case .deviceNotEligible:
            "This device can't run Apple Intelligence, which the assistant uses. You can still add tasks by hand, "
                + "or with lifr add --plan on a Mac."
        case .notEnabled:
            "Turn on Apple Intelligence in Settings > Apple Intelligence & Siri to use the assistant."
        case .modelNotReady:
            "Apple Intelligence is still downloading. Keep the device on Wi-Fi and power, then try again in a few "
                + "minutes."
        }
    }
}

/// One message in a conversation.
package struct ChatMessage: Identifiable, Hashable, Codable, Sendable {
    package enum Role: String, Hashable, Codable, Sendable { case person, assistant }

    package var id: UUID
    package var role: Role
    package var text: String
    package var at: Date

    package init(id: UUID, role: Role, text: String, at: Date) {
        self.id = id
        self.role = role
        self.text = text
        self.at = at
    }
}

/// What the assistant gets for one answer.
package struct AssistantRequest: Hashable, Sendable {
    /// The conversation, oldest first, ending with the person's new message.
    package var messages: [ChatMessage]
    /// The plan on the draft card, with the person's edits, so the
    /// assistant changes it rather than starting over.
    package var draft: Plan?
    /// Why the draft can't be added as it is, for the assistant to fix.
    package var problem: String?
    /// The records as they were when the message was sent, for the
    /// assistant to look things up in.
    package var workspace: Workspace
    package var now: Date
    package var timeZone: TimeZone

    package init(
        messages: [ChatMessage], draft: Plan? = nil, problem: String? = nil, workspace: Workspace, now: Date,
        timeZone: TimeZone
    ) {
        self.messages = messages
        self.draft = draft
        self.problem = problem
        self.workspace = workspace
        self.now = now
        self.timeZone = timeZone
    }

    package var today: CalendarDate { CalendarDate(now, in: timeZone) }
    package var lookup: AssistantLookup { AssistantLookup(workspace) }
    package var tools: AssistantTools { AssistantTools(workspace: workspace, now: now, timeZone: timeZone) }
}

/// The records the assistant may name: Avenues with their Projects, and
/// open tasks for parents. Taken from the workspace when the message is sent.
package struct AssistantLookup: Hashable, Sendable {
    package struct Place: Hashable, Sendable {
        package var key: String
        package var name: String
        package var projects: [String]

        package init(key: String, name: String, projects: [String]) {
            self.key = key
            self.name = name
            self.projects = projects
        }
    }

    package struct OpenTask: Hashable, Sendable {
        package var code: String
        package var title: String
        package var place: String

        package init(code: String, title: String, place: String) {
            self.code = code
            self.title = title
            self.place = place
        }
    }

    package var places: [Place]
    package var tasks: [OpenTask]
    package var labels: [String]

    package init(places: [Place] = [], tasks: [OpenTask] = [], labels: [String] = []) {
        self.places = places
        self.tasks = tasks
        self.labels = labels
    }

    /// Avenues that aren't archived, in sidebar order, with their active
    /// Projects; open top-level tasks, newest first.
    package init(_ workspace: Workspace) {
        let avenues = workspace.avenues.values
            .filter { $0.archivedAt == nil }
            .sorted { ($0.sortOrder, $0.id.uuidString) < ($1.sortOrder, $1.id.uuidString) }
        places = avenues.map { avenue in
            Place(
                key: avenue.key.rawValue, name: avenue.name,
                projects: workspace.projects.values
                    .filter { $0.avenueID == avenue.id && $0.status == .active }
                    .sorted { ($0.sortOrder, $0.id.uuidString) < ($1.sortOrder, $1.id.uuidString) }
                    .map(\.name))
        }
        let keys = Dictionary(uniqueKeysWithValues: avenues.map { ($0.id, $0.key.rawValue) })
        tasks = workspace.tasks.values
            .filter { $0.deletedAt == nil && $0.isOpen && $0.parentID == nil && keys[$0.avenueID] != nil }
            .sorted { ($0.createdAt, $0.id.uuidString) > ($1.createdAt, $1.id.uuidString) }
            .map { task in
                var place = keys[task.avenueID]!
                if let project = task.projectID.flatMap({ workspace.projects[$0] }) { place += "/" + project.name }
                return OpenTask(code: workspace.displayCode(of: task), title: task.title, place: place)
            }
        labels = workspace.labels.values.map(\.name).sorted()
    }
}

/// The assistant's answer so far.
package struct AssistantReply: Hashable, Sendable {
    /// What it says to the person.
    package var text: String
    /// The draft, once it has one: a new or changed plan.
    package var plan: Plan?
    /// The last reply of the stream.
    package var isComplete: Bool

    package init(text: String, plan: Plan? = nil, isComplete: Bool = false) {
        self.text = text
        self.plan = plan
        self.isComplete = isComplete
    }
}

/// Why the assistant couldn't answer.
package enum AssistantError: Error, Hashable, Sendable, CustomStringConvertible {
    case unavailable(AssistantAvailability)
    /// The conversation no longer fits the model.
    case tooLong
    /// The model declined, such as for content it won't write about.
    case declined
    /// Anything else the model reported.
    case failed(String)

    package var description: String {
        switch self {
        case .unavailable(let availability):
            availability.description
        case .tooLong:
            "This conversation is too long for the on-device model. Start a new conversation; the draft comes along."
        case .declined:
            "The assistant can't help with that. Rephrase the request, or edit the draft by hand."
        case .failed(let detail):
            "The assistant stopped: \(detail). Send the message again."
        }
    }
}

extension AssistantClient: TestDependencyKey {
    package static let testValue = AssistantClient()
    package static let previewValue = AssistantClient.scripted()
}

extension DependencyValues {
    package var assistant: AssistantClient {
        get { self[AssistantClient.self] }
        set { self[AssistantClient.self] = newValue }
    }
}

extension AssistantClient {
    /// Answers with `replies` in turn, without a model: for previews and
    /// tests. Each answer streams in two steps, the text and then the plan.
    package static func scripted(_ replies: [AssistantReply] = [.example]) -> AssistantClient {
        let next = LockIsolated(0)
        return AssistantClient(
            availability: { .available },
            reply: { _ in
                let reply = next.withValue { index in
                    defer { index += 1 }
                    return replies.isEmpty ? AssistantReply(text: "") : replies[index % replies.count]
                }
                return AsyncThrowingStream { continuation in
                    continuation.yield(AssistantReply(text: reply.text))
                    var done = reply
                    done.isComplete = true
                    continuation.yield(done)
                    continuation.finish()
                }
            })
    }
}

extension AssistantReply {
    /// Two tasks, each with Sub tasks.
    package static let example = AssistantReply(
        text: "Here's a plan with both tasks and their Sub tasks. Change anything, or tap Add.",
        plan: Plan(tasks: [
            .init(
                title: "Clean up living room", due: "saturday",
                subtasks: [.init(title: "Vacuum"), .init(title: "Dust")],
                place: "HOME"),
            .init(
                title: "Buy new clothes", subtasks: [.init(title: "Shoes"), .init(title: "Jacket")], place: "HOME"),
        ]),
        isComplete: true)
}
