import CaptureKit
import Foundation
import LifrAssistantClient

extension DictationClient {
    /// Dictates with the system's speech recognition, on the device only,
    /// in the person's language.
    package static func live() -> Self {
        let dictation = Dictation()
        return DictationClient(
            start: { try await dictation.start() },
            stop: { dictation.stop() },
            cancel: { dictation.cancel() })
    }
}

extension PhotoLibraryClient {
    /// The person's photo library, through PhotoKit, and the device's camera.
    package static func live() -> Self {
        PhotoLibraryClient(
            access: { PhotoLibrary.access },
            requestAccess: { await PhotoLibrary.requestAccess() },
            recents: { await PhotoLibrary.recent(limit: 24, side: 240) },
            photo: { id in try await PhotoLibrary.data(of: id) },
            hasCamera: { await Camera.isAvailable })
    }
}
