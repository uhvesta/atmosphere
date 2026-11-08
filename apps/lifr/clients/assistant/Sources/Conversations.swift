import ChatKit
import ComposableArchitecture
import Foundation
import LifrCore
import LifrModelsClient

/// One conversation with the assistant: ChatKit's, with Lifr's draft and
/// Undo kept beside its messages.
package typealias Conversation = ChatKit.Conversation<ChatMessage, AssistantContext>
package typealias ChatSummary = ChatKit.ChatSummary
package typealias ConversationQuery = ChatKit.ConversationQuery

/// What Lifr keeps with a conversation besides its messages.
package struct AssistantContext: ConversationContext {
    /// The plan on the draft card.
    package var draft: Plan?
    /// The journal entries each assistant message saved, for its Undo.
    package var changes: [UUID: [UUID]] = [:]
    /// The journal entries of plans added from the draft card.
    package var added: [UUID] = []
    /// The model it runs on, how long that thinks and its window, in place
    /// of the defaults in Settings.
    package var model = ModelChoice.defaults
    /// Whether Lifr picks a skill for a message sent without one, in place
    /// of Settings. nil follows Settings.
    package var picksSkills: Bool?

    package init() {}

    private enum CodingKeys: String, CodingKey { case draft, changes, added, model, picksSkills }

    /// Conversations saved before drafts, Undo, a model of their own or
    /// their own skill picking read with none.
    package init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        draft = try container.decodeIfPresent(Plan.self, forKey: .draft)
        changes = try container.decodeIfPresent([UUID: [UUID]].self, forKey: .changes) ?? [:]
        added = try container.decodeIfPresent([UUID].self, forKey: .added) ?? []
        model = try container.decodeIfPresent(ModelChoice.self, forKey: .model) ?? .defaults
        picksSkills = try container.decodeIfPresent(Bool.self, forKey: .picksSkills)
    }
}

extension ChatKit.Conversation where Message == ChatMessage, Context == AssistantContext {
    /// Every point to go to, on every branch, oldest first, with the task
    /// changes its replies saved that can still be undone.
    package var checkpoints: [Checkpoint] { checkpoints(changes: { context.changes[$0]?.count ?? 0 }) }

    /// The replies shown now that going to `id` would hide, and that saved
    /// changes still to undo.
    package func changedReplies(hiddenGoingTo id: UUID) -> [UUID] {
        changedReplies(hiddenGoingTo: id, changed: { context.changes[$0]?.isEmpty == false })
    }

    /// The replies shown now that writing the message `id` again would
    /// hide, and that saved changes still to undo.
    package func changedReplies(hiddenGoingBefore id: UUID) -> [UUID] {
        changedReplies(hiddenGoingBefore: id, changed: { context.changes[$0]?.isEmpty == false })
    }

    /// A new conversation of the messages up to `id`, on this one's branch
    /// to it, that doesn't share its Undo or draft: the tasks those
    /// changed belong to this one. It runs on the same model, and picks
    /// skills the same way.
    package func fork(at id: UUID, as forkID: UUID, now: Date) -> Conversation? {
        var fork = fork(at: id, as: forkID, now: now, name: String(localized: "\(title) (fork)"))
        fork?.context.model = context.model
        fork?.context.picksSkills = context.picksSkills
        return fork
    }
}

extension SharedKey where Self == AppStorageKey<Bool>.Default {
    /// Whether the assistant summarizes older messages by itself once they
    /// no longer fit what the model reads. Off unless the person turns it
    /// on: otherwise the chat offers Compact.
    package static var compactsConversations: Self {
        Self[.appStorage("lifrCompactsConversations"), default: false]
    }

    /// Whether the assistant names each new conversation after its first
    /// reply. Off unless the person turns it on.
    package static var namesConversations: Self {
        Self[.appStorage("lifrNamesConversations"), default: false]
    }

    /// Whether Lifr picks a skill for a message sent without one, among
    /// the skills that are on and may be picked automatically. Without a
    /// skill the assistant just talks.
    package static var picksSkills: Self {
        Self[.appStorage("lifrPicksSkills"), default: true]
    }
}

/// Keeps each conversation on this device, or in iCloud when it's synced.
/// Deleting a conversation never deletes the tasks it changed.
@DependencyClient
package struct ConversationsClient: Sendable {
    /// Every kept conversation, in no particular order.
    package var load: @Sendable () async throws -> [Conversation]
    /// Adds the conversation, or replaces the one with its ID, where its
    /// `isSynced` says: a synced one moves to iCloud, and one that stops
    /// syncing leaves iCloud and stays here.
    package var save: @Sendable (_ conversation: Conversation) async throws -> Void
    /// Removes conversations here and, if they were synced, from iCloud.
    package var delete: @Sendable (_ ids: [UUID]) async throws -> Void
}

extension ConversationsClient {
    /// Keeps conversations in memory, for previews and tests.
    package static func inMemory(_ conversations: [Conversation] = []) -> Self {
        let stored = LockIsolated(Dictionary(uniqueKeysWithValues: conversations.map { ($0.id, $0) }))
        return ConversationsClient(
            load: { Array(stored.value.values) },
            save: { conversation in stored.withValue { $0[conversation.id] = conversation } },
            delete: { ids in stored.withValue { for id in ids { $0[id] = nil } } }
        )
    }
}

extension ConversationsClient: TestDependencyKey {
    package static let testValue = ConversationsClient()
    package static let previewValue = ConversationsClient.inMemory()
}

extension DependencyValues {
    package var conversations: ConversationsClient {
        get { self[ConversationsClient.self] }
        set { self[ConversationsClient.self] = newValue }
    }
}
