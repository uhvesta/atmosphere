import ComposableArchitecture
import Foundation
import LifrModelsClient

/// A photo or recording the person sent with a message. Only this record
/// goes with the conversation when it syncs: the file stays on the device
/// it was sent from.
package struct ChatAttachment: Identifiable, Hashable, Codable, Sendable {
    package var id: UUID
    /// A photo or a recording.
    package var kind: ModelInput
    /// How long a recording is, in whole seconds.
    package var seconds: Int?

    package init(id: UUID, kind: ModelInput, seconds: Int? = nil) {
        self.id = id
        self.kind = kind
        self.seconds = seconds
    }

    /// A cautious count of the tokens the model reads for it: Gemma reads a
    /// photo as up to 280 tokens, and a recording as about 25 a second.
    package var estimatedTokens: Int {
        switch kind {
        case .image: 280
        case .audio: 25 * max(1, seconds ?? RecorderClient.limit)
        }
    }
}

/// Why a photo or recording couldn't be attached.
package enum AttachmentError: Error, Hashable, Sendable, CustomStringConvertible {
    /// The photo isn't in a format Lifr can read.
    case unreadablePhoto
    /// Lifr isn't allowed to use the microphone.
    case microphoneDenied
    /// The recording couldn't start or finish.
    case recordingFailed(String)
    /// Saving or reading the file on this device failed.
    case storageFailed(String)

    package var description: String {
        switch self {
        case .unreadablePhoto:
            String(
                localized: "Lifr couldn't read that photo. Pick another, or take a screenshot of it and attach that.")
        case .microphoneDenied:
            #if os(macOS)
                String(
                    localized: """
                        Lifr isn't allowed to use the microphone. Turn on Lifr in System Settings › Privacy & \
                        Security › Microphone, then record again.
                        """)
            #else
                String(
                    localized: """
                        Lifr isn't allowed to use the microphone. Turn on Microphone for Lifr in Settings › Lifr, \
                        then record again.
                        """)
            #endif
        case .recordingFailed(let reason):
            String(
                localized: """
                    The recording stopped (\(reason)). Record again; if another app is using the microphone, \
                    close it first.
                    """)
        case .storageFailed(let reason):
            String(
                localized: """
                    Couldn't keep the attachment on this device (\(reason)). Check that this device has free \
                    storage, then attach it again.
                    """)
        }
    }
}

/// Keeps photos and recordings on this device, by conversation. Photos are
/// kept as JPEG without their location or other metadata.
@DependencyClient
package struct AttachmentsClient: Sendable {
    /// Keeps `data`, a photo in any format the system reads or a WAV
    /// recording, for `conversation`. Throws `AttachmentError`.
    package var save:
        @Sendable (_ data: Data, _ kind: ModelInput, _ seconds: Int?, _ conversation: UUID) async throws ->
            ChatAttachment
    /// The attachment's file, or nil when it isn't on this device, such as
    /// for a conversation synced from another one.
    package var file: @Sendable (_ attachment: ChatAttachment, _ conversation: UUID) -> URL? = { _, _ in nil }
    /// Deletes one attachment's file.
    package var remove: @Sendable (_ attachment: ChatAttachment, _ conversation: UUID) async -> Void
    /// Deletes every file of `conversation`.
    package var removeAll: @Sendable (_ conversation: UUID) async -> Void
}

extension AttachmentsClient {
    /// Kept in memory, for tests and previews. Files are named but never
    /// written.
    package static func inMemory() -> Self {
        let stored = LockIsolated<[UUID: Set<UUID>]>([:])
        let ids = LockIsolated(0)
        return AttachmentsClient(
            save: { data, kind, seconds, conversation in
                guard !data.isEmpty else {
                    throw kind == .image ? AttachmentError.unreadablePhoto : AttachmentError.recordingFailed("empty")
                }
                let id = ids.withValue { count in
                    count += 1
                    return UUID(uuidString: String(format: "A77AC400-0000-0000-0000-%012d", count)) ?? UUID()
                }
                stored.withValue { _ = $0[conversation, default: []].insert(id) }
                return ChatAttachment(id: id, kind: kind, seconds: seconds)
            },
            file: { attachment, conversation in
                guard stored.value[conversation]?.contains(attachment.id) == true else { return nil }
                return URL(filePath: "/attachments/\(conversation.uuidString)/\(attachment.id.uuidString)")
            },
            remove: { attachment, conversation in
                stored.withValue { _ = $0[conversation]?.remove(attachment.id) }
            },
            removeAll: { conversation in
                stored.withValue { $0[conversation] = nil }
            })
    }
}

extension AttachmentsClient: TestDependencyKey {
    package static let testValue = AttachmentsClient()
    package static let previewValue = AttachmentsClient.inMemory()
}

extension DependencyValues {
    package var attachments: AttachmentsClient {
        get { self[AttachmentsClient.self] }
        set { self[AttachmentsClient.self] = newValue }
    }
}

/// Records from the microphone, for a message to the assistant.
@DependencyClient
package struct RecorderClient: Sendable {
    /// The longest recording, in seconds: what Gemma listens to at once.
    package static let limit = 30

    /// Asks for the microphone if Lifr hasn't yet, then records. Throws
    /// `AttachmentError`.
    package var start: @Sendable () async throws -> Void
    /// Stops and returns the recording, as 16 kHz mono WAV, with its length
    /// in seconds. Throws `AttachmentError`.
    package var stop: @Sendable () async throws -> Recording
    /// Stops and throws the recording away.
    package var cancel: @Sendable () async -> Void

    package struct Recording: Hashable, Sendable {
        package var data: Data
        package var seconds: Int

        package init(data: Data, seconds: Int) {
            self.data = data
            self.seconds = seconds
        }
    }
}

extension RecorderClient {
    /// Records nothing, and returns `recording` when stopped: for tests and
    /// previews.
    package static func scripted(_ recording: Recording = Recording(data: Data([1]), seconds: 4)) -> Self {
        RecorderClient(start: {}, stop: { recording }, cancel: {})
    }
}

extension RecorderClient: TestDependencyKey {
    package static let testValue = RecorderClient()
    package static let previewValue = RecorderClient.scripted()
}

extension DependencyValues {
    package var recorder: RecorderClient {
        get { self[RecorderClient.self] }
        set { self[RecorderClient.self] = newValue }
    }
}
