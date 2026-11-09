import ChatKit
import ComposableArchitecture
import Foundation
import LifrAPI
import LifrCore
import LifrModelsClient

/// Talks with the person about what to add and drafts it as a `Plan`. The
/// live version runs Apple's on-device model, so nothing leaves the device.
@DependencyClient
package struct AssistantClient: Sendable {
    // Each takes what the conversation runs on in place of the defaults.

    /// Whether this device can run the assistant now.
    package var availability: @Sendable (_ choice: ModelChoice) -> AssistantAvailability = { _ in .deviceNotEligible }
    /// The assistant's answer to the last message in `request`, as it's
    /// written: each element is the whole answer so far, the last one complete.
    package var reply: @Sendable (_ request: AssistantRequest) -> AssistantReplies = { _ in
        AssistantReplies { $0.finish(throwing: AssistantError.unavailable(.deviceNotEligible)) }
    }
    /// A skill written from the person's description of it, for them to
    /// review before saving, by `model` or the one picked for skills.
    package var draftSkill: @Sendable (_ description: String, _ model: ModelID?) async throws -> SkillDraft
    /// Which of `skills` fits the last message in `messages`, by ID, or nil
    /// when none does and the assistant should answer as it is.
    ///
    /// `drafting` is the skill drafting the plan open in the chat, if one is.
    package var pickSkill:
        @Sendable (_ messages: [ChatMessage], _ skills: [Skill], _ drafting: String?, _ choice: ModelChoice)
            async throws -> String?
    /// A short title for a conversation with these messages, by `model` or
    /// the one picked for titles.
    package var suggestTitle: @Sendable (_ messages: [ChatMessage], _ model: ModelID?) async throws -> String
    /// A short summary of `messages`, which follow the summary `earlier`
    /// when there is one, to send in their place once a conversation is
    /// longer than the model reads.
    package var compact:
        @Sendable (_ messages: [ChatMessage], _ earlier: String?, _ choice: ModelChoice) async throws -> String
    /// Loads the model ahead of the first message, so the first answer
    /// starts sooner. Does nothing when it's loaded or can't run.
    package var prewarm: @Sendable (_ choice: ModelChoice) -> Void = { _ in }
    /// What the model picked for the assistant reads besides text, such as
    /// photos and recordings.
    package var inputs: @Sendable (_ choice: ModelChoice) -> Set<ModelInput> = { _ in [] }
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
    /// The model picked needs more memory than the device has free: the
    /// bytes it takes with room to spare, and the bytes free now.
    case notEnoughMemory(needed: Int64, available: Int64)

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
        case .notEnoughMemory(let needed, let available):
            "The model picked for the assistant needs about \(Self.bytes(needed)) of memory and Lifr has "
                + "\(Self.bytes(available)) free now. Close other apps and try again, or pick a smaller model or "
                + "context window for this chat, or in Settings › Models."
        }
    }

    private static func bytes(_ count: Int64) -> String {
        ByteCountFormatter.string(fromByteCount: count, countStyle: .memory)
    }
}

/// One message in a conversation.
package struct ChatMessage: ChatEntry {
    package typealias Role = ChatRole

    package var id: UUID
    package var role: Role
    package var text: String
    package var at: Date
    /// The ID of the skill the person picked for this message, if any.
    package var skill: String?
    /// The photos and recordings the person sent with it.
    package var attachments: [ChatAttachment]
    /// What the model thought before it wrote this reply.
    package var thinking: ThreadThinking?
    /// What this reply asks to change, and whether the person applied it.
    package var proposal: Proposal?
    /// The model that wrote a reply, by name.
    package var model: String?
    /// How fast the model read the conversation and wrote a reply.
    package var speed: ThreadSpeed?

    package init(
        id: UUID, role: Role, text: String, at: Date, skill: String? = nil, attachments: [ChatAttachment] = [],
        thinking: ThreadThinking? = nil, proposal: Proposal? = nil, model: String? = nil, speed: ThreadSpeed? = nil
    ) {
        self.id = id
        self.role = role
        self.text = text
        self.at = at
        self.skill = skill
        self.attachments = attachments
        self.thinking = thinking
        self.proposal = proposal
        self.model = model
        self.speed = speed
    }

    private enum CodingKeys: String, CodingKey {
        case id, role, text, at, skill, attachments, thinking, proposal, model, speed
    }

    /// Reads messages saved before they had attachments, thinking,
    /// proposals, a model or a speed as having none.
    package init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(UUID.self, forKey: .id)
        role = try container.decode(Role.self, forKey: .role)
        text = try container.decode(String.self, forKey: .text)
        at = try container.decode(Date.self, forKey: .at)
        skill = try container.decodeIfPresent(String.self, forKey: .skill)
        attachments = try container.decodeIfPresent([ChatAttachment].self, forKey: .attachments) ?? []
        thinking = try container.decodeIfPresent(ThreadThinking.self, forKey: .thinking)
        proposal = try container.decodeIfPresent(Proposal.self, forKey: .proposal)
        model = try container.decodeIfPresent(String.self, forKey: .model)
        speed = try container.decodeIfPresent(ThreadSpeed.self, forKey: .speed)
    }

    package func encode(to encoder: any Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(id, forKey: .id)
        try container.encode(role, forKey: .role)
        try container.encode(text, forKey: .text)
        try container.encode(at, forKey: .at)
        try container.encodeIfPresent(skill, forKey: .skill)
        if !attachments.isEmpty { try container.encode(attachments, forKey: .attachments) }
        try container.encodeIfPresent(thinking, forKey: .thinking)
        try container.encodeIfPresent(proposal, forKey: .proposal)
        try container.encodeIfPresent(model, forKey: .model)
        try container.encodeIfPresent(speed, forKey: .speed)
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
    /// That the assistant may propose changes. Without it, it only reads
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
    /// The most characters of conversation sent at full size: more for a
    /// model with a larger window. See `AssistantPrompt.historyBudget(for:)`.
    package var historyBudget = AssistantPrompt.historyBudget
    /// What the assistant wrote of the conversation before `messages`, sent
    /// ahead of them.
    package var summary: String?
    /// What the conversation runs on in place of the defaults.
    package var model = ModelChoice.defaults

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

    /// The tools the skill allows, or none without one: then the assistant
    /// just talks. Tools that change records need `writes`.
    package var allowedTools: Set<AssistantTool> {
        let tools = skill?.tools ?? []
        return writes == nil ? tools.filter { !$0.writes } : tools
    }

    /// Whether the assistant may change existing tasks, in its answer's
    /// `changes`.
    package var changesTasks: Bool { allowedTools.contains(.updateTasks) }

    /// Whether the assistant may change existing Projects, in its answer's
    /// `projectChanges`.
    package var changesProjects: Bool { allowedTools.contains(.updateProjects) }

    /// Whether the assistant may propose a plan: when the skill drafts them.
    package var draftsPlans: Bool { skill?.draftsPlans ?? false }

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

    /// A Project of any status, which the assistant may change.
    package struct KnownProject: Hashable, Sendable {
        /// "KEY/Project name": how a change names it.
        package var place: String
        package var name: String
        package var status: ProjectStatus
        package var due: CalendarDate?

        package init(place: String, name: String, status: ProjectStatus, due: CalendarDate? = nil) {
            self.place = place
            self.name = name
            self.status = status
            self.due = due
        }
    }

    package var places: [Place]
    package var tasks: [OpenTask]
    package var labels: [String]
    /// Every Project in an Avenue that isn't archived, active, completed or
    /// archived, in sidebar order.
    package var projects: [KnownProject]

    package init(
        places: [Place] = [], tasks: [OpenTask] = [], labels: [String] = [], projects: [KnownProject] = []
    ) {
        self.places = places
        self.tasks = tasks
        self.labels = labels
        self.projects = projects
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
        projects = avenues.flatMap { avenue in
            workspace.projects.values
                .filter { $0.avenueID == avenue.id }
                .sorted { ($0.sortOrder, $0.id.uuidString) < ($1.sortOrder, $1.id.uuidString) }
                .map { project in
                    KnownProject(
                        place: avenue.key.rawValue + "/" + project.name, name: project.name, status: project.status,
                        due: project.dueDate)
                }
        }
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
    /// What the model thought before answering, and how long it took, for
    /// a model that says.
    package var thinking: ModelThinking?
    /// How many of the request's messages the model didn't get, as they
    /// didn't fit: older ones, which a summary can stand in for.
    package var leftOut = 0
    /// How much of the model's window the turn took, estimated, once the
    /// reply is complete.
    package var context: ContextUse?
    /// What it asks to change, for the person to apply.
    package var proposal: Proposal?

    package init(
        text: String, plan: Plan? = nil, isComplete: Bool = false, thinking: ModelThinking? = nil, leftOut: Int = 0
    ) {
        self.text = text
        self.plan = plan
        self.isComplete = isComplete
        self.thinking = thinking
        self.leftOut = leftOut
    }
}

/// Why the assistant couldn't answer.
package enum AssistantError: Error, Hashable, Sendable, CustomStringConvertible {
    case unavailable(AssistantAvailability)
    /// The last message, the draft and the instructions don't fit the
    /// model, even with older messages, labels and tools left out.
    case tooLong
    /// The model wrote nothing for a minute or more, and Lifr stopped
    /// waiting.
    case timedOut
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
    /// asking again.
    case unreadable
    /// Lifr stopped the model in the middle of the answer: the device ran
    /// low on memory, or Lifr went to the background.
    case interrupted(AssistantInterruption)
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
        case .tooLong:
            "Your message and the draft are more than the on-device model can read at once, even with older "
                + "messages left out. Shorten the message, ask for less at once, or remove rows from the draft."
        case .timedOut:
            "The assistant stopped answering, so Lifr stopped waiting. Send the message again; if it keeps "
                + "happening, ask for less at once."
        case .declined:
            "The assistant can't help with that. Rephrase the request, or edit the draft by hand."
        case .busy:
            "The assistant is busy. Lifr tries again when it's back on screen; or send the message again in a moment."
        case .simulatorHost(let detail):
            "The Simulator couldn't use this Mac's Apple Intelligence (\(detail)). This usually means the Simulator "
                + "runtime and macOS are different versions: update Xcode and its iOS Simulator to match macOS, or "
                + "run Lifr on a device or as the Mac app."
        case .unreadable:
            "The on-device model wrote answers Lifr couldn't read, twice. Send the message again, or ask for less "
                + "at once: long plans are the hardest for it."
        case .interrupted(let reason):
            "\(reason.description) Send the message again."
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
            availability: { _ in .available },
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
            draftSkill: { _, _ in .example },
            pickSkill: { _, _, _, _ in nil },
            suggestTitle: { _, _ in "Planning the week" },
            compact: { _, _, _ in AssistantPrompt.exampleSummary },
            prewarm: { _ in },
            inputs: { _ in inputs })
    }

    /// This client, but its replies to the calls `failures` numbers, from
    /// 1, fail with the error given for each.
    package func failing(_ failures: [Int: AssistantError]) -> AssistantClient {
        var client = self
        let calls = LockIsolated(0)
        client.reply = { [reply] request in
            let call = calls.withValue { count in
                count += 1
                return count
            }
            guard let failure = failures[call] else { return reply(request) }
            return AssistantReplies { $0.finish(throwing: failure) }
        }
        return client
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
