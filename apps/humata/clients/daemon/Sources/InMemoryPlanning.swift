import Foundation

extension InMemoryDaemon {
    /// The tool an agent proposes a task with, as Claude names Humata's.
    static let proposeTask = "mcp__humata__propose_task"
    static let proposeSpecEdit = "mcp__humata__propose_spec_edit"

    func propose(_ step: Step, in session: String) {
        let (tool, arguments): (String, [String: String])
        switch step {
        case .propose(let title, let spec, let parentID):
            var given = ["title": title, "spec": spec]
            if let parentID { given["parent_id"] = parentID }
            (tool, arguments) = (Self.proposeTask, given)
        case .proposeSpecEdit(let itemID, let spec):
            (tool, arguments) = (Self.proposeSpecEdit, ["item_id": itemID, "spec": spec])
        default: return
        }
        let callID = state.withValue { state in
            defer { state.nextRequest += 1 }
            return "c\(state.nextRequest)"
        }
        let json =
            (try? JSONSerialization.data(withJSONObject: arguments, options: .sortedKeys))
            .flatMap { String(data: $0, encoding: .utf8) } ?? "{}"
        append(.toolStarted(callID: callID, tool: tool, argumentsJSON: json), to: session)
        append(
            .toolCompleted(callID: callID, success: true, output: "Proposed; the person decides whether to apply it."),
            to: session)
    }

    /// The session's proposals, oldest first, from the tool calls in its
    /// history that succeeded.
    func proposals(_ session: String) throws -> [Proposal] {
        try known(session)
        return state.withValue { state in
            let history = state.histories[session] ?? []
            let succeeded = Set(
                history.compactMap { event -> String? in
                    guard case .toolCompleted(let call, true, _) = event.kind else { return nil }
                    return call
                })
            let decided = state.decisions[session] ?? [:]
            return history.compactMap { event -> Proposal? in
                guard case .toolStarted(let call, let tool, let json) = event.kind, succeeded.contains(call),
                    var proposal = Self.proposal(call: call, tool: tool, argumentsJSON: json)
                else { return nil }
                proposal.state = decided[call] ?? .pending
                // A spec edit shows the item's title when it proposes none,
                // and while pending the spec it would replace.
                if proposal.kind == .specEdit, let item = state.items.first(where: { $0.id == proposal.itemID }) {
                    if proposal.title.isEmpty { proposal.title = item.title }
                    if proposal.state == .pending { proposal.currentSpec = item.spec }
                }
                return proposal
            }
        }
    }

    private static func proposal(call: String, tool: String, argumentsJSON: String) -> Proposal? {
        let kind: Proposal.Kind
        if tool.hasSuffix("propose_task") {
            kind = .task
        } else if tool.hasSuffix("propose_spec_edit") {
            kind = .specEdit
        } else {
            return nil
        }
        guard let data = argumentsJSON.data(using: .utf8),
            let arguments = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return nil }
        func text(_ key: String) -> String {
            (arguments[key] as? String)?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        }
        let parent = text("parent_id")
        let item = text("item_id")
        let proposal = Proposal(
            id: call, kind: kind, title: text("title"), spec: arguments["spec"] as? String ?? "",
            parentID: parent.isEmpty ? nil : parent, itemID: item.isEmpty ? nil : item, note: text("note"))
        switch kind {
        case .task: return proposal.title.isEmpty ? nil : proposal
        case .specEdit: return item.isEmpty ? nil : proposal
        }
    }

    /// An outside agent proposes `proposal` for `project`'s board, as
    /// through `humata mcp`.
    package func proposeFromOutside(_ proposal: Proposal, for project: String) {
        state.withValue { $0.outside[project, default: []].append(proposal) }
    }

    func outsideProposals(_ project: String) -> [Proposal] {
        state.withValue { state in
            let decided = state.decisions["", default: [:]]
            return (state.outside[project] ?? []).map { proposal in
                var proposal = proposal
                proposal.state = decided[proposal.id] ?? .pending
                if proposal.kind == .specEdit, let item = state.items.first(where: { $0.id == proposal.itemID }) {
                    if proposal.title.isEmpty { proposal.title = item.title }
                    if proposal.state == .pending { proposal.currentSpec = item.spec }
                }
                return proposal
            }
        }
    }

    /// The project an outside agent's proposal `id` is for.
    private func outsideProject(of id: String) -> String? {
        state.value.outside.first { $0.value.contains { $0.id == id } }?.key
    }

    private func pending(_ session: String, _ proposalID: String) throws -> Proposal {
        let listed =
            if session.isEmpty {
                outsideProject(of: proposalID).map(outsideProposals) ?? []
            } else {
                try proposals(session)
            }
        guard let proposal = listed.first(where: { $0.id == proposalID }) else {
            throw DaemonError.refused(
                .unknownProposal, message: "Session \(session) proposed nothing as \(proposalID); list its proposals.")
        }
        guard proposal.state == .pending else {
            throw DaemonError.refused(.unknownProposal, message: "That proposal was already applied or declined.")
        }
        return proposal
    }

    func applyProposal(_ session: String, _ proposalID: String, title: String?, spec: String?) throws -> Item {
        let proposal = try pending(session, proposalID)
        let author: BoardAuthor = session.isEmpty ? .mcp : .session(session)
        let item: Item
        switch proposal.kind {
        case .task:
            let project =
                session.isEmpty
                ? outsideProject(of: proposalID) : state.value.sessions.first(where: { $0.id == session })?.projectID
            guard let project else {
                throw DaemonError.refused(
                    .invalidRequest, message: "Session \(session) is in no project, so its tasks have no board.")
            }
            item = try createItem(
                project,
                ItemFields(parentID: proposal.parentID, title: title ?? proposal.title, spec: spec ?? proposal.spec),
                by: author)
        case .specEdit:
            guard let itemID = proposal.itemID else { throw Self.unknownItem(proposalID) }
            let proposedSpec = proposal.spec.isEmpty ? nil : proposal.spec
            item = try updateItem(
                itemID, ItemUpdate(title: title ?? proposal.title, spec: spec ?? proposedSpec), by: author)
        }
        state.withValue { $0.decisions[session, default: [:]][proposalID] = .applied(itemID: item.id) }
        return item
    }

    func declineProposal(_ session: String, _ proposalID: String) throws {
        _ = try pending(session, proposalID)
        state.withValue { $0.decisions[session, default: [:]][proposalID] = .declined }
    }

    /// A fork of `session` working on `item`; a planning session's fork
    /// gets its own copy of the project's code.
    func startWork(_ session: String, on itemID: String) throws -> Session {
        guard let item = state.value.items.first(where: { $0.id == itemID }) else { throw Self.unknownItem(itemID) }
        var fork = try fork(session, before: nil, name: item.title, chatOnly: false)
        return state.withValue { state in
            fork.itemID = itemID
            if fork.planning, let project = state.projects.first(where: { $0.id == item.projectID }) {
                fork.planning = false
                fork.workingDirectory = Self.copy(of: project, for: fork.id)
            }
            if let index = state.sessions.firstIndex(where: { $0.id == fork.id }) { state.sessions[index] = fork }
            return fork
        }
    }

    func updateProject(_ projectID: String, _ update: ProjectUpdate) throws -> Project {
        let repositories = try update.repositories.map(Self.named)
        return try state.withValue { state in
            guard let index = state.projects.firstIndex(where: { $0.id == projectID }) else {
                throw DaemonError.refused(.unknownProject, message: "There is no project \(projectID).")
            }
            if let name = update.name?.trimmingCharacters(in: .whitespacesAndNewlines) {
                guard !name.isEmpty else {
                    throw DaemonError.refused(.invalidRequest, message: "A project needs a name.")
                }
                state.projects[index].name = name
            }
            if let instructions = update.instructions { state.projects[index].instructions = instructions }
            if let repositories { state.projects[index].repositories = repositories }
            if let agentSessions = update.agentSessions { state.projects[index].agentSessions = agentSessions }
            if let skills = update.skills {
                state.projects[index].skills = skills.compactMap { skill in
                    let name = skill.name.trimmingCharacters(in: .whitespacesAndNewlines)
                    let text = skill.text.trimmingCharacters(in: .whitespacesAndNewlines)
                    return name.isEmpty || text.isEmpty ? nil : Skill(name: name, text: text)
                }
            }
            return state.projects[index]
        }
    }
}
