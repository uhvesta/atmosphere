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
            if case .confirming(let others, let busy) = store.phase {
                confirming(others: others, busy: busy)
            } else {
                choices(survey)
            }
        }
        .padding(40)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private func choices(_ survey: InstallSurvey) -> some View {
        HStack(spacing: 12) {
            if store.offersRunFromHere {
                Button("Run from Here") { store.send(.runFromHereTapped) }
                    .help(
                        survey.sourceIsEjectable
                            ? "Use this copy where it is; nothing in Applications changes. Humata's daemon will run "
                                + "from this disk image or drive, and stop when it's ejected."
                            : "Use this copy where it is; nothing in Applications changes")
            } else {
                Button("Quit") { store.send(.quitTapped) }
            }
            if store.offersInstall {
                let install = Button(survey.installed == nil ? "Install" : "Install and Replace") {
                    store.send(.installTapped)
                }
                if store.opensInstalled {
                    install
                } else {
                    install.keyboardShortcut(.defaultAction)
                }
            }
            if store.opensInstalled {
                Button("Open the Installed Copy") { store.send(.openInstalledTapped) }
                    .keyboardShortcut(.defaultAction)
            }
        }
        .disabled(store.isWorking)
    }

    /// What installing would interrupt, and the choice to go on.
    private func confirming(others: [RunningCopy], busy: Bool) -> some View {
        VStack(spacing: 12) {
            Text(Self.interrupting(others: others, busy: busy))
                .font(.callout)
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: 460)
            HStack(spacing: 12) {
                Button(busy ? "Wait" : "Cancel") { store.send(.cancelTapped) }
                    .keyboardShortcut(.cancelAction)
                Button(others.isEmpty ? "Stop and Install" : "Quit Them and Install") { store.send(.confirmTapped) }
                    .keyboardShortcut(.defaultAction)
            }
        }
    }

    private static func interrupting(others: [RunningCopy], busy: Bool) -> String {
        var said: [String] = []
        if !others.isEmpty {
            let places = others.map { $0.path.isEmpty ? "another place" : $0.path }.joined(separator: ", ")
            said.append(
                (others.count == 1 ? "Another copy of Humata is open" : "Other copies of Humata are open")
                    + " (\(places)). Installing quits \(others.count == 1 ? "it" : "them") first.")
        }
        if busy {
            said.append(
                "A session on this Mac is in a turn. Installing stops Humata's daemon, which ends it; wait for it "
                    + "to finish to keep it.")
        }
        return said.joined(separator: " ")
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
                + "it to \(path), quits any other open copy of Humata, stops Humata's daemon if one is running, "
                + "which ends any turn in progress, then opens it from there and quits this copy, which you can "
                + "then delete."
        case .install(replacing: let installed?):
            return "\(path) is Humata \(installed), older than this copy (\(survey.version)). Install quits any "
                + "other open copy of Humata, stops Humata's daemon, which ends any turn in progress, puts this "
                + "copy in its place, and opens it."
        case .openInstalled(let installed) where installed > survey.version:
            return "\(path) is Humata \(installed), newer than this copy (\(survey.version)), whose daemon "
                + "can't run in place of the newer one's. Open the installed copy and quit this one."
        case .openInstalled(let installed):
            return "\(path) is Humata \(installed), as new as this copy (\(survey.version)), so nothing is "
                + "copied. Open it and quit this copy, or run this copy from here."
        }
    }

    @ViewBuilder private var status: some View {
        switch store.phase {
        case .offering, .confirming:
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
        case .checking: "Checking for other copies of Humata and turns in progress…"
        case .copying: "Copying Humata…"
        case .quittingOthers: "Quitting the other copies of Humata…"
        case .stoppingDaemon: "Stopping Humata's daemon…"
        case .replacing: "Putting the new copy in place…"
        case .opening: "Opening Humata from Applications…"
        }
    }
}
