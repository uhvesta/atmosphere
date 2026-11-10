import ChatFeatures
import ChatKit
import ComposableArchitecture
import Foundation
import LifrAssistantClient
import LifrCore
import LifrModelsClient
import LifrSkills

/// What a reply of the assistant carries besides its words.
package struct ReplyChanges: Hashable, Sendable {
    /// What it asks to change, for the person to apply.
    package var proposal: Proposal?
    /// The draft it wrote or changed, for the card under the chat.
    package var plan: Plan?
    /// How many older messages it was written without, as they didn't fit.
    package var leftOut = 0
    /// The skill Lifr picked for the message it answers, sent without one.
    package var skill: String?
}

extension ChatMessage: ThreadEntry {
    package typealias Extra = ReplyChanges

    package var files: [ThreadFile] {
        attachments.filter { $0.kind == .text }.map { ThreadFile(name: $0.name ?? "File", text: $0.text ?? "") }
    }

    package var photos: [ThreadPhoto] {
        attachments.filter { $0.kind == .image }.map { ThreadPhoto(id: $0.id) }
    }

    package init(person id: UUID, text: String, at: Date, files: [ThreadFile], photos: [ThreadPhoto]) {
        let photos = photos.map { ChatAttachment(id: $0.id, kind: .image) }
        let files = files.enumerated().map { index, file in
            ChatAttachment(id: Self.fileID(index, of: id), name: file.name, text: file.text)
        }
        self.init(id: id, role: .person, text: text, at: at, attachments: photos + files)
    }

    package init(
        reply id: UUID, text: String, at: Date, thinking: ThreadThinking?, model: String?, speed: ThreadSpeed?,
        extra: ReplyChanges?
    ) {
        self.init(
            id: id, role: .assistant, text: text, at: at, thinking: thinking, proposal: extra?.proposal, model: model,
            speed: speed)
    }

    /// The ID of the message's file `index`, the same each time it's made:
    /// the message's own with its last byte moved along.
    private static func fileID(_ index: Int, of message: UUID) -> UUID {
        var bytes = message.uuid
        bytes.8 ^= 0x40
        bytes.15 &+= UInt8(truncatingIfNeeded: index + 1)
        return UUID(uuid: bytes)
    }
}

/// What the assistant is told besides the chat: the records, the draft's
/// problem, the skills, and what's picked and recorded for the next
/// message.
package struct LifrHost: Equatable, Sendable {
    package var workspace: Workspace
    /// Why the draft can't be added as it is now, for the assistant to fix.
    package var problem: String?
    /// Every skill, on or off. The built-ins until the person's load.
    package var skills = Skill.builtIn
    /// The ID of the skill picked for the next message.
    package var skill: String?
    /// The recordings that go with the next message.
    package var recordings: [ChatAttachment] = []
    /// The skills Lifr may pick for the next message, sent without one.
    package var pick: SkillPick?

    package init(workspace: Workspace) {
        self.workspace = workspace
    }
}

/// The skills Lifr may pick for a message sent without one, and the one it
/// keeps when it picks none.
package struct SkillPick: Equatable, Sendable {
    var candidates: [Skill]
    var kept: Skill?
}

/// Why the assistant didn't reply.
package enum ReplyFailure: Error, CustomStringConvertible {
    case assistant(AssistantError)
    /// The assistant ended without a word.
    case noReply
    /// The model finished, maybe after thinking, without writing an answer.
    case emptyReply
    case failed(any Error)

    package var description: String {
        switch self {
        case .assistant(let error): error.description
        case .noReply: String(localized: "The assistant stopped without replying. Send the message again.")
        case .emptyReply:
            String(
                localized: """
                    The model finished without writing an answer, which happens when it thinks for too long. \
                    Retry, or set Thinking lower or off in the model menu, or pick another model.
                    """)
        case .failed(let error): String(describing: error)
        }
    }
}

/// Replies with Lifr's assistant, which looks things up in the records,
/// drafts plans and proposes changes. A reply keeps going while Lifr is in
/// the background.
package struct LifrReplier: ChatReplier {
    package typealias Message = ChatMessage
    package typealias Context = AssistantContext
    package typealias Failure = ReplyFailure
    package typealias Host = LifrHost

    /// How long the assistant may write nothing before Lifr stops waiting:
    /// it gives up between one and two of these after the last progress.
    package static let stallLimit = Duration.seconds(60)

    @Dependency(\.assistant) private var assistant
    @Dependency(\.attachments) private var attachments
    @Dependency(\.guidance) private var guidance
    @Dependency(\.backgroundTime) private var backgroundTime
    @Dependency(\.continuousClock) private var clock
    @Dependency(\.date.now) private var now
    @Dependency(\.timeZone) private var timeZone

    package init() {}

    package func ask(_ request: ReplyRequest<Self>) -> ReplyAsk<ReplyChanges, ReplyFailure> {
        let chat = request.chat
        let host = request.host
        let asked = chat.messages.last
        var sent = AssistantRequest(
            messages: Array(chat.unsummarized), draft: chat.draft, problem: host.problem,
            workspace: host.workspace, now: now, timeZone: timeZone,
            // The second model's answer only reads: keeping it can't
            // change anything twice.
            writes: request.isRival ? nil : AssistantWrites(conversation: chat.id),
            skill: asked?.skill.flatMap { id in host.skills.first { $0.id == id } },
            attachments: (asked?.attachments ?? []).compactMap { attachment in
                attachment.kind.input.flatMap { input in
                    attachments.file(attachment, chat.id).map { ModelAttachment(input, file: $0) }
                }
            })
        sent.summary = chat.summary?.summary.text
        sent.model = request.choice
        let prompt = ReplyPrompt(
            instructions: "", prompt: asked?.text ?? "", photos: asked?.photos.count ?? 0)
        let pick = request.isRival || asked?.skill != nil ? nil : host.pick
        let write = Writer(
            assistant: assistant, guidance: guidance, backgroundTime: backgroundTime, clock: clock)
        return ReplyAsk(prompt: prompt) { progress throws(ReplyFailure) in
            do {
                return try await write(sent, picking: pick, progress: progress)
            } catch let error as ReplyFailure {
                throw error
            } catch let error as AssistantError {
                throw .assistant(error)
            } catch {
                throw .failed(error)
            }
        }
    }

    package func sending(_ message: ChatMessage, replacing old: ChatMessage?, host: LifrHost) -> ChatMessage {
        var message = message
        message.skill = host.skill
        // The new message keeps the recordings sent with the old one.
        let kept = old?.attachments.filter { $0.kind == .audio } ?? []
        message.attachments += kept + host.recordings
        return message
    }

    /// Sending again can't help a request that's too long or declined, and
    /// could make saved changes twice.
    package func canRetry(after failure: ReplyFailure) -> Bool {
        guard case .assistant(let error) = failure else { return true }
        return error == .unreadable || error == .timedOut || error.isFailure
    }
}

/// Writes one reply: loads the person's guidance, picks a skill when Lifr
/// may, then sends the reply as it's written.
private struct Writer: Sendable {
    let assistant: AssistantClient
    let guidance: GuidanceClient
    let backgroundTime: BackgroundTime
    let clock: any Clock<Duration>

    func callAsFunction(
        _ request: AssistantRequest, picking pick: SkillPick?,
        progress: @escaping @Sendable (ReplyProgress) -> Void
    ) async throws -> ChatReply<ReplyChanges> {
        var request = request
        // The default when the person's can't be read; the Instructions
        // screen says why.
        request.guidance = (try? await guidance.load().text) ?? Guidance.defaultText
        var picked: String?
        if let pick {
            // A pick that fails leaves the message without a skill, to
            // answer as it is.
            let id = await pickedSkill(pick, for: request) ?? pick.kept?.id
            let skills = pick.candidates + (pick.kept.map { [$0] } ?? [])
            if let id, let skill = skills.first(where: { $0.id == id }) {
                request.skill = skill
                picked = id
            }
        }
        let last = LockIsolated<AssistantReply?>(nil)
        try await backgroundTime.run("Answering") { [request] progressed in
            let returns = backgroundTime.returns
            try await Self.watch(request, assistant: assistant, clock: clock, returns: returns) { reply in
                progressed()
                last.setValue(reply)
                if let thinking = reply.thinking?.shown { progress(.thinking(thinking)) }
                if !reply.isComplete { progress(.text(reply.text)) }
            }
        }
        guard let reply = last.value else { throw ReplyFailure.noReply }
        // Nothing to show or apply would leave an empty bubble.
        if reply.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, reply.proposal == nil,
            reply.plan == nil
        {
            throw ReplyFailure.emptyReply
        }
        return ChatReply(
            text: reply.text, thinking: reply.thinking?.shown, context: reply.context,
            speed: reply.speed.map {
                ThreadSpeed(
                    read: $0.read, written: $0.written, reading: $0.reading, writing: $0.writing,
                    estimated: $0.estimated)
            },
            extra: ReplyChanges(proposal: reply.proposal, plan: reply.plan, leftOut: reply.leftOut, skill: picked),
            prompt: reply.sent.map {
                ReplyPrompt(
                    instructions: $0.instructions, prompt: $0.prompt, photos: $0.photos, context: reply.context)
            })
    }

    /// The skill the assistant picks from `pick` for `request`, or nil
    /// when it can't, or takes longer than `stallLimit`: a pick is short,
    /// and the reply waits for it.
    private func pickedSkill(_ pick: SkillPick, for request: AssistantRequest) async -> String? {
        await withTaskGroup(of: String?.self) { [assistant, clock] group in
            group.addTask {
                try? await assistant.pickSkill(request.messages, pick.candidates, pick.kept?.id, request.model)
            }
            group.addTask {
                try? await clock.sleep(for: LifrReplier.stallLimit)
                return nil
            }
            let first = await group.next() ?? nil
            group.cancelAll()
            return first
        }
    }

    /// Sends each reply to `request`, and stops with `.timedOut` when the
    /// assistant writes nothing for `stallLimit`. Time in the background
    /// doesn't count: a model there may be waiting for Lifr to be back on
    /// screen, so each return starts the wait again.
    private static func watch(
        _ request: AssistantRequest, assistant: AssistantClient, clock: any Clock<Duration>,
        returns: @escaping @Sendable () -> Int, _ each: @escaping @Sendable (AssistantReply) async -> Void
    ) async throws {
        let progress = LockIsolated(0)
        try await withThrowingTaskGroup(of: Void.self) { [request] group in
            group.addTask {
                for try await reply in assistant.reply(request) {
                    progress.withValue { $0 += 1 }
                    await each(reply)
                }
            }
            group.addTask {
                var seen = -1
                var back = -1
                while progress.value != seen || returns() != back {
                    seen = progress.value
                    back = returns()
                    try await clock.sleep(for: LifrReplier.stallLimit)
                }
                throw AssistantError.timedOut
            }
            try await group.next()
            group.cancelAll()
        }
    }
}

extension ModelThinking {
    /// As the chat keeps it, unless the model thought nothing worth
    /// showing.
    fileprivate var shown: ThreadThinking? {
        text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            ? nil : ThreadThinking(text: text, duration: duration)
    }
}

extension ChatPhotosClient {
    /// Kept with the conversation's other attachments, so they sync, fork
    /// and are deleted with it.
    static func attachments(_ attachments: AttachmentsClient) -> Self {
        ChatPhotosClient(
            save: { data, chat throws(ChatPhotoError) in
                do {
                    return ThreadPhoto(id: try await attachments.save(data, .image, nil, chat).id)
                } catch AttachmentError.unreadablePhoto {
                    throw .unreadable
                } catch {
                    throw .write(underlying: error)
                }
            },
            file: { photo, chat in attachments.file(ChatAttachment(id: photo.id, kind: .image), chat) },
            remove: { photo, chat in await attachments.remove(ChatAttachment(id: photo.id, kind: .image), chat) },
            removeAll: { chat in await attachments.removeAll(chat) })
    }
}
