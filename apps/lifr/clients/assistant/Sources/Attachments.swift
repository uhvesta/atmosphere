import ComposableArchitecture
import Foundation
import LifrModelsClient

/// A photo, recording or file the person sent with a message. Only this
/// record goes with the conversation when it syncs: a photo's or
/// recording's file stays on the device it was sent from. A file's text is
/// in the record itself, so it goes too.
package struct ChatAttachment: Identifiable, Hashable, Codable, Sendable {
    package var id: UUID
    /// A photo, a recording or a file's text.
    package var kind: Kind
    /// How long a recording is, in whole seconds.
    package var seconds: Int?
    /// A file's name, as the person sees it.
    package var name: String?
    /// A file's text, which goes to the model as context.
    package var text: String?

    /// What's attached.
    package enum Kind: String, Hashable, Codable, Sendable {
        case image
        case audio
        /// Text from a file, such as a note, Markdown or a PDF.
        case text

        package init(_ input: ModelInput) {
            switch input {
            case .image: self = .image
            case .audio: self = .audio
            }
        }

        /// What the model takes it as; nil for text, which goes in the
        /// prompt.
        package var input: ModelInput? {
            switch self {
            case .image: .image
            case .audio: .audio
            case .text: nil
            }
        }
    }

    package init(id: UUID, kind: Kind, seconds: Int? = nil) {
        self.id = id
        self.kind = kind
        self.seconds = seconds
    }

    /// A file's text, named as the file is.
    package init(id: UUID, name: String, text: String) {
        self.id = id
        kind = .text
        self.name = name
        self.text = text
    }

    /// The most text kept from a file, in characters. The prompt keeps as
    /// much of it as fits the model.
    package static let textLimit = 8_000

    /// A cautious count of the tokens the model reads for it: Gemma reads a
    /// photo as up to 280 tokens, and a recording as about 25 a second. A
    /// file's text is counted with the prompt it's in.
    package var estimatedTokens: Int {
        switch kind {
        case .image: 280
        case .audio: 25 * max(1, seconds ?? RecorderClient.limit)
        case .text: 0
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
    /// Recording was stopped before the microphone started.
    case nothingRecorded
    /// Saving or reading the file on this device failed.
    case storageFailed(String)
    /// A file couldn't be attached, with why and what to do, for the person.
    case unreadableFile(String)

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
        case .nothingRecorded:
            String(
                localized: """
                    The recording stopped before the microphone started, so there's nothing to attach. Tap the \
                    microphone again to record.
                    """)
        case .unreadableFile(let why):
            why
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
    /// Keeps a copy of each attachment's file from one conversation for
    /// another, such as a fork, leaving out those not on this device.
    package var copy: @Sendable (_ attachments: [ChatAttachment], _ from: UUID, _ to: UUID) async -> Void
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
                return ChatAttachment(id: id, kind: ChatAttachment.Kind(kind), seconds: seconds)
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
            },
            copy: { attachments, from, to in
                stored.withValue { stored in
                    let kept = stored[from] ?? []
                    for attachment in attachments where kept.contains(attachment.id) {
                        stored[to, default: []].insert(attachment.id)
                    }
                }
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

/// Its description is written for the person, so alerts show it.
extension AttachmentError: LocalizedError {
    package var errorDescription: String? { description }
}
