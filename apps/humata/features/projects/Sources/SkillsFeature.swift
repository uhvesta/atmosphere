import ComposableArchitecture
import Foundation
import HumataDaemonClient
import SwiftUI

/// The Settings window's tabs, kept so another screen can open it at one.
package enum SettingsTab {
    package static let key = "settingsTab"
    package static let environments = "environments"
    package static let approvals = "approvals"
    package static let storage = "storage"
    package static let skills = "skills"
    package static let plugins = "plugins"
    package static let models = "models"
}

/// Settings › Skills: every skill on the Mac and each host, in the agents'
/// own folders and the projects' repositories, with each project's own;
/// and every clash, two files of one name that one session would get,
/// with which one wins. Looking on the hosts can take a while, so it
/// shows a spinner meanwhile.
@Reducer
package struct SkillsFeature {
    @ObservableState
    package struct State: Equatable {
        /// Nil while the Mac and hosts are looked through.
        package var list: SkillList?
        /// Project names by id, to group by.
        package var projects: [String: String] = [:]
        /// The clashes whose pick is being saved.
        package var saving: Set<SkillClash.ID> = []
        /// Why they couldn't be listed or the pick saved, and what to do.
        package var failure: String?

        package init(list: SkillList? = nil) {
            self.list = list
        }

        /// Clashes nobody has picked for yet, which keep sessions from
        /// starting.
        package var unpicked: Int { list?.clashes.filter { $0.winner == nil }.count ?? 0 }
    }

    package enum Action {
        case task
        case refreshTapped
        case loaded(SkillList, [Project])
        case failed(String)
        case picked(SkillClash.ID, winner: String)
        case pickSaved(SkillClash.ID)
        case pickFailed(SkillClash.ID, String)
        case showInFinderTapped(String)
    }

    @Dependency(\.daemon) var daemon
    @Dependency(\.openURL) var openURL

    private enum CancelID { case load }

    package init() {}

    /// Lists them again; a failure on screen stays until the next.
    private func load() -> Effect<Action> {
        .run { send in
            async let projects = daemon.projects()
            let list = try await daemon.skills()
            await send(.loaded(list, try await projects))
        } catch: { error, send in
            await send(.failed(error.localizedDescription))
        }
        .cancellable(id: CancelID.load, cancelInFlight: true)
    }

    package var body: some ReducerOf<Self> {
        Reduce { state, action in
            switch action {
            case .task, .refreshTapped:
                state.list = nil
                state.failure = nil
                return load()

            case .loaded(let list, let projects):
                state.list = list
                state.projects = Dictionary(projects.map { ($0.id, $0.name) }, uniquingKeysWith: { first, _ in first })
                return .none

            case .failed(let message):
                state.failure = message
                state.list = state.list ?? SkillList()
                return .none

            case .picked(let id, let winner):
                guard let index = state.list?.clashes.firstIndex(where: { $0.id == id }),
                    let clash = state.list?.clashes[index]
                else { return .none }
                state.list?.clashes[index].winner = winner
                state.saving.insert(id)
                state.failure = nil
                return .run { send in
                    try await daemon.pickSkill(clash.name, clash.candidates.map(\.hash), winner)
                    await send(.pickSaved(id))
                } catch: { error, send in
                    await send(.pickFailed(id, error.localizedDescription))
                }

            case .pickSaved(let id):
                state.saving.remove(id)
                return .none

            case .pickFailed(let id, let message):
                state.saving.remove(id)
                state.failure = message
                return load()

            case .showInFinderTapped(let path):
                let url = URL(fileURLWithPath: path).deletingLastPathComponent()
                return .run { _ in await openURL(url) }
            }
        }
    }
}

/// Where a skill is, for the person to find it.
extension FoundSkill {
    package var place: String {
        let at: String =
            switch origin {
            case .claudeUser: "~/.claude/skills"
            case .copilotUser: "~/.copilot/skills"
            case .claudeRepository: "\(repository)/.claude/skills"
            case .githubRepository: "\(repository)/.github/skills"
            case .project: "The project's own"
            }
        return host.isEmpty ? at : "\(at) on \(host)"
    }

    /// Which agent loads it of itself, if one does.
    package var agent: String? {
        switch origin {
        case .claudeUser, .claudeRepository: "Claude Code"
        case .copilotUser, .githubRepository: "Copilot"
        case .project: nil
        }
    }
}

package struct SkillsView: View {
    let store: StoreOf<SkillsFeature>

    package init(store: StoreOf<SkillsFeature>) {
        self.store = store
    }

    package var body: some View {
        Group {
            if let list = store.list {
                content(list)
            } else {
                VStack(spacing: 8) {
                    ProgressView()
                    Text("Looking for skills on this Mac and each host…")
                        .foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .toolbar {
            Button("Look Again", systemImage: "arrow.clockwise") { store.send(.refreshTapped) }
                .disabled(store.list == nil)
        }
        .task { await store.send(.task).finish() }
    }

    private func content(_ list: SkillList) -> some View {
        Form {
            if let failure = store.failure {
                Section {
                    Label(failure, systemImage: "exclamationmark.triangle.fill")
                        .foregroundStyle(.red)
                }
            }
            if !list.clashes.isEmpty {
                Section {
                    ForEach(list.clashes) { clash in
                        clashRow(clash)
                    }
                } header: {
                    Text("Same Name, Different Files")
                } footer: {
                    Text(
                        "One session would get each of these. Pick which it reads; the pick holds for every "
                            + "project and host until either file changes. A session can't start while one is unpicked."
                    )
                    .font(.caption)
                    .foregroundStyle(.secondary)
                }
            }
            ForEach(groups(list), id: \.title) { group in
                Section(group.title) {
                    ForEach(group.skills) { skill in
                        skillRow(skill)
                    }
                }
            }
            if list.skills.isEmpty {
                Section {
                    Text(
                        "No skills yet. Put a folder with a SKILL.md in ~/.claude/skills, ~/.copilot/skills, or a "
                            + "repository's .claude/skills or .github/skills, or add one to a project in its settings."
                    )
                    .foregroundStyle(.secondary)
                }
            }
            if !list.unreachable.isEmpty {
                Section {
                    Text(
                        "Couldn't reach \(list.unreachable.formatted(.list(type: .and))), so its skills aren't listed. "
                            + "Check the host in Environments, then look again."
                    )
                    .font(.caption)
                    .foregroundStyle(.secondary)
                }
            }
        }
        .formStyle(.grouped)
    }

    private func clashRow(_ clash: SkillClash) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text(clash.name).font(.headline)
                if clash.winner == nil {
                    Text("Pick one")
                        .font(.caption.bold())
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(.yellow.opacity(0.3), in: Capsule())
                }
                Spacer()
                if store.saving.contains(clash.id) {
                    ProgressView().controlSize(.small)
                }
            }
            Picker(
                clash.name,
                selection: Binding(
                    get: { clash.winner },
                    set: { winner in
                        if let winner { store.send(.picked(clash.id, winner: winner)) }
                    })
            ) {
                ForEach(clash.candidates, id: \.hash) { skill in
                    VStack(alignment: .leading) {
                        Text(skill.place)
                        if !skill.description.isEmpty {
                            Text(skill.description).font(.caption).foregroundStyle(.secondary)
                        }
                    }
                    .tag(String?.some(skill.hash))
                }
            }
            .pickerStyle(.radioGroup)
            .labelsHidden()
        }
        .padding(.vertical, 2)
    }

    private func skillRow(_ skill: FoundSkill) -> some View {
        HStack(alignment: .firstTextBaseline) {
            VStack(alignment: .leading, spacing: 2) {
                Text(skill.name)
                if !skill.description.isEmpty {
                    Text(skill.description)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                }
                Text(skill.place)
                    .font(.caption2)
                    .foregroundStyle(.tertiary)
            }
            Spacer()
            if let agent = skill.agent {
                Text(agent)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            if skill.host.isEmpty, !skill.path.isEmpty {
                Button("Show in Finder", systemImage: "folder") { store.send(.showInFinderTapped(skill.path)) }
                    .labelStyle(.iconOnly)
                    .buttonStyle(.borderless)
                    .help("Show in Finder")
            }
        }
    }

    /// The skills by where a session gets them: each host's own, then each
    /// project's, its own and its repositories' on every host.
    private func groups(_ list: SkillList) -> [(title: String, skills: [FoundSkill])] {
        var groups: [(title: String, skills: [FoundSkill])] = []
        let hosts = Set(list.skills.filter { $0.projectID.isEmpty }.map(\.host)).sorted()
        for host in hosts {
            let theirs = list.skills.filter { $0.projectID.isEmpty && $0.host == host }
            groups.append((host.isEmpty ? "On This Mac" : "On \(host)", theirs))
        }
        let projects = Set(list.skills.map(\.projectID).filter { !$0.isEmpty })
            .sorted { (store.projects[$0] ?? $0) < (store.projects[$1] ?? $1) }
        for project in projects {
            let theirs = list.skills.filter { $0.projectID == project }
            groups.append(("In \(store.projects[project] ?? project)", theirs))
        }
        return groups
    }
}
