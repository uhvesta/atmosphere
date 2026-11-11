import ChatFeatures
import ComposableArchitecture
import Foundation
import LifrAPI
import LifrAssistantClient
import LifrBugReport
import LifrCore
import LifrLockClient
import LifrSessionClient
import LifrSkills
import LifrStoreClient
import LifrSyncClient
import LifrTaskDetail
import ModelsFeatures

/// One Avenue's agent access, as Settings shows it.
package struct AvenueAccess: Identifiable, Hashable, Sendable {
    package var id: UUID
    package var name: String
    package var access: AgentAccess
}

extension ConnectedApp {
    /// Whether the grant lets the app make changes.
    package var canWrite: Bool { grant.scopes.contains(.write) }
}

/// Settings. On the Mac it also covers agents: per-Avenue access, the apps
/// you approved, and the CLI.
@Reducer
package struct SettingsFeature {
    @ObservableState
    package struct State: Equatable {
        /// The latest save shown, so an earlier one's late reply is ignored.
        package var saves = SaveOrder()
        /// Whether to show agent access, connected apps and the CLI. Only
        /// the Mac runs the CLI.
        package var showsAgents: Bool
        package var procrastinatedAfterDays = Settings().procrastinatedAfterDays
        package var avenues: [AvenueAccess] = []
        package var apps: [ConnectedApp] = []
        package var cli = CLIInstallation.notInstalled
        package var launchesAtLogin = false
        /// What the lock confirms with on this device.
        package var biometry = Biometry.faceID
        /// Whether Lifr locks when it opens. Changed only once the person
        /// confirms it's them.
        @Shared(.locksApp) package var locksApp
        /// The chat's experiments, kept on this device and off until turned
        /// on.
        @Shared(.chatShowsSpeed) package var showsSpeed
        @Shared(.chatShowsPrompt) package var showsPrompt
        @Shared(.chatCompares) package var compares
        /// Whether sync runs often while open. The app restarts its timer
        /// when this changes.
        @Shared(.syncsContinuously) package var syncsContinuously
        /// Where sync is. The app passes each change on.
        package var sync = SyncState()
        /// Kept so the Activity log opens on what's shown.
        package var workspace = Workspace()
        @Presents package var activity: ActivityFeature.State?
        @Presents package var labels: LabelsFeature.State?
        @Presents package var skills: SkillsFeature.State?
        @Presents package var instructions: InstructionsFeature.State?
        @Presents package var models: ModelsFeature.State?
        @Presents package var bugReports: BugReportsFeature.State?
        @Presents package var revoke: ConfirmationDialogState<Revoke>?
        @Presents package var alert: AlertState<Never>?

        package init(showsAgents: Bool) {
            self.showsAgents = showsAgents
        }

        /// Shows `snapshot`'s records, here and in Activity, unless later
        /// ones are shown already.
        package mutating func show(_ snapshot: Snapshot) {
            guard saves.isLatest(snapshot) else { return }
            let workspace = snapshot.workspace
            self.workspace = workspace
            activity?.show(snapshot)
            labels?.show(snapshot)
            procrastinatedAfterDays = workspace.settings.procrastinatedAfterDays
            avenues = workspace.avenues.values
                .filter { $0.archivedAt == nil }
                .sorted { ($0.sortOrder, $0.id.uuidString) < ($1.sortOrder, $1.id.uuidString) }
                .map { AvenueAccess(id: $0.id, name: $0.name, access: $0.agentAccess) }
        }
    }

    package enum Revoke: Equatable, Sendable {
        case confirmed(String)
    }

    package enum Action {
        case task
        case loaded(Result<Workspace, any Error>)
        case appsLoaded(Result<[ConnectedApp], any Error>)
        case cliLoaded(CLIInstallation)
        case launchAtLoginLoaded(Bool)
        case launchAtLoginToggled(Bool)
        /// The lock's switch: asks the person to confirm before it changes.
        case lockToggled(Bool)
        case lockConfirmed(Bool, Result<Void, any Error>)
        case event(SessionEvent)
        case procrastinatedDaysChanged(Int)
        case agentAccessPicked(UUID, AgentAccess)
        case writePicked(String, Bool)
        case renewTapped(String)
        case revokeTapped(String)
        case installCLITapped
        case activityTapped
        case syncNowTapped
        case syncContinuouslyToggled(Bool)
        case activity(PresentationAction<ActivityFeature.Action>)
        case labelsTapped
        case labels(PresentationAction<LabelsFeature.Action>)
        case skillsTapped
        case skills(PresentationAction<SkillsFeature.Action>)
        case instructionsTapped
        case instructions(PresentationAction<InstructionsFeature.Action>)
        case modelsTapped
        case models(PresentationAction<ModelsFeature.Action>)
        case bugReportsTapped
        case bugReports(PresentationAction<BugReportsFeature.Action>)
        case loginItemFailed(any Error)
        case saved(Result<Saved, any Error>)
        case sessionChanged(Result<Void, any Error>)
        case revoke(PresentationAction<Revoke>)
        case alert(PresentationAction<Never>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate {
            case saved(Snapshot)
        }
    }

    @Dependency(\.store) private var store
    @Dependency(\.session) private var session
    @Dependency(\.sync) private var sync
    @Dependency(\.date.now) private var now
    @Dependency(\.lock) private var lock

    package init() {}

    package var body: some ReducerOf<Self> {
        Reduce { (state: inout State, action: Action) -> Effect<Action> in
            switch action {
            case .task:
                state.biometry = lock.biometry()
                let loadStore: Effect<Action> = .run { [store] send in
                    await send(.loaded(Result { try await store.load() }))
                }
                guard state.showsAgents else { return loadStore }
                return .merge(
                    loadStore,
                    .run { [session] send in
                        await send(.appsLoaded(Result { try await session.apps() }))
                        await send(.cliLoaded(await session.cliInstallation()))
                        await send(.launchAtLoginLoaded(await session.launchesAtLogin()))
                        for await event in await session.events() {
                            await send(.event(event))
                        }
                    }
                )

            case .loaded(.success(let workspace)):
                state.show(Snapshot(workspace, number: 0))
                return .none

            case .appsLoaded(.success(let apps)), .event(.appsChanged(let apps)):
                state.apps = apps
                return .none

            case .cliLoaded(let cli):
                state.cli = cli
                return .none

            case .launchAtLoginLoaded(let enabled):
                state.launchesAtLogin = enabled
                return .none

            case .launchAtLoginToggled(let enabled):
                state.launchesAtLogin = enabled
                return .run { [session] send in
                    do {
                        try await session.setLaunchesAtLogin(enabled)
                    } catch {
                        await send(.launchAtLoginLoaded(await session.launchesAtLogin()))
                        await send(.loginItemFailed(error))
                    }
                }

            case .lockToggled(let on):
                guard on != state.locksApp else { return .none }
                let reason =
                    on
                    ? String(localized: "Lock Lifr when it opens") : String(localized: "Stop locking Lifr")
                return .run { [lock] send in
                    await send(.lockConfirmed(on, Result { try await lock.authenticate(reason) }))
                }

            case .lockConfirmed(let on, .success):
                state.$locksApp.withLock { $0 = on }
                return .none

            case .lockConfirmed(_, .failure(let error)):
                guard (error as? LockError) != .cancelled else { return .none }
                state.alert = .failed("Couldn't change the lock", error)
                return .none

            case .loginItemFailed(let error):
                state.alert = .failed("Couldn't change Launch at login", error)
                return .none

            case .event:
                return .none

            case .procrastinatedDaysChanged(let days):
                let days = min(max(days, 1), 90)
                state.procrastinatedAfterDays = days
                return .run { [store] send in
                    await send(.saved(Result { try await store.setSettings(Settings(procrastinatedAfterDays: days)) }))
                }

            case .agentAccessPicked(let id, let access):
                return .run { [store] send in
                    await send(.saved(Result { try await store.setAgentAccess(access, ofAvenue: id) }))
                }

            case .writePicked(let id, let canWrite):
                guard var grant = state.apps.first(where: { $0.id == id })?.grant else { return .none }
                grant.scopes = canWrite ? [.read, .write] : [.read]
                return update(id, grant)

            case .renewTapped(let id):
                guard var grant = state.apps.first(where: { $0.id == id })?.grant else { return .none }
                grant.expiresAt = Grant.agentDefault(client: grant.client, now: now).expiresAt
                return update(id, grant)

            case .revokeTapped(let id):
                guard let app = state.apps.first(where: { $0.id == id }) else { return .none }
                state.revoke = ConfirmationDialogState {
                    TextState("Revoke \(app.name)?")
                } actions: {
                    ButtonState(role: .destructive, action: .confirmed(id)) { TextState("Revoke") }
                    ButtonState(role: .cancel) { TextState("Cancel") }
                } message: {
                    TextState("Its next request is refused. It can ask again, and you'd approve it anew.")
                }
                return .none

            case .revoke(.presented(.confirmed(let id))):
                return .run { [session] send in
                    await send(.sessionChanged(Result { try await session.revoke(id) }))
                    await send(.appsLoaded(Result { try await session.apps() }))
                }

            case .installCLITapped:
                return .run { [session] send in
                    do {
                        await send(.cliLoaded(try await session.installCLI()))
                    } catch {
                        await send(.sessionChanged(.failure(error)))
                    }
                }

            case .activityTapped:
                state.activity = ActivityFeature.State(workspace: state.workspace)
                return .none

            case .labelsTapped:
                state.labels = LabelsFeature.State(workspace: state.workspace)
                return .none

            case .labels(.presented(.delegate(.saved(let snapshot)))):
                state.show(snapshot)
                return .send(.delegate(.saved(snapshot)))

            case .syncNowTapped:
                guard state.sync.status != .syncing else { return .none }
                return .run { [sync] _ in _ = await sync.sync() }

            case .syncContinuouslyToggled(let on):
                state.$syncsContinuously.withLock { $0 = on }
                return .none

            case .modelsTapped:
                state.models = ModelsFeature.State()
                return .none

            case .bugReportsTapped:
                state.bugReports = BugReportsFeature.State()
                return .none

            case .skillsTapped:
                state.skills = SkillsFeature.State()
                return .none

            case .instructionsTapped:
                state.instructions = InstructionsFeature.State()
                return .none

            case .activity(.presented(.delegate(.saved(let snapshot)))):
                state.show(snapshot)
                return .send(.delegate(.saved(snapshot)))

            case .saved(.success(let saved)):
                guard saved.number >= state.saves.latest else { return .none }
                state.show(saved.snapshot)
                return .send(.delegate(.saved(saved.snapshot)))

            case .loaded(.failure(let error)):
                state.alert = .failed("Couldn't load your settings", error)
                return .none

            case .saved(.failure(let error)):
                state.alert = .failed("Couldn't change the setting", error)
                return .none

            case .appsLoaded(.failure(let error)), .sessionChanged(.failure(let error)):
                state.alert = .failed("Couldn't change connected apps", error)
                return .none

            case .sessionChanged(.success), .activity, .labels, .skills, .instructions, .models, .bugReports, .revoke,
                .alert, .delegate:
                return .none
            }
        }
        .ifLet(\.$activity, action: \.activity) { ActivityFeature() }
        .ifLet(\.$labels, action: \.labels) { LabelsFeature() }
        .ifLet(\.$skills, action: \.skills) { SkillsFeature() }
        .ifLet(\.$instructions, action: \.instructions) { InstructionsFeature() }
        .ifLet(\.$models, action: \.models) { ModelsFeature() }
        .ifLet(\.$bugReports, action: \.bugReports) { BugReportsFeature() }
        .ifLet(\.$revoke, action: \.revoke)
        .ifLet(\.$alert, action: \.alert)
    }

    private func update(_ id: String, _ grant: Grant) -> Effect<Action> {
        .run { [session] send in
            await send(.sessionChanged(Result { try await session.update(id, grant) }))
            await send(.appsLoaded(Result { try await session.apps() }))
        }
    }
}
