import Foundation
import ImageIO
import LifrAssistantClient
import LifrModelsClient
import UniformTypeIdentifiers

extension AttachmentsClient {
    /// Kept in `Assistant/Attachments/<conversation>` in Application
    /// Support, readable only by Lifr, encrypted while the device is locked
    /// after a restart, and left out of backups: they stay on this device.
    package static func onDisk() -> Self {
        let files = { () throws(AttachmentError) -> AttachmentFiles in
            do {
                let support = try FileManager.default.url(
                    for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
                return AttachmentFiles(
                    root: support.appending(path: "Assistant", directoryHint: .isDirectory)
                        .appending(path: "Attachments", directoryHint: .isDirectory))
            } catch {
                throw .storageFailed(error.localizedDescription)
            }
        }
        return AttachmentsClient(
            save: { data, kind, seconds, conversation in
                try files().save(data, kind: kind, seconds: seconds, conversation: conversation)
            },
            file: { attachment, conversation in try? files().file(attachment, conversation: conversation) },
            remove: { attachment, conversation in try? files().remove(attachment, conversation: conversation) },
            removeAll: { conversation in try? files().removeAll(conversation) })
    }
}

/// The photos and recordings of every conversation, under `root`.
struct AttachmentFiles: Sendable {
    /// The longest side of a kept photo, in pixels: Gemma reads photos at
    /// up to this size, so anything larger only takes room.
    static let longestSide = 1024

    let root: URL

    /// Keeps a photo as JPEG, at most `longestSide` pixels and without its
    /// metadata, or a recording as the WAV it is.
    func save(
        _ data: Data, kind: ModelInput, seconds: Int?, conversation: UUID, id: UUID = UUID()
    ) throws(AttachmentError) -> ChatAttachment {
        let attachment = ChatAttachment(id: id, kind: kind, seconds: kind == .audio ? seconds : nil)
        let contents: Data
        switch kind {
        case .image:
            contents = try Self.jpeg(data)
        case .audio:
            guard Self.isWAV(data) else { throw .recordingFailed("it isn't a WAV file") }
            contents = data
        }
        let folder = folder(conversation)
        let url = url(attachment, conversation: conversation)
        do {
            try FileManager.default.createDirectory(
                at: folder, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
            for directory in [root, folder] {
                try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: directory.path)
                var values = URLResourceValues()
                values.isExcludedFromBackup = true
                var excluded = directory
                try excluded.setResourceValues(values)
            }
            try contents.write(to: url, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: url.path)
        } catch {
            throw .storageFailed(error.localizedDescription)
        }
        return attachment
    }

    /// The attachment's file, or nil when it isn't on this device.
    func file(_ attachment: ChatAttachment, conversation: UUID) -> URL? {
        let url = url(attachment, conversation: conversation)
        return FileManager.default.fileExists(atPath: url.path) ? url : nil
    }

    func remove(_ attachment: ChatAttachment, conversation: UUID) throws {
        try removeIfPresent(url(attachment, conversation: conversation))
    }

    func removeAll(_ conversation: UUID) throws {
        try removeIfPresent(folder(conversation))
    }

    private func removeIfPresent(_ url: URL) throws {
        do {
            try FileManager.default.removeItem(at: url)
        } catch CocoaError.fileNoSuchFile {
            return
        }
    }

    private func folder(_ conversation: UUID) -> URL {
        root.appending(path: conversation.uuidString, directoryHint: .isDirectory)
    }

    private func url(_ attachment: ChatAttachment, conversation: UUID) -> URL {
        folder(conversation).appending(
            path: attachment.id.uuidString + (attachment.kind == .image ? ".jpg" : ".wav"))
    }

    /// `data` as a JPEG no larger than `longestSide`, turned upright, with
    /// none of the original's metadata, such as where it was taken.
    static func jpeg(_ data: Data) throws(AttachmentError) -> Data {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil), CGImageSourceGetCount(source) > 0,
            let image = CGImageSourceCreateThumbnailAtIndex(
                source, 0,
                [
                    kCGImageSourceCreateThumbnailFromImageAlways: true,
                    kCGImageSourceCreateThumbnailWithTransform: true,
                    kCGImageSourceThumbnailMaxPixelSize: longestSide,
                ] as CFDictionary)
        else { throw .unreadablePhoto }
        let output = NSMutableData()
        guard
            let destination = CGImageDestinationCreateWithData(
                output as CFMutableData, UTType.jpeg.identifier as CFString, 1, nil)
        else { throw .storageFailed("JPEG isn't available") }
        CGImageDestinationAddImage(
            destination, image, [kCGImageDestinationLossyCompressionQuality: 0.8] as CFDictionary)
        guard CGImageDestinationFinalize(destination) else { throw .unreadablePhoto }
        return output as Data
    }

    /// Whether `data` starts like a WAV file.
    static func isWAV(_ data: Data) -> Bool {
        data.count > 12 && data.prefix(4) == Data("RIFF".utf8) && data.dropFirst(8).prefix(4) == Data("WAVE".utf8)
    }
}
