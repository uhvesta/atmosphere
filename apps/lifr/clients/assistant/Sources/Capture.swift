import CaptureKit
import ComposableArchitecture
import Foundation

/// Writes what the person says into the message field, on this device, for
/// a model that doesn't listen to recordings.
@DependencyClient
package struct DictationClient: Sendable {
    /// Asks for speech recognition and the microphone if Lifr hasn't yet,
    /// then listens. The stream has everything heard so far each time it
    /// grows, and ends after `stop`. Throws `DictationError`.
    package var start: @Sendable () async throws -> AsyncThrowingStream<String, any Error>
    /// Stops listening; the stream ends with the last words.
    package var stop: @Sendable () async -> Void
    /// Stops listening and throws away what's not yet written.
    package var cancel: @Sendable () async -> Void
}

extension DictationClient {
    /// Hears `heard`, one after another, then waits to be stopped: for
    /// tests and previews.
    package static func scripted(_ heard: [String] = ["Move my run", "Move my run to Friday"]) -> Self {
        let listening = LockIsolated<AsyncThrowingStream<String, any Error>.Continuation?>(nil)
        return DictationClient(
            start: {
                let (stream, continuation) = AsyncThrowingStream.makeStream(of: String.self)
                for words in heard { continuation.yield(words) }
                listening.setValue(continuation)
                return stream
            },
            stop: { listening.withValue { $0?.finish() } },
            cancel: { listening.withValue { $0?.finish() } })
    }
}

extension DictationClient: TestDependencyKey {
    package static let testValue = DictationClient()
    package static let previewValue = DictationClient.scripted()
}

/// The person's photo library and camera, for + on the iPhone.
@DependencyClient
package struct PhotoLibraryClient: Sendable {
    /// Whether Lifr may show the person's photos, without asking.
    package var access: @Sendable () -> PhotosAccess = { .denied }
    /// Asks the person, the first time, whether Lifr may show their photos.
    package var requestAccess: @Sendable () async -> PhotosAccess = { .denied }
    /// The newest photos, as thumbnails, newest first.
    package var recents: @Sendable () async -> [RecentPhoto] = { [] }
    /// A recent photo itself. Throws `PhotoLibraryError`.
    package var photo: @Sendable (_ id: RecentPhoto.ID) async throws -> Data
    /// Whether there's a camera to take a photo with.
    package var hasCamera: @Sendable () async -> Bool = { false }
}

extension PhotoLibraryClient {
    /// A library of `recents`, each photo its thumbnail, that Lifr may
    /// show: for tests and previews.
    package static func scripted(_ recents: [RecentPhoto], camera: Bool = false) -> Self {
        PhotoLibraryClient(
            access: { .allowed },
            requestAccess: { .allowed },
            recents: { recents },
            photo: { id in
                guard let photo = recents.first(where: { $0.id == id }) else { throw PhotoLibraryError.missing }
                return photo.thumbnail
            },
            hasCamera: { camera })
    }

    /// A library Lifr may not show, as when the person said no.
    package static let denied = PhotoLibraryClient(
        access: { .denied }, requestAccess: { .denied }, recents: { [] },
        photo: { _ in throw PhotoLibraryError.missing },
        hasCamera: { false })
}

extension PhotoLibraryClient: TestDependencyKey {
    package static let testValue = PhotoLibraryClient()
    package static let previewValue = PhotoLibraryClient.denied
}

/// Reads the files the person picks to send: a photo as it is, anything
/// else as its text.
@DependencyClient
package struct FileReaderClient: Sendable {
    /// Reads the file at `url`, keeping at most `ChatAttachment.textLimit`
    /// characters of text. Throws `FileError`.
    package var read: @Sendable (_ url: URL) async throws -> PickedFile
}

extension FileReaderClient {
    /// Reads the files on disk, as the app does.
    package static let live = FileReaderClient { url in
        try PickedFile.read(url, limit: ChatAttachment.textLimit)
    }

    /// Reads `files`, by the URL's last path component: for tests and
    /// previews.
    package static func scripted(_ files: [String: PickedFile]) -> Self {
        FileReaderClient { url in
            guard let file = files[url.lastPathComponent] else {
                throw FileError.unreadable(name: url.lastPathComponent, reason: "no such file")
            }
            return file
        }
    }
}

extension FileReaderClient: TestDependencyKey {
    package static let testValue = FileReaderClient()
    package static let previewValue = FileReaderClient.live
}

extension DependencyValues {
    package var dictation: DictationClient {
        get { self[DictationClient.self] }
        set { self[DictationClient.self] = newValue }
    }

    package var photoLibrary: PhotoLibraryClient {
        get { self[PhotoLibraryClient.self] }
        set { self[PhotoLibraryClient.self] = newValue }
    }

    package var fileReader: FileReaderClient {
        get { self[FileReaderClient.self] }
        set { self[FileReaderClient.self] = newValue }
    }
}
