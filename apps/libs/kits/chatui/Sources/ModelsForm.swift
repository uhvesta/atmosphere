import InferenceKit
import SwiftUI

/// What the Models screen knows of one model it lists.
public struct ModelEntryState {
    public var download: DownloadState?
    public var isInstalled: Bool
    /// The window it runs at.
    public var window: Int
    /// Where it runs and what was measured of it, once it's downloaded.
    public var tuning: ModelTuning?
    public var isMeasuring: Bool
    /// Why measuring it didn't finish.
    public var measureError: String?

    public init(
        download: DownloadState?, isInstalled: Bool, window: Int, tuning: ModelTuning?, isMeasuring: Bool,
        measureError: String?
    ) {
        self.download = download
        self.isInstalled = isInstalled
        self.window = window
        self.tuning = tuning
        self.isMeasuring = isMeasuring
        self.measureError = measureError
    }
}

/// What the person can do to a model the Models screen lists.
public struct ModelEntryActions {
    public var start: (ModelID) -> Void
    public var pause: (ModelID) -> Void
    public var remove: (ModelID) -> Void
    public var pickWindow: (Int, ModelID) -> Void
    public var measure: (ModelID) -> Void
    /// Where it runs, or nil for the app to pick.
    public var pickBackend: (ModelBackend?, ModelID) -> Void
    /// Opens the screen comparing its formats, engines and settings.
    public var tune: (ModelID) -> Void

    public init(
        start: @escaping (ModelID) -> Void, pause: @escaping (ModelID) -> Void, remove: @escaping (ModelID) -> Void,
        pickWindow: @escaping (Int, ModelID) -> Void, measure: @escaping (ModelID) -> Void,
        pickBackend: @escaping (ModelBackend?, ModelID) -> Void, tune: @escaping (ModelID) -> Void
    ) {
        self.start = start
        self.pause = pause
        self.remove = remove
        self.pickWindow = pickWindow
        self.measure = measure
        self.pickBackend = pickBackend
        self.tune = tune
    }
}

/// The Models screen: what each use runs on, from the app, then the
/// models to download, with how each runs here once it's downloaded, and
/// a Hugging Face link to download another from.
public struct ModelsForm<Uses: View, Options: View>: View {
    let app: String
    let models: [ModelSpec]
    let entry: (ModelSpec) -> ModelEntryState
    let actions: ModelEntryActions
    @Binding var link: String
    @Binding var sha256: String
    let addError: String?
    let isAdding: Bool
    let canAdd: Bool
    let add: () -> Void
    let uses: Uses
    let options: Options

    /// `app` names the app in the screen's words; `models` are those to
    /// download, and `entry` what's known of each. `uses` picks what each
    /// use runs on, and `options` follows it.
    public init(
        app: String, models: [ModelSpec], entry: @escaping (ModelSpec) -> ModelEntryState, actions: ModelEntryActions,
        link: Binding<String>, sha256: Binding<String>, addError: String?, isAdding: Bool, canAdd: Bool,
        add: @escaping () -> Void, @ViewBuilder uses: () -> Uses, @ViewBuilder options: () -> Options
    ) {
        self.app = app
        self.models = models
        self.entry = entry
        self.actions = actions
        _link = link
        _sha256 = sha256
        self.addError = addError
        self.isAdding = isAdding
        self.canAdd = canAdd
        self.add = add
        self.uses = uses()
        self.options = options()
    }

    public var body: some View {
        Form {
            uses
            options
            Section {
                ForEach(models) { model in
                    ModelEntry(model: model, state: entry(model), actions: actions)
                }
            } header: {
                Text("Models")
            } footer: {
                Text(
                    "Measure a downloaded model to see how fast it runs here, and how much memory and CPU it "
                        + "takes, on the GPU and on the CPU. \(app) then runs it where it's fastest, unless you pick. "
                        + "Tune it to compare its formats, engines and their settings side by side.")
            }
            Section {
                TextField("Hugging Face link", text: $link)
                    .textContentType(.URL)
                    .autocorrectionDisabled()
                    #if os(iOS)
                        .keyboardType(.URL)
                        .textInputAutocapitalization(.never)
                    #endif
                TextField("SHA-256 (optional)", text: $sha256)
                    .autocorrectionDisabled()
                    #if os(iOS)
                        .textInputAutocapitalization(.never)
                    #endif
                if let addError {
                    Text(addError).foregroundStyle(.red)
                }
                Button(action: add) {
                    if isAdding {
                        ProgressView()
                    } else {
                        Text("Download model")
                    }
                }
                .disabled(!canAdd)
            } header: {
                Text("From Hugging Face")
            } footer: {
                Text(
                    "Paste the link to a .litertlm or .gguf model, such as one from litert-community. \(app) pins "
                        + "it to the current commit and checks every file. Give the SHA-256 its publisher lists to "
                        + "be sure it's theirs.")
            }
        }
        .navigationTitle("Models")
    }
}

/// A model to download, and once it's downloaded, how it runs here.
private struct ModelEntry: View {
    let model: ModelSpec
    let state: ModelEntryState
    let actions: ModelEntryActions

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            ModelRow(
                model, download: state.download, isInstalled: state.isInstalled, window: state.window,
                pickWindow: { actions.pickWindow($0, model.id) }, start: { actions.start(model.id) },
                pause: { actions.pause(model.id) }, remove: { actions.remove(model.id) })
            if state.isInstalled, state.download == nil, let tuning = state.tuning {
                ModelTuningView(
                    tuning, isMeasuring: state.isMeasuring, error: state.measureError,
                    measure: { actions.measure(model.id) }, pick: { actions.pickBackend($0, model.id) })
                Button("Tune for this device…") { actions.tune(model.id) }
                    .buttonStyle(.borderless)
                    .disabled(state.isMeasuring)
            }
        }
    }
}
