import CaptureKit
import ChatKit
import ComposableArchitecture
import Foundation
import ImageIO
import UniformTypeIdentifiers

/// The person's photo library and camera, for + on the iPhone.
public struct PhotoLibraryClient: Sendable {
    /// Whether the app may show the person's photos, without asking.
    public var access: @Sendable () -> PhotosAccess
    /// Asks the person, the first time, whether the app may show their
    /// photos.
    public var requestAccess: @Sendable () async -> PhotosAccess
    /// The newest photos, as thumbnails, newest first.
    public var recents: @Sendable () async -> [RecentPhoto]
    /// A recent photo itself.
    public var photo: @Sendable (_ id: RecentPhoto.ID) async throws(PhotoLibraryError) -> Data
    /// Whether there's a camera to take a photo with.
    public var hasCamera: @Sendable () async -> Bool

    public init(
        access: @escaping @Sendable () -> PhotosAccess,
        requestAccess: @escaping @Sendable () async -> PhotosAccess,
        recents: @escaping @Sendable () async -> [RecentPhoto],
        photo: @escaping @Sendable (_ id: RecentPhoto.ID) async throws(PhotoLibraryError) -> Data,
        hasCamera: @escaping @Sendable () async -> Bool
    ) {
        self.access = access
        self.requestAccess = requestAccess
        self.recents = recents
        self.photo = photo
        self.hasCamera = hasCamera
    }

    /// The person's photo library, through PhotoKit, and the device's
    /// camera.
    public static func live() -> Self {
        PhotoLibraryClient(
            access: { PhotoLibrary.access },
            requestAccess: { await PhotoLibrary.requestAccess() },
            recents: { await PhotoLibrary.recent(limit: 24, side: 240) },
            photo: { id throws(PhotoLibraryError) in try await PhotoLibrary.data(of: id) },
            hasCamera: { await Camera.isAvailable })
    }

    /// A library of `recents`, each photo its thumbnail, that the app may
    /// show: for tests and flows.
    public static func scripted(_ recents: [RecentPhoto], camera: Bool = false) -> Self {
        PhotoLibraryClient(
            access: { .allowed },
            requestAccess: { .allowed },
            recents: { recents },
            photo: { id throws(PhotoLibraryError) in
                guard let photo = recents.first(where: { $0.id == id }) else { throw .missing }
                return photo.thumbnail
            },
            hasCamera: { camera })
    }

    /// A library the app may not show, as when the person said no.
    public static let denied = PhotoLibraryClient(
        access: { .denied }, requestAccess: { .denied }, recents: { [] },
        photo: { _ throws(PhotoLibraryError) in throw .missing }, hasCamera: { false })
}

extension PhotoLibraryClient: DependencyKey {
    public static let liveValue = PhotoLibraryClient.live()
    public static var testValue: PhotoLibraryClient { .denied }
}

/// Why a photo couldn't be kept for a chat.
public enum ChatPhotoError: Error, CustomStringConvertible {
    /// The data isn't a photo the system can read.
    case unreadable
    /// Writing it on this device failed.
    case write(underlying: any Error)

    public var description: String {
        switch self {
        case .unreadable:
            String(localized: "Couldn't read that photo. Pick another, or take a screenshot of it and add that.")
        case .write(let error):
            String(
                localized: """
                    Couldn't keep the photo on this device: \(error.localizedDescription) Free some space, then \
                    add it again.
                    """)
        }
    }
}

/// Keeps the photos sent in chats, on this device, by chat.
public struct ChatPhotosClient: Sendable {
    /// Keeps `data`, a photo in any format the system reads, for `chat`.
    public var save: @Sendable (_ data: Data, _ chat: UUID) async throws(ChatPhotoError) -> ThreadPhoto
    /// The photo's file, or nil when it isn't on this device.
    public var file: @Sendable (_ photo: ThreadPhoto, _ chat: UUID) -> URL?
    /// Deletes one photo, as when it's taken off before it's sent.
    public var remove: @Sendable (_ photo: ThreadPhoto, _ chat: UUID) async -> Void
    /// Deletes every photo of `chat`, as when the chat is deleted.
    public var removeAll: @Sendable (_ chat: UUID) async -> Void

    public init(
        save: @escaping @Sendable (_ data: Data, _ chat: UUID) async throws(ChatPhotoError) -> ThreadPhoto,
        file: @escaping @Sendable (_ photo: ThreadPhoto, _ chat: UUID) -> URL?,
        remove: @escaping @Sendable (_ photo: ThreadPhoto, _ chat: UUID) async -> Void,
        removeAll: @escaping @Sendable (_ chat: UUID) async -> Void
    ) {
        self.save = save
        self.file = file
        self.remove = remove
        self.removeAll = removeAll
    }

    /// Kept in `folder`, one folder a chat, as JPEG without the photo's
    /// metadata, encrypted while the device is locked after a restart and
    /// left out of backups.
    public static func onDisk(_ folder: URL) -> Self {
        let photos = ChatPhotoFiles(root: folder)
        return ChatPhotosClient(
            save: { data, chat throws(ChatPhotoError) in try photos.save(data, chat: chat) },
            file: { photo, chat in photos.file(photo, chat: chat) },
            remove: { photo, chat in try? photos.remove(photo, chat: chat) },
            removeAll: { chat in try? photos.removeAll(chat) })
    }

    /// In Application Support/Chat Photos.
    public static var standard: Self {
        let support =
            FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
            ?? FileManager.default.temporaryDirectory
        return onDisk(support.appendingPathComponent("Chat Photos", isDirectory: true))
    }

    /// Kept in memory, for tests and flows: each photo's file is its data
    /// in a temporary folder, so it shows as it is.
    public static func inMemory() -> Self {
        onDisk(FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true))
    }
}

extension ChatPhotosClient: DependencyKey {
    public static let liveValue = ChatPhotosClient.standard
    public static var testValue: ChatPhotosClient { .inMemory() }
}

extension DependencyValues {
    public var photoLibrary: PhotoLibraryClient {
        get { self[PhotoLibraryClient.self] }
        set { self[PhotoLibraryClient.self] = newValue }
    }

    public var chatPhotos: ChatPhotosClient {
        get { self[ChatPhotosClient.self] }
        set { self[ChatPhotosClient.self] = newValue }
    }
}

/// The photos of every chat, under `root`.
struct ChatPhotoFiles: Sendable {
    /// The longest side of a kept photo, in pixels: on-device models read
    /// photos at about this size, so anything larger only takes room.
    static let longestSide = 1024

    let root: URL

    func save(_ data: Data, chat: UUID) throws(ChatPhotoError) -> ThreadPhoto {
        let photo = ThreadPhoto(id: UUID())
        let jpeg = try Self.jpeg(data)
        let folder = folder(chat)
        do {
            try FileManager.default.createDirectory(
                at: folder, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
            for directory in [root, folder] {
                var values = URLResourceValues()
                values.isExcludedFromBackup = true
                var excluded = directory
                try excluded.setResourceValues(values)
            }
            try jpeg.write(to: url(photo, chat: chat), options: Self.writing)
        } catch {
            throw .write(underlying: error)
        }
        return photo
    }

    func file(_ photo: ThreadPhoto, chat: UUID) -> URL? {
        let url = url(photo, chat: chat)
        return FileManager.default.fileExists(atPath: url.path) ? url : nil
    }

    func remove(_ photo: ThreadPhoto, chat: UUID) throws {
        try removeIfPresent(url(photo, chat: chat))
    }

    func removeAll(_ chat: UUID) throws {
        try removeIfPresent(folder(chat))
    }

    private func removeIfPresent(_ url: URL) throws {
        do {
            try FileManager.default.removeItem(at: url)
        } catch CocoaError.fileNoSuchFile {
            return
        }
    }

    #if os(iOS)
        private static let writing: Data.WritingOptions = [
            .atomic, .completeFileProtectionUntilFirstUserAuthentication,
        ]
    #else
        private static let writing: Data.WritingOptions = [.atomic]
    #endif

    private func folder(_ chat: UUID) -> URL { root.appendingPathComponent(chat.uuidString, isDirectory: true) }

    private func url(_ photo: ThreadPhoto, chat: UUID) -> URL {
        folder(chat).appendingPathComponent("\(photo.id.uuidString).jpg")
    }

    /// `data` as a JPEG no larger than `longestSide`, turned upright, with
    /// none of the original's metadata, such as where it was taken.
    static func jpeg(_ data: Data) throws(ChatPhotoError) -> Data {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil), CGImageSourceGetCount(source) > 0,
            let image = CGImageSourceCreateThumbnailAtIndex(
                source, 0,
                [
                    kCGImageSourceCreateThumbnailFromImageAlways: true,
                    kCGImageSourceCreateThumbnailWithTransform: true,
                    kCGImageSourceThumbnailMaxPixelSize: longestSide,
                ] as CFDictionary)
        else { throw .unreadable }
        let output = NSMutableData()
        guard
            let destination = CGImageDestinationCreateWithData(
                output as CFMutableData, UTType.jpeg.identifier as CFString, 1, nil)
        else { throw .unreadable }
        CGImageDestinationAddImage(
            destination, image, [kCGImageDestinationLossyCompressionQuality: 0.8] as CFDictionary)
        guard CGImageDestinationFinalize(destination) else { throw .unreadable }
        return output as Data
    }
}
