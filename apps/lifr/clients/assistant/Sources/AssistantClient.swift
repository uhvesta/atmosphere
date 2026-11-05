import ComposableArchitecture
import Foundation
import LifrAPI
import LifrCore
import LifrModelsClient

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
    /// A skill written from the person's description of it, for them to
    /// review before saving.
    package var draftSkill: @Sendable (_ description: String) async throws -> SkillDraft
    /// A short title for a conversation with these messages.
    package var suggestTitle: @Sendable (_ messages: [ChatMessage]) async throws -> String
    /// Loads the model ahead of the first message, so the first answer
    /// starts sooner. Does nothing when it's loaded or can't run.
    package var prewarm: @Sendable () -> Void = {}
    /// What the model picked for the assistant reads besides text, such as
    /// photos and recordings.
    package var inputs: @Sendable () -> Set<ModelInput> = { [] }
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
    /// The model picked for the assistant isn't on the device.
    case notDownloaded
    /// The model picked needs more memory than the device has free.
    case notEnoughMemory

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
        case .notDownloaded:
            "The model picked for the assistant isn't on this device. Download it again in Settings › Models, or "
                + "pick another model there."
        case .notEnoughMemory:
            "The model picked for the assistant needs more memory than this device has free. Close other apps "
                + "and try again, or pick a smaller model in Settings › Models."
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
    /// The ID of the skill the person picked for this message, if any.
    package var skill: String?
    /// The photos and recordings the person sent with it.
    package var attachments: [ChatAttachment]

    package init(
        id: UUID, role: Role, text: String, at: Date, skill: String? = nil, attachments: [ChatAttachment] = []
    ) {
        self.id = id
        self.role = role
        self.text = text
        self.at = at
        self.skill = skill
        self.attachments = attachments
    }

    private enum CodingKeys: String, CodingKey { case id, role, text, at, skill, attachments }

    /// Reads messages saved before they had attachments as having none.
    package init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(UUID.self, forKey: .id)
        role = try container.decode(Role.self, forKey: .role)
        text = try container.decode(String.self, forKey: .text)
        at = try container.decode(Date.self, forKey: .at)
        skill = try container.decodeIfPresent(String.self, forKey: .skill)
        attachments = try container.decodeIfPresent([ChatAttachment].self, forKey: .attachments) ?? []
    }

    package func encode(to encoder: any Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(id, forKey: .id)
        try container.encode(role, forKey: .role)
        try container.encode(text, forKey: .text)
        try container.encode(at, forKey: .at)
        try container.encodeIfPresent(skill, forKey: .skill)
        if !attachments.isEmpty { try container.encode(attachments, forKey: .attachments) }
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
    /// How the assistant saves changes it makes. Without it, it only reads
    /// and drafts plans.
    package var writes: AssistantWrites?
    /// The skill the person picked for this message. Without one, the
    /// assistant may do what any built-in skill does.
    package var skill: Skill?
    /// How the person wants the assistant to behave: the opening of its
    /// instructions, before what Lifr adds.
    package var guidance: String
    /// The files of the photos and recordings sent with the new message,
    /// those still on this device.
    package var attachments: [ModelAttachment]

    package init(
        messages: [ChatMessage], draft: Plan? = nil, problem: String? = nil, workspace: Workspace, now: Date,
        timeZone: TimeZone, writes: AssistantWrites? = nil, skill: Skill? = nil,
        guidance: String = Guidance.defaultText, attachments: [ModelAttachment] = []
    ) {
        self.messages = messages
        self.draft = draft
        self.problem = problem
        self.workspace = workspace
        self.now = now
        self.timeZone = timeZone
        self.writes = writes
        self.skill = skill
        self.guidance = guidance
        self.attachments = attachments
    }

    /// The tools the skill allows, or every tool without one. Tools that
    /// save changes need `writes`.
    package var allowedTools: Set<AssistantTool> {
        let tools = skill?.tools ?? Set(AssistantTool.allCases)
        return writes == nil ? tools.filter { !$0.writes } : tools
    }

    /// Whether the assistant may propose a plan: without a skill, or when
    /// the skill drafts plans.
    package var draftsPlans: Bool { skill?.draftsPlans ?? true }

    package var today: CalendarDate { CalendarDate(now, in: timeZone) }
    package var lookup: AssistantLookup { AssistantLookup(workspace) }
    /// Tools for one answer.
    package func tools() -> AssistantTools {
        AssistantTools(workspace: workspace, now: now, timeZone: timeZone, writes: writes)
    }
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
            .filter { $0.deletedAt == nil && $0.isOpen && $0.parentID == nil }
            .sorted { ($0.createdAt, $0.id.uuidString) > ($1.createdAt, $1.id.uuidString) }
            .compactMap { task in
                guard var place = keys[task.avenueID] else { return nil }
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
    /// The last message, the draft and the instructions don't fit the
    /// model, even with older messages, labels and tools left out. `saved`
    /// when it changed tasks before running out of room.
    case tooLong(saved: Bool)
    /// The model wrote nothing for a minute or more, and Lifr stopped
    /// waiting. `saved` when it changed tasks before that.
    case timedOut(saved: Bool)
    /// The model declined, such as for content it won't write about.
    case declined
    /// The model is busy with another request, or iOS is limiting it while
    /// Lifr is in the background.
    case busy
    /// The Simulator couldn't reach the Mac's model. It forwards requests
    /// to macOS, which only works when the Simulator runtime and macOS
    /// are the same version.
    case simulatorHost(String)
    /// The model's answer didn't fit what Lifr asked for, even after
    /// asking again. `saved` when it changed tasks before that.
    case unreadable(saved: Bool)
    /// Lifr stopped the model in the middle of the answer: the device ran
    /// low on memory, or Lifr went to the background. `saved` when it
    /// changed tasks before that.
    case interrupted(AssistantInterruption, saved: Bool)
    /// The model picked can't read what was sent with the message.
    case cantRead(ModelInput)
    /// Anything else the model reported.
    case failed(String)

    /// Whether the model stopped for a reason of its own, which may not
    /// happen again.
    package var isFailure: Bool {
        if case .failed = self { return true }
        return false
    }

    package var description: String {
        switch self {
        case .unavailable(let availability):
            availability.description
        case .tooLong(saved: true):
            "The assistant changed tasks, then ran out of room in the on-device model. The changes are saved; undo "
                + "them or see them in Settings › Activity. Ask for the rest in a shorter message."
        case .tooLong(saved: false):
            "Your message and the draft are more than the on-device model can read at once, even with older "
                + "messages left out. Shorten the message, ask for less at once, or remove rows from the draft."
        case .timedOut(saved: false):
            "The assistant stopped answering, so Lifr stopped waiting. Send the message again; if it keeps "
                + "happening, ask for less at once."
        case .timedOut(saved: true):
            "The assistant changed tasks, then stopped answering. The changes are saved; undo them or see them in "
                + "Settings › Activity. Check them before sending the message again, or they may be made twice."
        case .declined:
            "The assistant can't help with that. Rephrase the request, or edit the draft by hand."
        case .busy:
            "The assistant is busy. Lifr tries again when it's back on screen; or send the message again in a moment."
        case .simulatorHost(let detail):
            "The Simulator couldn't use this Mac's Apple Intelligence (\(detail)). This usually means the Simulator "
                + "runtime and macOS are different versions: update Xcode and its iOS Simulator to match macOS, or "
                + "run Lifr on a device or as the Mac app."
        case .unreadable(saved: false):
            "The on-device model wrote answers Lifr couldn't read, twice. Send the message again, or ask for less "
                + "at once: long plans are the hardest for it."
        case .unreadable(saved: true):
            "The assistant changed tasks, then wrote an answer Lifr couldn't read. The changes are saved; undo them "
                + "or see them in Settings › Activity. Check them before sending the message again, or they may "
                + "be made twice."
        case .interrupted(let reason, saved: false):
            "\(reason.description) Send the message again."
        case .interrupted(let reason, saved: true):
            "\(reason.description) It had changed tasks; those changes are saved. Check them in Settings › "
                + "Activity before sending the message again, or they may be made twice."
        case .cantRead(.image):
            "The model picked for the assistant can't read photos. Pick one that can, such as Gemma 4 E4B, in "
                + "Settings › Models, or remove the photo and send the message again."
        case .cantRead(.audio):
            "The model picked for the assistant can't listen to recordings. Pick one that can, such as Gemma 4 "
                + "E4B, in Settings › Models, or remove the recording and send the message again."
        case .failed(let detail):
            "The assistant stopped: \(detail). Send the message again."
        }
    }
}

/// Why Lifr stopped the model in the middle of an answer.
package enum AssistantInterruption: Hashable, Sendable, CustomStringConvertible {
    /// The device ran low on memory.
    case memory
    /// Lifr went to the background.
    case background

    package var description: String {
        switch self {
        case .memory:
            "The device ran low on memory, so Lifr stopped the assistant's model. If it keeps happening, close "
                + "other apps or pick a smaller model in Settings › Models."
        case .background:
            "Lifr stopped the assistant's model when it went to the background."
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
    package static func scripted(
        _ replies: [AssistantReply] = [.example], inputs: Set<ModelInput> = []
    ) -> AssistantClient {
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
            },
            draftSkill: { _ in .example },
            suggestTitle: { _ in "Planning the week" },
            prewarm: {},
            inputs: { inputs })
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
