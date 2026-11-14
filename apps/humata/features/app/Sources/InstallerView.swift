import ComposableArchitecture
import SwiftUI

/// The installer, in the window in place of the sidebar and chat: what
/// would happen, and the choices.
package struct InstallerView: View {
    let store: StoreOf<InstallerFeature>

    package init(store: StoreOf<InstallerFeature>) {
        self.store = store
    }

    package var body: some View {
        let survey = store.survey
        VStack(spacing: 18) {
            Image(systemName: "square.and.arrow.down.on.square")
                .font(.system(size: 44))
                .foregroundStyle(.secondary)
            Text(title)
                .font(.title2.weight(.semibold))
            Text(explanation(survey))
                .font(.callout)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: 460)
            status
            HStack(spacing: 12) {
                Button("Run from Here") { store.send(.runFromHereTapped) }
                    .help("Use this copy where it is; nothing in Applications changes")
                switch survey.offer {
                case .install:
                    Button(survey.installed == nil ? "Install" : "Install and Replace") {
                        store.send(.installTapped)
                    }
                    .keyboardShortcut(.defaultAction)
                case .openInstalled:
                    Button("Open the Installed Copy") { store.send(.openInstalledTapped) }
                        .keyboardShortcut(.defaultAction)
                }
            }
            .disabled(store.isWorking)
        }
        .padding(40)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private var title: String {
        switch store.survey.offer {
        case .install: "Install Humata in Applications"
        case .openInstalled: "Humata Is Already in Applications"
        }
    }

    private func explanation(_ survey: InstallSurvey) -> String {
        let path = survey.destination.path(percentEncoded: false)
        switch survey.offer {
        case .install(replacing: nil):
            return "This copy of Humata (\(survey.version)) was opened from outside Applications. Install copies "
                + "it to \(path), opens it from there and quits this copy, which you can then delete."
        case .install(replacing: let installed?):
            return "\(path) is Humata \(installed), older than this copy (\(survey.version)). Install stops "
                + "Humata's daemon, which ends any turn in progress, puts this copy in its place, and opens it."
        case .openInstalled(let installed):
            return "\(path) is Humata \(installed), as new as this copy (\(survey.version)) or newer, so nothing "
                + "is copied. Open it and quit this copy, or run this copy from here."
        }
    }

    @ViewBuilder private var status: some View {
        switch store.phase {
        case .offering:
            EmptyView()
        case .working(let step):
            HStack(spacing: 8) {
                ProgressView().controlSize(.small)
                Text(Self.saying(step))
                    .font(.callout)
            }
        case .failed(let message):
            Label {
                Text(message)
                    .font(.callout)
                    .fixedSize(horizontal: false, vertical: true)
                    .textSelection(.enabled)
            } icon: {
                Image(systemName: "exclamationmark.triangle.fill")
                    .foregroundStyle(.orange)
            }
            .frame(maxWidth: 460)
        }
    }

    private static func saying(_ step: InstallerFeature.Step) -> String {
        switch step {
        case .copying: "Copying Humata…"
        case .stoppingDaemon: "Stopping Humata's daemon…"
        case .replacing: "Putting the new copy in place…"
        case .opening: "Opening Humata from Applications…"
        }
    }
}
