import Foundation

/// Speech: text read aloud, in a language, in a voice the person picked.
///
/// The system voice, the one the OS ships for each language, is always
/// there and needs no download. A voice model is a neural text-to-speech
/// model the person downloads from the Voices section of the Models
/// screen; it sounds more natural, and runs on this device like the
/// language models. A `SpeechEngine` turns text into PCM audio, a piece
/// at a time, so playback starts before the whole text is made.
///
/// Which one speaks is worked out here, with no audio: `SpeechRoute`
/// takes the choice, the language and what's downloaded, and falls back
/// to the system voice when the picked model isn't downloaded or doesn't
/// speak the language. The engines and the player live in
/// InferenceKitSpeech.
///
/// The voice model, picked in November 2026 for its size, its languages
/// and its licence:
///
/// - Supertonic 3, by Supertone: about 99M parameters, 31 languages, and
///   it reads raw text, so it needs no phonemizer such as espeak-ng
///   (GPL). Licensed under OpenRAIL-M, which allows shipping it in an
///   app, also commercially, as long as the use restrictions in its
///   Attachment A pass on to the people using it; the screen says so.
///   It runs on Core ML, built into the OS, from a parity export to Core
///   ML packages (iOS 18 and macOS 15 and later), so it needs no new
///   engine. About 400 MB.
///
/// Considered and left out: Kokoro 82M (Apache 2.0, but every language
/// past English and Japanese needs espeak-ng's GPL phonemizer); Qwen3-TTS
/// 0.6B (Apache 2.0, 10 languages, but it needs a language model and an
/// audio codec ported to MLX first, a follow-up); Kitten TTS and Audio8
/// (their own licences); Pocket TTS and NeuTTS (gated downloads); and
/// several with non-commercial licences.

/// A voice a voice model speaks in, such as one of Supertonic's ten.
public struct VoiceStyle: Hashable, Codable, Sendable, Identifiable {
    /// Its name in the model's files, such as "F1".
    public var id: String
    /// Its name, for the person.
    public var name: String

    public init(id: String, name: String) {
        self.id = id
        self.name = name
    }
}

/// A neural text-to-speech model to download: its files, the languages it
/// speaks, as BCP 47 base subtags such as "en", and its voices.
public struct VoiceModel: Hashable, Sendable, Identifiable {
    public var spec: ModelSpec
    public var languages: [String]
    public var voices: [VoiceStyle]

    public init(spec: ModelSpec, languages: [String], voices: [VoiceStyle]) {
        self.spec = spec
        self.languages = languages
        self.voices = voices
    }

    public var id: ModelID { spec.id }

    /// Whether it speaks `language`, a BCP 47 tag such as "pt-BR": its
    /// base subtag is one of the model's.
    public func speaks(_ language: String) -> Bool {
        languages.contains(Self.base(of: language))
    }

    /// The first subtag of `language`, lowercased: "pt" for "pt-BR".
    public static func base(of language: String) -> String {
        let first = language.split(whereSeparator: { $0 == "-" || $0 == "_" }).first.map(String.init) ?? language
        return first.lowercased()
    }
}

/// The voice that speaks: the system's, or a voice of a downloaded model.
public enum VoiceChoice: Hashable, Codable, Sendable {
    case system
    case model(ModelID, voice: String)

    /// The model it picks, if any.
    public var model: ModelID? {
        switch self {
        case .system: nil
        case .model(let id, _): id
        }
    }
}

/// What speaks one text, once the choice is checked against what's
/// downloaded and the language.
public enum SpeechRoute: Hashable, Sendable {
    /// The system voice for the language.
    case system
    /// A downloaded voice model, in one of its voices.
    case model(InstalledModel, VoiceModel, voice: String)

    /// What speaks `language` for `choice`: the model it picks if that's
    /// downloaded, speaks the language and has the voice, else the system
    /// voice. `catalog` lists the voice models there are.
    public static func resolve(
        _ choice: VoiceChoice, language: String, installed: [InstalledModel],
        catalog: [VoiceModel] = VoiceCatalog.all
    ) -> SpeechRoute {
        guard case .model(let id, let voice) = choice,
            let entry = catalog.first(where: { $0.id == id }),
            let model = installed.first(where: { $0.id == id }),
            entry.speaks(language),
            entry.voices.contains(where: { $0.id == voice })
        else { return .system }
        return .model(model, entry, voice: voice)
    }
}

/// A piece of speech: mono samples from -1 to 1, at `sampleRate` a second.
public struct SpeechAudio: Hashable, Sendable {
    public var samples: [Float]
    public var sampleRate: Double

    public init(samples: [Float], sampleRate: Double) {
        self.samples = samples
        self.sampleRate = sampleRate
    }

    /// How long it plays, in seconds.
    public var duration: Double { sampleRate > 0 ? Double(samples.count) / sampleRate : 0 }
}

/// Turns text into speech.
public protocol SpeechEngine: Sendable {
    /// Speaks `text` in `language`, a BCP 47 tag, in `voice`, one of the
    /// engine's voices, or its default when empty. Hands each piece of
    /// audio to `audio` as soon as it's made, in order, and returns once
    /// the last is handed over. Stops early, without an error, when the
    /// task is cancelled.
    func synthesize(
        _ text: String, language: String, voice: String, audio: @escaping @Sendable (SpeechAudio) -> Void
    ) async throws(SpeechError)
}

/// Why speech couldn't play.
public enum SpeechError: Error, Hashable, Sendable, CustomStringConvertible {
    /// A file of a downloaded voice model is missing or can't be read.
    case modelDamaged(model: String, file: String, reason: String)
    /// Core ML couldn't load or compile one of the model's parts.
    case modelUnloadable(model: String, part: String, reason: String)
    /// The model ran but couldn't make audio from the text.
    case synthesisFailed(model: String, reason: String)
    /// Nothing in the text is something the voice can say.
    case nothingToSay
    /// The system voice made no audio, such as when no voice for the
    /// language is installed.
    case systemVoiceFailed(language: String)
    /// The device's audio couldn't start.
    case audioUnavailable(reason: String)

    public var description: String {
        switch self {
        case .modelDamaged(let model, let file, let reason):
            "\(model)'s file \(file) is missing or damaged (\(reason)). Remove the voice in Settings, under "
                + "Models, and download it again. Until then the system voice speaks."
        case .modelUnloadable(let model, let part, let reason):
            "\(model) couldn't load its \(part) (\(reason)). Restart the app; if it happens again, remove the "
                + "voice and download it again, or pick the system voice."
        case .synthesisFailed(let model, let reason):
            "\(model) couldn't read this text aloud (\(reason)). Pick the system voice in Settings, under Models, "
                + "to hear it."
        case .nothingToSay:
            "There's nothing in this text the voice can say."
        case .systemVoiceFailed(let language):
            Self.noSystemVoice(language)
        case .audioUnavailable(let reason):
            "Audio couldn't start (\(reason)). Check that another app isn't holding the speaker, then try again."
        }
    }

    /// What to do when the system has no voice for `language`, which
    /// depends on whether a voice model speaks it.
    private static func noSystemVoice(_ language: String) -> String {
        let name = Locale.current.localizedString(forLanguageCode: VoiceModel.base(of: language)) ?? language
        if VoiceCatalog.all.contains(where: { $0.speaks(language) }) {
            return "The system has no voice for \(name). Download a voice that speaks it in Settings, under "
                + "Models, or add a system voice in the device's settings, under Accessibility, Spoken Content, "
                + "Voices."
        }
        return "No voice on this device speaks \(name) yet: the system has none, and none of the voices to "
            + "download speak it. If the device's settings, under Accessibility, Spoken Content, Voices, offer "
            + "one, add it there."
    }
}
