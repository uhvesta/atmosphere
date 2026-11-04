import ComposableArchitecture
import Foundation
import LifrAPI
import LifrAssistantClient
import LifrCore
import LifrSessionClient
import LifrSkills
import LifrStoreClient
import LifrTaskDetail

/// One Avenue's agent access, as Settings shows it.
package struct AvenueAccess: Identifiable, Hashable, Sendable {
    package var id: UUID
    package var name: String
    package var access: AgentAccess
}

extension AgentAccess {
    package var title: String {
        switch self {
        case .full: String(localized: "Read and write")
        case .readOnly: String(localized: "Read only")
        case .hidden: String(localized: "Hidden")
        }
    }
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
        /// Whether to show agent access, connected apps and the CLI. Only
        /// the Mac runs the CLI.
        package var showsAgents: Bool
        package var procrastinatedAfterDays = Settings().procrastinatedAfterDays
        package var avenues: [AvenueAccess] = []
        package var apps: [ConnectedApp] = []
        package var cli = CLIInstallation.notInstalled
        package var launchesAtLogin = false
        /// Kept so the Activity log opens on what's shown.
        package var workspace = Workspace()
        @Presents package var activity: ActivityFeature.State?
        @Presents package var skills: SkillsFeature.State?
        @Presents package var revoke: ConfirmationDialogState<Revoke>?
        @Presents package var alert: AlertState<Never>?

        package init(showsAgents: Bool) {
            self.showsAgents = showsAgents
        }

        package mutating func show(_ workspace: Workspace) {
            self.workspace = workspace
            activity?.workspace = workspace
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
        case event(SessionEvent)
        case procrastinatedDaysChanged(Int)
        case agentAccessPicked(UUID, AgentAccess)
        case writePicked(String, Bool)
        case renewTapped(String)
        case revokeTapped(String)
        case installCLITapped
        case activityTapped
        case activity(PresentationAction<ActivityFeature.Action>)
        case skillsTapped
        case skills(PresentationAction<SkillsFeature.Action>)
        case loginItemFailed(any Error)
        case saved(Result<Saved, any Error>)
        case sessionChanged(Result<Void, any Error>)
        case revoke(PresentationAction<Revoke>)
        case alert(PresentationAction<Never>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate {
            case saved(Workspace)
        }
    }

    @Dependency(\.store) private var store
    @Dependency(\.session) private var session
    @Dependency(\.date.now) private var now

    package init() {}

    package var body: some ReducerOf<Self> {
        Reduce { state, action in
            switch action {
            case .task:
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
                state.show(workspace)
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

            case .skillsTapped:
                state.skills = SkillsFeature.State()
                return .none

            case .activity(.presented(.delegate(.saved(let workspace)))):
                state.show(workspace)
                return .send(.delegate(.saved(workspace)))

            case .saved(.success(let saved)):
                state.show(saved.workspace)
                return .send(.delegate(.saved(saved.workspace)))

            case .loaded(.failure(let error)):
                state.alert = .failed("Couldn't load your settings", error)
                return .none

            case .saved(.failure(let error)):
                state.alert = .failed("Couldn't change the setting", error)
                return .none

            case .appsLoaded(.failure(let error)), .sessionChanged(.failure(let error)):
                state.alert = .failed("Couldn't change connected apps", error)
                return .none

            case .sessionChanged(.success), .activity, .skills, .revoke, .alert, .delegate:
                return .none
            }
        }
        .ifLet(\.$activity, action: \.activity) { ActivityFeature() }
        .ifLet(\.$skills, action: \.skills) { SkillsFeature() }
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
