import InferenceKit
import SwiftUI

/// A model to download: its name, size and licence, what's to do with it
/// now, its download's progress, and for one downloaded with a choice of
/// windows, the window it runs at.
public struct ModelRow: View {
    let model: ModelSpec
    let download: DownloadState?
    let isInstalled: Bool
    let window: Int
    let pickWindow: (Int) -> Void
    let start: () -> Void
    let pause: () -> Void
    let remove: () -> Void

    /// `window` is the one it runs at; `start` downloads it or goes on.
    public init(
        _ model: ModelSpec, download: DownloadState?, isInstalled: Bool, window: Int,
        pickWindow: @escaping (Int) -> Void, start: @escaping () -> Void, pause: @escaping () -> Void,
        remove: @escaping () -> Void
    ) {
        self.model = model
        self.download = download
        self.isInstalled = isInstalled
        self.window = window
        self.pickWindow = pickWindow
        self.start = start
        self.pause = pause
        self.remove = remove
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .firstTextBaseline) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(model.name).font(.headline)
                    Text(details).font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                action
            }
            progress
            if isInstalled, !model.contextChoices.isEmpty {
                windowPicker
            }
        }
        .buttonStyle(.borderless)
    }

    /// The window it runs at, for a model with a choice: a larger one
    /// keeps more of a conversation and takes more memory.
    @ViewBuilder private var windowPicker: some View {
        Picker("Context window", selection: Binding(get: { window }, set: pickWindow)) {
            ForEach(model.contextChoices, id: \.self) { tokens in
                Text("\(tokens / 1_024)K tokens").tag(tokens)
            }
        }
        let memory = model.memoryBytes(at: window)
        Text(
            memory > 0
                ? "Uses about \(Self.bytes(memory)) of memory. A larger window keeps more of a conversation."
                : "A larger window keeps more of a conversation and uses more memory."
        )
        .font(.caption).foregroundStyle(.secondary)
    }

    private var details: String {
        "\(model.summary) · \(Self.bytes(model.downloadSize)) · \(model.license)"
    }

    private static func bytes(_ count: Int64) -> String {
        ByteCountFormatter.string(fromByteCount: count, countStyle: .file)
    }

    @ViewBuilder private var action: some View {
        switch download {
        case .downloading:
            Button("Pause", action: pause)
        case .verifying:
            Text("Checking").foregroundStyle(.secondary)
        case .paused:
            Button("Resume", action: start)
        case .failed:
            Button("Download model", action: start)
        case nil:
            if isInstalled {
                Button("Remove", role: .destructive, action: remove)
            } else {
                Button("Download model", action: start)
            }
        }
    }

    @ViewBuilder private var progress: some View {
        switch download {
        case .downloading(let received, let total), .paused(let received, let total):
            ProgressView(value: download?.fraction ?? 0) {
                EmptyView()
            } currentValueLabel: {
                Text("\(Self.bytes(received)) of \(Self.bytes(total))")
            }
        case .verifying:
            ProgressView(value: 1)
        case .failed(let error):
            Text(error.description).font(.caption).foregroundStyle(.red)
        case nil:
            if isInstalled {
                Text("Downloaded").font(.caption).foregroundStyle(.secondary)
            }
        }
    }
}
