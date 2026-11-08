import BlobKit
import ChatKit
import ComposableArchitecture
import Foundation
import InferenceKit

/// A chat as it's kept: its messages and what it runs on.
package struct SavedChat: Hashable, Codable, Sendable, Identifiable {
    package var conversation: ChatThread
    package var model: ModelChoice

    package init(conversation: ChatThread, model: ModelChoice = .defaults) {
        self.conversation = conversation
        self.model = model
    }

    package var id: UUID { conversation.id }
}

/// Why a chat couldn't be kept, read or deleted.
package enum ChatShelfError: Error, CustomStringConvertible {
    case write(chat: String, underlying: any Error)
    case read(file: String, underlying: any Error)
    case delete(chat: String, underlying: any Error)

    package var description: String {
        switch self {
        case .write(let chat, let error):
            "“\(chat)” couldn't be saved: \(error.localizedDescription). Free some space on the device, then "
                + "send a message to save it again."
        case .read(let file, let error):
            "The chat in \(file) couldn't be opened: \(error). It stays on the device, so a newer version of the "
                + "app may open it."
        case .delete(let chat, let error):
            "“\(chat)” couldn't be deleted: \(error.localizedDescription). Try again."
        }
    }
}

/// The chats on the device, one file each, in a folder of their own.
package struct ChatShelf: Sendable {
    package let folder: URL

    package init(folder: URL) { self.folder = folder }

    /// The chats in the app's Application Support folder.
    package static var standard: ChatShelf {
        let support =
            FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
            ?? FileManager.default.temporaryDirectory
        return ChatShelf(folder: support.appendingPathComponent("Chats", isDirectory: true))
    }

    package func save(_ chat: SavedChat) throws(ChatShelfError) {
        do {
            try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
            try Blob.encode(chat).write(to: file(for: chat.id), options: Self.writing)
        } catch {
            throw .write(chat: chat.conversation.title, underlying: error)
        }
    }

    /// Every chat it can read, and the files it couldn't.
    package func chats() -> (chats: [SavedChat], unreadable: [ChatShelfError]) {
        let files =
            (try? FileManager.default.contentsOfDirectory(at: folder, includingPropertiesForKeys: nil))?
            .filter { $0.pathExtension == "chat" } ?? []
        var chats: [SavedChat] = []
        var unreadable: [ChatShelfError] = []
        for file in files {
            guard let data = try? Data(contentsOf: file) else { continue }
            do {
                chats.append(try Blob.decode(SavedChat.self, from: data))
            } catch {
                unreadable.append(.read(file: file.lastPathComponent, underlying: error))
            }
        }
        return (chats, unreadable)
    }

    package func delete(_ chat: SavedChat) throws(ChatShelfError) {
        let url = file(for: chat.id)
        guard FileManager.default.fileExists(atPath: url.path) else { return }
        do {
            try FileManager.default.removeItem(at: url)
        } catch {
            throw .delete(chat: chat.conversation.title, underlying: error)
        }
    }

    /// Encrypted while the device is locked.
    #if os(iOS)
        private static let writing: Data.WritingOptions = [.atomic, .completeFileProtection]
    #else
        private static let writing: Data.WritingOptions = [.atomic]
    #endif

    private func file(for id: UUID) -> URL { folder.appendingPathComponent("\(id.uuidString).chat") }
}

/// Where the chats are kept: on the device, or in memory for tests and
/// flows.
package struct ChatShelfClient: Sendable {
    package var chats: @Sendable () -> (chats: [SavedChat], unreadable: [ChatShelfError])
    package var save: @Sendable (SavedChat) throws(ChatShelfError) -> Void
    package var delete: @Sendable (SavedChat) throws(ChatShelfError) -> Void

    package init(
        chats: @escaping @Sendable () -> (chats: [SavedChat], unreadable: [ChatShelfError]),
        save: @escaping @Sendable (SavedChat) throws(ChatShelfError) -> Void,
        delete: @escaping @Sendable (SavedChat) throws(ChatShelfError) -> Void
    ) {
        self.chats = chats
        self.save = save
        self.delete = delete
    }

    package static func live(_ shelf: ChatShelf) -> Self {
        ChatShelfClient(
            chats: { shelf.chats() }, save: { chat throws(ChatShelfError) in try shelf.save(chat) },
            delete: { chat throws(ChatShelfError) in try shelf.delete(chat) })
    }

    /// Chats in memory, starting with `chats`.
    package static func inMemory(_ chats: [SavedChat] = []) -> Self {
        let kept = LockIsolated(chats)
        return ChatShelfClient(
            chats: { (kept.value, []) },
            save: { chat in
                kept.withValue { chats in
                    chats.removeAll { $0.id == chat.id }
                    chats.append(chat)
                }
            },
            delete: { chat in kept.withValue { $0.removeAll { $0.id == chat.id } } })
    }

    /// Chats that can't be saved, for showing what that says.
    package static func full(_ chats: [SavedChat] = []) -> Self {
        ChatShelfClient(
            chats: { (chats, []) },
            save: { chat throws(ChatShelfError) in
                throw .write(chat: chat.conversation.title, underlying: CocoaError(.fileWriteOutOfSpace))
            },
            delete: { _ in })
    }
}

extension ChatShelfClient: DependencyKey {
    package static let liveValue = ChatShelfClient.live(.standard)
    package static var testValue: ChatShelfClient { .inMemory() }
}

extension DependencyValues {
    package var chatShelf: ChatShelfClient {
        get { self[ChatShelfClient.self] }
        set { self[ChatShelfClient.self] = newValue }
    }
}
