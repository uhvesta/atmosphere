import InferenceKit

extension ModelRuntimeError {
    /// Why the model couldn't write for the page or look a word up, and
    /// what to do: for a reader, who tries again where it failed rather
    /// than sending a message.
    var reading: String {
        switch self {
        case .unavailable(let status): Self.reading(status)
        case .exceededContext:
            "This is too long for the model to read at once. Pick a model that reads more in Settings."
        case .decodingFailure: "The model's answer came back in a shape the app can't read. Try again."
        case .declined: "The model declined to write about this. Try again, or pick another model in Settings."
        case .busy: "The model is busy. Wait a moment, then try again."
        case .cancelled: "The model stopped before it finished. Try again."
        case .interrupted(.memory):
            "The device ran low on memory, so the app stopped the model. Try again; if it keeps happening, "
                + "close other apps or pick a smaller model in Settings."
        case .interrupted(.background): "The app stopped the model when it went to the background. Try again."
        case .failed(let reason):
            "The model stopped: \(reason). Try again; if it keeps failing, restart the app."
        case .unsupportedInput, .simulatorHost, .engineMissing: description
        }
    }

    private static func reading(_ status: ModelStatus) -> String {
        switch status {
        case .available: "The model couldn't answer. Try again."
        case .systemTooOld:
            "Apple's model needs iOS 26 or macOS 26. Update the system, or download a model in Settings."
        case .deviceNotEligible: "This device can't run Apple Intelligence. Download a model in Settings."
        case .notEnabled:
            "Apple Intelligence is turned off. Turn it on in System Settings, or download a model in Settings."
        case .modelNotReady: "Apple's model is still downloading. Wait for it to finish, then try again."
        case .notDownloaded:
            "The model picked isn't on this device. Download it again in Settings, or pick another."
        case .notEnoughMemory:
            "There isn't enough free memory for the model. Close other apps and try again, or pick a smaller "
                + "model in Settings."
        }
    }
}
