import Foundation
import InferenceKit
import SwiftUI

/// A Voice picker on the Models screen: for every language, or for one
/// language, which can follow the one for every language.
public struct VoicePicker: Identifiable {
    /// The language it picks for, a BCP 47 tag, or nil for every language.
    public var language: String?
    /// What it picks now; nil follows the one for every language.
    public var choice: VoiceChoice?

    public init(language: String?, choice: VoiceChoice?) {
        self.language = language
        self.choice = choice
    }

    public var id: String { language ?? "" }
}

/// What the person can do in the Voices section.
public struct VoiceActions {
    public var start: (ModelID) -> Void
    public var pause: (ModelID) -> Void
    public var remove: (ModelID) -> Void
    /// Picks a voice for a language, or for every language when it's nil;
    /// a nil voice has the language follow the one for every language.
    public var pick: (VoiceChoice?, _ language: String?) -> Void

    public init(
        start: @escaping (ModelID) -> Void, pause: @escaping (ModelID) -> Void, remove: @escaping (ModelID) -> Void,
        pick: @escaping (VoiceChoice?, String?) -> Void
    ) {
        self.start = start
        self.pause = pause
        self.remove = remove
        self.pick = pick
    }
}

/// The Models screen's Voices section: the voice that reads text aloud,
/// for every language and for each one picked apart, and the voice models
/// to download, each with its size and languages. Nothing downloads until
/// the person taps.
public struct VoicesSection: View {
    let app: String
    let models: [VoiceModel]
    let installed: Set<ModelID>
    let downloads: [ModelID: DownloadState]
    let pickers: [VoicePicker]
    let actions: VoiceActions

    public init(
        app: String, models: [VoiceModel], installed: Set<ModelID>, downloads: [ModelID: DownloadState],
        pickers: [VoicePicker], actions: VoiceActions
    ) {
        self.app = app
        self.models = models
        self.installed = installed
        self.downloads = downloads
        self.pickers = pickers
        self.actions = actions
    }

    public var body: some View {
        Section {
            ForEach(pickers) { picker in
                Picker(
                    Self.title(of: picker),
                    selection: Binding(get: { picker.choice }, set: { actions.pick($0, picker.language) })
                ) {
                    if picker.language != nil {
                        Text("Same as other languages").tag(VoiceChoice?.none)
                    }
                    Text("System voice").tag(VoiceChoice?.some(.system))
                    ForEach(choices(for: picker.language)) { option in
                        Text(option.name).tag(VoiceChoice?.some(option.choice))
                    }
                }
            }
            ForEach(models) { model in
                VStack(alignment: .leading, spacing: 4) {
                    ModelRow(
                        model.spec, download: downloads[model.id], isInstalled: installed.contains(model.id),
                        window: 0, pickWindow: { _ in }, start: { actions.start(model.id) },
                        pause: { actions.pause(model.id) }, remove: { actions.remove(model.id) })
                    Text(Self.languages(of: model)).font(.caption).foregroundStyle(.secondary)
                }
            }
        } header: {
            Text("Voices")
        } footer: {
            Text(
                "The system voice needs no download. A voice model sounds more natural, and like the other models "
                    + "here it runs on this device. Where it doesn't speak a language, \(app) uses the system "
                    + "voice. Supertonic 3's licence, OpenRAIL-M, forbids using it to pass speech off as a real "
                    + "person's, to deceive or to harm. Audio8 Persian's licence allows only non-commercial use.")
        }
    }

    /// The voices a downloaded model has that speak `language`, or all of
    /// them for every language.
    func choices(for language: String?) -> [VoiceOption] {
        models.filter { installed.contains($0.id) && (language.map($0.speaks) ?? true) }.flatMap { model in
            model.voices.map { voice in
                VoiceOption(choice: .model(model.id, voice: voice.id), name: "\(model.spec.name), \(voice.name)")
            }
        }
    }

    static func title(of picker: VoicePicker) -> String {
        guard let language = picker.language else { return String(localized: "Voice") }
        let name = Locale.current.localizedString(forIdentifier: language) ?? language
        return String(localized: "Voice for \(name)")
    }

    /// The languages `model` speaks, by name, in alphabetical order.
    static func languages(of model: VoiceModel) -> String {
        let names = model.languages.map { Locale.current.localizedString(forLanguageCode: $0) ?? $0 }
            .sorted { $0.localizedStandardCompare($1) == .orderedAscending }
        return String(localized: "Speaks \(names.formatted(.list(type: .and)))")
    }
}

/// A voice to pick, and its name.
struct VoiceOption: Identifiable, Equatable {
    var choice: VoiceChoice
    var name: String

    var id: VoiceChoice { choice }
}
