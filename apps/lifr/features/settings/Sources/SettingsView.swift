import ChatFeatures
import ComposableArchitecture
import LifrAPI
import LifrAssistantClient
import LifrBugReport
import LifrCore
import LifrFeedbackClient
import LifrLockClient
import LifrRemindersClient
import LifrSessionClient
import LifrSkills
import LifrSyncClient
import LifrUI
import SwiftUI

/// Settings, or one page of them. Only `.all` starts the settings' work and
/// shows their alerts; with pages, whatever holds them does that once.
package struct SettingsView: View {
    @Bindable package var store: StoreOf<SettingsFeature>
    private let page: SettingsPage
    @Shared(.namesConversations) private var namesConversations
    @Shared(.picksSkills) private var picksSkills
    @Shared(.badgeCount) private var badgeCount
    @Shared(.completionSound) private var completionSound
    @Shared(.compactsConversations) private var compactsConversations

    package init(store: StoreOf<SettingsFeature>, page: SettingsPage = .all) {
        self.store = store
        self.page = page
    }

    private func shows(_ part: SettingsPage) -> Bool { page == .all || page == part }

    @ViewBuilder private var sections: some View {
        if shows(.general) {
            general
            #if os(macOS)
                quickAddKeys
            #endif
        }
        if shows(.sync) {
            SyncSection(store: store)
            LockSection(store: store)
        }
        if shows(.assistant) { assistant }
        if shows(.general) {
            Section {
                PushButton("Bug Reports") { store.send(.bugReportsTapped) }
            } footer: {
                Text(bugReportsFooter)
            }
            Section {
                Button(store.tipsReset ? "Tips Will Show Again" : "Reset Tips") { store.send(.resetTipsTapped) }
                    .disabled(store.tipsReset)
            } footer: {
                Text("Shows the tips about swiping a task and dragging one onto a day again.")
            }
            Section {
                PushButton("Data") { store.send(.dataTapped) }
            } footer: {
                Text("Export your tasks, import a file, or bring back a daily backup.")
            }
            AboutSection(version: store.appVersion)
            Section {
                Button("Delete All Data…", role: .destructive) { store.send(.eraseTapped) }
            } footer: {
                Text("Deletes your tasks here, in iCloud and, once they sync, on your other devices.")
            }
        }
        if shows(.agents) && store.showsAgents {
            avenueAccess
            connectedApps
            commandLine
        }
    }

    package var body: some View {
        Form {
            if let loadError = store.loadError {
                Section {
                    Text(loadError).foregroundStyle(.red)
                    Button("Try Again") { store.send(.task) }
                } header: {
                    Text("Couldn't load your settings")
                }
            } else {
                sections
            }
        }
        .navigationTitle("Settings")
        .navigationDestination(
            item: shows(.general) ? $store.scope(state: \.activity, action: \.activity) : .constant(nil)
        ) { store in
            ActivityView(store: store)
        }
        .navigationDestination(
            item: shows(.general) ? $store.scope(state: \.labels, action: \.labels) : .constant(nil)
        ) { store in
            LabelsView(store: store)
        }
        .navigationDestination(
            item: shows(.assistant) ? $store.scope(state: \.skills, action: \.skills) : .constant(nil)
        ) { store in
            SkillsView(store: store)
        }
        .navigationDestination(
            item: shows(.assistant) ? $store.scope(state: \.instructions, action: \.instructions) : .constant(nil)
        ) { store in
            InstructionsView(store: store)
        }
        .navigationDestination(
            item: shows(.assistant) ? $store.scope(state: \.models, action: \.models) : .constant(nil)
        ) { store in
            ModelsView(store: store)
        }
        .navigationDestination(
            item: shows(.general) ? $store.scope(state: \.bugReports, action: \.bugReports) : .constant(nil)
        ) { store in
            BugReportsView(store: store)
        }
        .navigationDestination(
            item: shows(.general) ? $store.scope(state: \.data, action: \.data) : .constant(nil)
        ) { store in
            DataView(store: store)
        }
        .sheet(item: shows(.general) ? $store.scope(state: \.erase, action: \.erase) : .constant(nil)) { store in
            EraseView(store: store)
        }
        .task {
            guard page == .all else { return }
            await store.send(.task).finish()
        }
        .confirmationDialog(page == .all ? $store.scope(state: \.revoke, action: \.revoke) : .constant(nil))
        .alert(page == .all ? $store.scope(state: \.alert, action: \.alert) : .constant(nil))
    }

    #if os(macOS)
        private var quickAddKeys: some View {
            Section {
                Picker(
                    "Quick Add from any app",
                    selection: Binding(
                        get: { store.quickAddHotKey }, set: { store.send(.quickAddHotKeyPicked($0)) })
                ) {
                    ForEach(QuickAddHotKey.allCases, id: \.self) { Text($0.title) }
                }
            } footer: {
                Text(
                    "Opens a small Quick Add over whatever you're working in, even with no Lifr window open. "
                        + "If nothing opens, the system or another app uses those keys: pick others.")
            }
        }
    #endif

    @ViewBuilder private var general: some View {
        Section {
            Stepper(
                "In Progress for \(store.procrastinatedAfterDays) days",
                value: Binding(
                    get: { store.procrastinatedAfterDays }, set: { store.send(.procrastinatedDaysChanged($0)) }),
                in: 1...90)
        } header: {
            Text("Procrastinated")
        } footer: {
            Text("An In Progress task with no activity for longer than this counts as Procrastinated.")
        }
        Section {
            Picker(
                "Remind me",
                selection: Binding(get: { store.defaultReminder }, set: { store.send(.defaultReminderPicked($0)) })
            ) {
                ForEach(DefaultReminder.allCases, id: \.self) { Text($0.title).tag($0) }
            }
            Picker("App icon badge", selection: Binding($badgeCount)) {
                ForEach(BadgeCount.allCases, id: \.self) { Text($0.title).tag($0) }
            }
            Toggle(
                "Daily summary",
                isOn: Binding(
                    get: { store.dailySummary.time != nil },
                    set: { store.send(.dailySummarySet($0 ? DailySummary.morning : nil)) })
            )
            if let time = store.dailySummary.time {
                DatePicker(
                    "Summary at",
                    selection: Binding(
                        get: {
                            Calendar.current.date(bySettingHour: time.hour, minute: time.minute, second: 0, of: Date())
                                ?? Date()
                        },
                        set: { store.send(.dailySummarySet(TimeOfDay($0, in: .current))) }),
                    displayedComponents: .hourAndMinute)
            }
        } header: {
            Text("Reminders")
        } footer: {
            Text(
                """
                A new task with a due date gets this reminder on every device. The badge and the daily summary, \
                which says what's due each day that has something due, are per device.
                """)
        }
        AppearanceSection()
        Section {
            Toggle("Sound when a task is done", isOn: Binding($completionSound))
        } footer: {
            Text("A soft sound when you finish a task. This device only.")
        }
        Section {
            PushButton("Labels") { store.send(.labelsTapped) }
        } footer: {
            Text("Rename, merge or delete the labels on your tasks.")
        }
        Section {
            PushButton("Activity") { store.send(.activityTapped) }
        } footer: {
            Text("Every change, who made it, and from where. Any of them can be reverted.")
        }
    }

    @ViewBuilder private var assistant: some View {
        Section {
            Toggle("Pick a skill automatically", isOn: Binding($picksSkills))
            Toggle("Name conversations", isOn: Binding($namesConversations))
            Toggle("Compact long conversations", isOn: Binding($compactsConversations))
            PushButton("Instructions") { store.send(.instructionsTapped) }
            PushButton("Models") { store.send(.modelsTapped) }
            PushButton("Skills") { store.send(.skillsTapped) }
        } header: {
            Text("Assistant")
        } footer: {
            Text(
                "Picking a skill asks the model which skill fits each message you send without one, among "
                    + "those marked Use Automatically in Skills; with none, the assistant just talks. Names each "
                    + "new conversation after its first reply, on this device, with the model picked "
                    + "in Models. Names you give are kept. Compacting summarizes older messages once they no "
                    + "longer fit what the model reads; without it, a chat offers Compact then. Conversations "
                    + "stay on this device unless you choose Sync on one in Conversations. "
                    + "Instructions stay on this device. Skills sync through iCloud, encrypted end to end.")
        }
        ChatExperimentsSection(
            showsSpeed: Binding(store.$showsSpeed), showsPrompt: Binding(store.$showsPrompt),
            compares: Binding(store.$compares))
    }

    private var bugReportsFooter: String {
        #if os(iOS)
            String(localized: "Reports you saved, encrypted on this iPhone. Shake it to report a bug.")
        #else
            String(localized: "Reports you saved, encrypted on this Mac. Choose Help › Report a Bug to make one.")
        #endif
    }

    private var avenueAccess: some View {
        Section {
            ForEach(store.avenues) { avenue in
                Picker(
                    avenue.name,
                    selection: Binding(
                        get: { avenue.access }, set: { store.send(.agentAccessPicked(avenue.id, $0)) })
                ) {
                    ForEach(AgentAccess.allCases, id: \.self) { Text($0.title).tag($0) }
                }
            }
        } header: {
            Text("Agent access")
        } footer: {
            Text("Agents never see a Hidden Avenue, whatever their grant says.")
        }
    }

    private var connectedApps: some View {
        Section {
            if store.apps.isEmpty {
                Text("No apps yet. When an agent first runs lifr, Lifr asks you to approve it.")
                    .foregroundStyle(.secondary)
            }
            ForEach(store.apps) { app in
                ConnectedAppRow(app: app, store: store)
            }
        } header: {
            Text("Connected apps")
        }
    }

    private var commandLine: some View {
        Section {
            switch store.cli {
            case .notInstalled:
                Button("Install lifr") { store.send(.installCLITapped) }
            case .installed(let path):
                LabeledContent("Installed at", value: path)
            }
        } header: {
            Text("Command line")
        } footer: {
            Text("Then connect Claude Code with: claude mcp add lifr -- lifr mcp")
        }
    }
}

private struct ConnectedAppRow: View {
    let app: ConnectedApp
    let store: StoreOf<SettingsFeature>

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text(app.name).font(.headline)
                if app.isConnected {
                    Image(systemName: "circle.fill")
                        .font(.caption2)
                        .foregroundStyle(.green)
                        .accessibilityLabel("Connected")
                }
                Spacer()
                Button("Revoke", role: .destructive) { store.send(.revokeTapped(app.id)) }
            }
            Picker(
                "Access",
                selection: Binding(get: { app.canWrite }, set: { store.send(.writePicked(app.id, $0)) })
            ) {
                Text("Read only").tag(false)
                Text("Read and write").tag(true)
            }
            HStack {
                Group {
                    if let lastUsed = app.lastUsedAt {
                        Text("Last used \(lastUsed, format: .dateTime.month().day().hour().minute())")
                    } else {
                        Text("Not used yet")
                    }
                    Text("Expires \(app.grant.expiresAt, format: .dateTime.month().day().year())")
                }
                .font(.caption)
                .foregroundStyle(.secondary)
                Spacer()
                Button("Renew") { store.send(.renewTapped(app.id)) }
                    .font(.caption)
            }
        }
        .buttonStyle(.borderless)
    }
}

/// The approval sheet.
package struct ApprovalView: View {
    @Bindable package var store: StoreOf<ApprovalFeature>

    package init(store: StoreOf<ApprovalFeature>) {
        self.store = store
    }

    package var body: some View {
        Form {
            Section {
                Text("\(store.request.client) wants to use your tasks through Lifr.")
                    .font(.headline)
                LabeledContent("Signed as", value: store.request.identity)
            } footer: {
                if store.request.isInterpreter {
                    Text(
                        "\(store.request.client) runs scripts, and every script it runs would share this access. "
                            + "Approve it only for a short time."
                    )
                    .foregroundStyle(.orange)
                }
            }
            Section {
                Toggle("Make changes", isOn: $store.canWrite)
                if store.canWrite {
                    Toggle("Ask me before each change", isOn: $store.confirmWrites)
                }
            } header: {
                Text("Access")
            } footer: {
                Text(
                    "It can always read. Agents can never empty the trash, change Avenues or Settings, or approve apps."
                )
            }
            Section {
                Toggle("All Avenues", isOn: $store.allAvenues)
                if !store.allAvenues {
                    ForEach(store.avenues) { avenue in
                        Toggle(avenue.name, isOn: $store.avenueIDs.contains(avenue.id))
                    }
                }
            } header: {
                Text("Avenues")
            } footer: {
                Text("Hidden Avenues aren't listed; agents never see them.")
            }
            Section {
                Picker("Expires after", selection: $store.expiry) {
                    ForEach(ApprovalFeature.Expiry.allCases, id: \.self) { Text($0.title).tag($0) }
                }
            }
            if let error = store.error {
                Section { Text(error).foregroundStyle(.red) }
            }
        }
        .navigationTitle("Approve \(store.request.client)?")
        .toolbar {
            ToolbarItem(placement: .cancellationAction) {
                Button("Deny") { store.send(.denyTapped) }
            }
            ToolbarItem(placement: .confirmationAction) {
                Button("Approve") { store.send(.approveTapped) }.disabled(!store.canApprove)
            }
        }
    }
}

extension Set {
    fileprivate subscript(contains element: Element) -> Bool {
        get { contains(element) }
        set {
            if newValue { insert(element) } else { remove(element) }
        }
    }
}

extension Binding {
    fileprivate func contains<Element: Hashable>(_ element: Element) -> Binding<Bool> where Value == Set<Element> {
        self[contains: element]
    }
}

#Preview {
    NavigationStack {
        SettingsView(store: Store(initialState: SettingsFeature.State(showsAgents: true)) { SettingsFeature() })
    }
}

/// Whether sync worked last time, when, and Sync Now.
private struct SyncSection: View {
    let store: StoreOf<SettingsFeature>

    var body: some View {
        Section {
            LabeledContent("Status") {
                TimelineView(.periodic(from: .now, by: 30)) { context in
                    Text(SyncStatusText.status(store.sync, now: context.date))
                }
            }
            Toggle(
                "Sync continuously",
                isOn: Binding(
                    get: { store.syncsContinuously }, set: { store.send(.syncContinuouslyToggled($0)) })
            )
            .disabled(isUnavailable)
            #if os(macOS)
                // Syncing all the time only helps while Lifr is open.
                Toggle(
                    "Keep Lifr in the menu bar at login",
                    isOn: Binding(
                        get: { store.launchesAtLogin }, set: { store.send(.launchAtLoginToggled($0)) }))
            #endif
            Button("Sync Now") { store.send(.syncNowTapped) }
                .disabled(store.sync.status == .syncing || store.sync.status == .failed(.notSetUp))
        } header: {
            Text("Sync")
        } footer: {
            if case .failed(let error) = store.sync.status {
                Text(error.description)
            } else if store.syncsContinuously {
                Text(
                    """
                    Changes go to your other devices as you make them, and Lifr checks for theirs every 2 \
                    minutes while it's open, and now and then in the background. A change made on two devices \
                    at once that can't be merged waits for you to review it.
                    """)
            } else {
                Text(
                    """
                    Lifr syncs when you open it, every 15 minutes while it's open, and when you choose Sync Now. \
                    A change made on two devices at once that can't be merged waits for you to review it.
                    """)
            }
        }
    }

    /// This build has no iCloud, or the device isn't signed in.
    private var isUnavailable: Bool {
        store.sync.status == .failed(.notSetUp) || store.sync.status == .failed(.noAccount)
    }
}

/// Where sync is, in a few words: in Settings, the Mac's sidebar and its
/// menu bar, and the iPhone's bars when it needs the person.
package enum SyncStatusText {
    /// Whether to tell the person without being asked: sync failed, other
    /// than in a build with no iCloud or with no iCloud account, which
    /// keeps tasks on this device by choice.
    package static func needsAttention(_ sync: SyncState) -> Bool {
        guard case .failed(let error) = sync.status else { return false }
        return error != .notSetUp && error != .noAccount
    }

    /// Whether Settings can help: a failure other than being offline, which
    /// clears by itself once the network is back.
    package static func needsFix(_ sync: SyncState) -> Bool {
        guard needsAttention(sync), case .failed(let error) = sync.status else { return false }
        if case .unreachable = error { return false }
        return true
    }

    /// Why sync failed, in full, or nil when it didn't.
    package static func reason(_ sync: SyncState) -> String? {
        guard case .failed(let error) = sync.status else { return nil }
        return error.description
    }

    /// The SF Symbol beside the status.
    package static func symbol(_ sync: SyncState) -> String {
        switch sync.status {
        case .syncing: "arrow.triangle.2.circlepath.icloud"
        case .idle: "checkmark.icloud"
        case .failed(.notSetUp): "icloud"
        case .failed(.unreachable): "icloud.slash"
        case .failed: "exclamationmark.icloud"
        }
    }

    /// The status as of `now`; a last sync reads as how long ago it was, so
    /// a view that shows it redraws as time passes.
    package static func status(_ sync: SyncState, now: Date = .now) -> String {
        switch sync.status {
        case .syncing: return String(localized: "Syncing…")
        // Not a failure to retry: this build has no iCloud, or the device
        // isn't signed in.
        case .failed(.notSetUp): return String(localized: "Off")
        case .failed(.noAccount): return String(localized: "Not signed in")
        case .failed(.unreachable):
            // Saved here all the same, and sent once the network is back.
            guard sync.pending > 0 else { return String(localized: "Offline") }
            return String(localized: "Offline, \(waiting(sync.pending))")
        case .failed:
            guard sync.pending > 0 else { return String(localized: "Didn't finish") }
            return String(localized: "Didn't finish, \(waiting(sync.pending))")
        case .idle:
            guard let last = sync.lastSynced else { return String(localized: "Not synced yet") }
            guard now.timeIntervalSince(last) >= 60 else { return String(localized: "Synced just now") }
            let formatter = RelativeDateTimeFormatter()
            formatter.unitsStyle = .short
            return String(localized: "Synced \(formatter.localizedString(for: last, relativeTo: now))")
        }
    }

    /// "1 change waiting", "3 changes waiting", in the person's language.
    private static func waiting(_ count: Int) -> String {
        String(AttributedString(localized: "^[\(count) change](inflect: true) waiting").characters)
    }
}

/// The app lock's switch.
private struct LockSection: View {
    let store: StoreOf<SettingsFeature>

    var body: some View {
        Section {
            Toggle(title, isOn: Binding(get: { store.locksApp }, set: { store.send(.lockToggled($0)) }))
        } header: {
            Text("Privacy")
        } footer: {
            #if os(macOS)
                Text(
                    "Lifr asks when it opens, and when you come back after a minute away. Widgets show counts "
                        + "only. Apps you connected keep their access; revoke them in Connected apps.")
            #else
                Text(
                    "Lifr asks when it opens, and when you come back after a minute away. The app switcher shows "
                        + "nothing while it's locked. Widgets show counts only. Apps you connected keep their access; "
                        + "revoke them in Connected apps on the Mac.")
            #endif
        }
    }

    private var title: LocalizedStringKey {
        switch store.biometry {
        case .faceID: "Lock with Face ID"
        case .touchID: "Lock with Touch ID"
        case .opticID: "Lock with Optic ID"
        case .passcode: "Lock with passcode"
        }
    }
}

/// A row that opens a screen inside Settings, with the chevron that says so.
private struct PushButton: View {
    let title: LocalizedStringKey
    let action: () -> Void

    init(_ title: LocalizedStringKey, action: @escaping () -> Void) {
        self.title = title
        self.action = action
    }

    var body: some View {
        Button(action: action) {
            HStack {
                Text(title).foregroundStyle(Color.primary)
                Spacer()
                Image(systemName: "chevron.right")
                    .font(.footnote.bold())
                    .foregroundStyle(Color.secondary.opacity(0.6))
                    .accessibilityHidden(true)
            }
            .contentShape(.rect)
        }
        // A form tints a button and, on the Mac, draws it as a push button;
        // this is a row that opens a screen.
        .buttonStyle(.plain)
    }
}

/// The version, what happens to your data, and the open source Lifr is
/// built on.
private struct AboutSection: View {
    /// Each library Lifr ships, and its license.
    private static let libraries: [(name: String, license: String)] = [
        ("The Composable Architecture, Dependencies, Sharing and their companions", "MIT"),
        ("MLX Swift and MLX Swift LM", "MIT"),
        ("Swift Transformers, Swift Hugging Face and Swift Jinja", "Apache 2.0"),
        ("Swift Collections, Swift Numerics, Swift Crypto and Swift Argument Parser", "Apache 2.0"),
        ("EventSource", "MIT"),
        ("yyjson", "MIT"),
    ]

    let version: String

    var body: some View {
        Section {
            LabeledContent("Version", value: version)
            DisclosureGroup("Acknowledgements") {
                ForEach(Self.libraries, id: \.name) { library in
                    LabeledContent(library.name, value: library.license)
                        .font(.footnote)
                }
            }
        } header: {
            Text("About")
        } footer: {
            Text(manageFooter)
        }
    }

    private var manageFooter: String {
        #if os(iOS)
            String(
                localized: """
                    Your tasks stay on your devices and in your iCloud, encrypted end to end. To see or remove \
                    what iCloud keeps, open the Settings app, tap your name, then iCloud › Manage Account Storage.
                    """)
        #else
            String(
                localized: """
                    Your tasks stay on your devices and in your iCloud, encrypted end to end. To see or remove \
                    what iCloud keeps, open System Settings, click your name, then iCloud › Manage.
                    """)
        #endif
    }
}
