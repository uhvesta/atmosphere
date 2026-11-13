import ComposableArchitecture
import LifrLockClient
import LifrRemindersClient
import LifrSyncClient
import SwiftUI

/// Three short screens, each skippable, or one that says another device's
/// tasks arrived.
package struct OnboardingView: View {
    @Bindable package var store: StoreOf<OnboardingFeature>

    package init(store: StoreOf<OnboardingFeature>) {
        self.store = store
    }

    package var body: some View {
        VStack(spacing: 0) {
            ScrollView {
                Group {
                    switch store.step {
                    case .welcome: WelcomeStep()
                    case .avenues: AvenuesStep(store: store)
                    case .setup: SetupStep(store: store)
                    case .found: FoundStep()
                    }
                }
                .frame(maxWidth: 520)
                .padding(24)
                .frame(maxWidth: .infinity)
            }
            .scrollBounceBehavior(.basedOnSize)
            footer
        }
        .task { await store.send(.task).finish() }
        .animation(.snappy, value: store.step)
        #if os(macOS)
            .frame(minWidth: 480, minHeight: 560)
        #endif
    }

    private var footer: some View {
        VStack(spacing: 12) {
            if let error = store.error {
                Text("Couldn't save your Avenues: \(error). Try again, or skip and add them later from Avenues.")
                    .font(.footnote)
                    .foregroundStyle(.red)
                    .multilineTextAlignment(.center)
            }
            Button {
                store.send(.continueTapped)
            } label: {
                Group {
                    if store.isFinishing {
                        ProgressView()
                    } else {
                        Text(store.step == .setup || store.step == .found ? "Start" : "Continue")
                    }
                }
                .frame(maxWidth: 360)
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
            .keyboardShortcut(.defaultAction)
            .disabled(store.isFinishing)
            HStack(spacing: 24) {
                if store.step == .avenues || store.step == .setup {
                    Button("Back") { store.send(.backTapped) }
                }
                if store.step != .found {
                    Button("Skip") { store.send(.skipTapped) }
                        .accessibilityHint("Starts with only the Inbox")
                }
            }
            .buttonStyle(.borderless)
            .disabled(store.isFinishing)
        }
        .padding(.horizontal, 24)
        .padding(.bottom, 16)
        .padding(.top, 8)
    }
}

private struct WelcomeStep: View {
    var body: some View {
        VStack(spacing: 24) {
            Text("Welcome to Lifr")
                .font(.largeTitle.bold())
                .multilineTextAlignment(.center)
            HStack(spacing: 12) {
                Part(symbol: "square.stack.3d.up", title: "Avenue", caption: "A part of your life, like Home")
                Image(systemName: "chevron.right").foregroundStyle(.tertiary).accessibilityHidden(true)
                Part(symbol: "folder", title: "Project", caption: "Something with an end, like Move house")
                Image(systemName: "chevron.right").foregroundStyle(.tertiary).accessibilityHidden(true)
                Part(symbol: "circle", title: "Task", caption: "One step, like Book the van")
            }
            Text(
                """
                Every task lives in an Avenue, and can belong to a Project there. A task without one goes to \
                the Inbox. Each task has a status: To Do, In Progress, Done, Procrastinated or Won't Do.
                """
            )
            .foregroundStyle(.secondary)
            .multilineTextAlignment(.center)
        }
    }

    private struct Part: View {
        let symbol: String
        let title: LocalizedStringKey
        let caption: LocalizedStringKey

        var body: some View {
            VStack(spacing: 6) {
                Image(systemName: symbol)
                    .font(.title)
                    .foregroundStyle(.tint)
                    .frame(height: 36)
                    .accessibilityHidden(true)
                Text(title).font(.headline)
                Text(caption)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            }
            .frame(maxWidth: .infinity)
            .accessibilityElement(children: .combine)
        }
    }
}

private struct AvenuesStep: View {
    @Bindable var store: StoreOf<OnboardingFeature>

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Pick a few Avenues")
                .font(.title.bold())
            Text("The Inbox is already there. You can add, rename or archive Avenues any time.")
                .foregroundStyle(.secondary)
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 140), spacing: 10)], spacing: 10) {
                ForEach(OnboardingFeature.State.suggestions) { suggestion in
                    let isPicked = store.picked.contains(suggestion.key)
                    Button {
                        store.send(.suggestionToggled(suggestion.key))
                    } label: {
                        Label(suggestion.name, systemImage: isPicked ? "checkmark.circle.fill" : suggestion.symbol)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(12)
                            .background(
                                isPicked ? AnyShapeStyle(.tint.opacity(0.15)) : AnyShapeStyle(.fill.quaternary),
                                in: .rect(cornerRadius: 12)
                            )
                            .contentShape(.rect(cornerRadius: 12))
                    }
                    .buttonStyle(.plain)
                    .accessibilityAddTraits(isPicked ? .isSelected : [])
                }
            }
            TextField("Or name your own", text: $store.custom)
                .textFieldStyle(.roundedBorder)
                .onSubmit { store.send(.continueTapped) }
        }
    }
}

private struct SetupStep: View {
    let store: StoreOf<OnboardingFeature>

    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            Text("A few choices")
                .font(.title.bold())
            Text("Each is optional. You can change them later in Settings.")
                .foregroundStyle(.secondary)
            Choice(
                symbol: "bell", title: "Reminders",
                reason: "Lifr reminds you about a task only when you add a reminder to it.",
                done: notificationsDone
            ) {
                if store.notifications == .notDetermined {
                    Button("Allow Notifications") { store.send(.allowNotificationsTapped) }
                }
            }
            Choice(symbol: "icloud", title: "iCloud", reason: iCloudReason, done: nil) {}
            Choice(
                symbol: "lock", title: "App lock",
                reason: "Asks for \(lockName) when Lifr opens, so someone holding your device can't read your tasks.",
                done: store.locksApp ? String(localized: "On") : nil
            ) {
                if !store.locksApp {
                    Button("Turn On") { store.send(.lockTapped) }
                }
            }
        }
    }

    private var notificationsDone: String? {
        switch store.notifications {
        case .allowed: String(localized: "Allowed")
        case .denied: String(localized: "Off. Turn them on in the Settings app.")
        case .notDetermined: nil
        }
    }

    private var iCloudReason: String {
        switch store.sync.status {
        case .failed(.noAccount):
            String(
                localized: """
                    Not signed in, so your tasks stay on this device only. Sign in to iCloud to see them on \
                    your other devices.
                    """)
        case .failed(.notSetUp):
            String(localized: "Sync is off in this build, so your tasks stay on this device.")
        default:
            String(localized: "Your tasks sync to your other devices through your iCloud, encrypted end to end.")
        }
    }

    private var lockName: String {
        switch store.biometry {
        case .faceID: String(localized: "Face ID")
        case .touchID: String(localized: "Touch ID")
        case .opticID: String(localized: "Optic ID")
        case .passcode: String(localized: "your passcode")
        }
    }

    private struct Choice<Actions: View>: View {
        let symbol: String
        let title: LocalizedStringKey
        let reason: String
        let done: String?
        @ViewBuilder let actions: () -> Actions

        var body: some View {
            HStack(alignment: .top, spacing: 14) {
                Image(systemName: symbol)
                    .font(.title2)
                    .foregroundStyle(.tint)
                    .frame(width: 32)
                    .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: 6) {
                    Text(title).font(.headline)
                    Text(reason).foregroundStyle(.secondary)
                    if let done {
                        Text(done).font(.callout.weight(.medium))
                    }
                    actions()
                }
            }
        }
    }
}

private struct FoundStep: View {
    var body: some View {
        VStack(spacing: 16) {
            Image(systemName: "checkmark.icloud")
                .font(.system(size: 56))
                .foregroundStyle(.tint)
                .accessibilityHidden(true)
            Text("Found your tasks")
                .font(.largeTitle.bold())
            Text("Your Avenues and tasks from your other devices are here, and they keep syncing.")
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
        }
        .padding(.top, 40)
    }
}
