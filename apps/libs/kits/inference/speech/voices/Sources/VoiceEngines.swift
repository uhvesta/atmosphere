import InferenceKit
import InferenceKitSupertonic

/// The engine for each kind of downloaded voice model, which an app hands
/// to `Speaker`. Adding or swapping an engine is a case here and its
/// package; Speaker and the apps stay as they are.
public enum VoiceEngines {
    /// The engine that runs `model`, a download of `entry`.
    @Sendable public static func engine(for model: InstalledModel, _ entry: VoiceModel) -> any SpeechEngine {
        switch entry.engine {
        case .supertonic: SupertonicEngine(model: model)
        }
    }
}
