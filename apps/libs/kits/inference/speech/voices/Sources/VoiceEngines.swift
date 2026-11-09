import InferenceKit
import InferenceKitSupertonic

#if canImport(InferenceKitDualAR)
    import InferenceKitDualAR
#endif

/// The engine for each kind of downloaded voice model, which an app hands
/// to `Speaker`. Adding or swapping an engine is a case here and its
/// package; Speaker and the apps stay as they are.
public enum VoiceEngines {
    /// The engine that runs `model`, a download of `entry`.
    @Sendable public static func engine(for model: InstalledModel, _ entry: VoiceModel) -> any SpeechEngine {
        switch entry.engine {
        case .supertonic: SupertonicEngine(model: model)
        case .dualAR:
            #if canImport(InferenceKitDualAR)
                DualAREngine(model: model)
            #else
                Unavailable(model: entry.spec.name, reason: "MLX doesn't run in the Simulator; try it on a device")
            #endif
        }
    }
}

/// An engine this build leaves out, which says why when asked to speak.
private struct Unavailable: SpeechEngine {
    var model: String
    var reason: String

    func synthesize(
        _ text: String, language: String, voice: String, audio: @escaping @Sendable (SpeechAudio) -> Void
    ) async throws(SpeechError) {
        throw .synthesisFailed(model: model, reason: reason)
    }
}
