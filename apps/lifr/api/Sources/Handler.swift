import Foundation
import LifrCore

/// What the app must ask the person before a request runs.
package enum Confirmation: Hashable, Sendable {
    /// The grant confirms writes: a notification to approve or deny.
    case write(APIMethod, task: String?)
    /// `trash_task`, always: these tasks go to the trash.
    case trash([String])
    /// Touch ID for a terminal session.
    case unlock
    /// Touch ID before every record is written out.
    case export
}

/// Runs one request against the workspace. The session server (M9) owns the
/// socket, the grants and Touch ID; this is everything after that, so the
/// CLI, MCP and tests get the same answers.
package struct Handler {
    /// The grant of the calling client, or `nil` when it isn't approved yet.
    package var grant: Grant?
    /// Who the change is by, when, and where IDs come from. The actor is
    /// replaced with the grant's.
    package var context: Context
    /// Asks the person; `true` when they approve.
    package var confirm: (Confirmation) -> Bool

    package init(grant: Grant?, context: Context, confirm: @escaping (Confirmation) -> Bool) {
        self.grant = grant
        self.context = context
        self.confirm = confirm
        if let grant { self.context.actor = grant.actor(device: context.actor.device) }
    }

    /// Answers `request`. Never throws: every failure is an error response.
    package func handle(_ request: RPCRequest, in workspace: inout Workspace) -> RPCResponse {
        do {
            guard let method = APIMethod(rawValue: request.method) else {
                throw APIError.methodNotFound(request.method)
            }
            return try .success(id: request.id, run(method, request.params ?? .object([:]), &workspace))
        } catch let error as APIError {
            return .failure(id: request.id, error)
        } catch let error as LifrError {
            return .failure(id: request.id, apiError(error, workspace))
        } catch {
            return .failure(
                id: request.id,
                APIError(
                    .internalError,
                    "Lifr hit an unexpected error: \(error). Nothing changed. Retry, and report it if it keeps "
                        + "happening."
                ))
        }
    }

    /// Answers a line of JSON from the socket, including ones that don't parse.
    package func handle(line: Data, in workspace: inout Workspace) -> RPCResponse {
        guard let value = try? Wire.decoder.decode(JSONValue.self, from: line) else {
            return .failure(
                id: nil,
                APIError(
                    .parseError,
                    "The request isn't valid JSON. Send one JSON-RPC 2.0 object per line."))
        }
        guard let request = try? value.decode(RPCRequest.self), request.jsonrpc == "2.0" else {
            var id: Int?
            if case .object(let fields) = value, case .number(let number)? = fields["id"] { id = Int(exactly: number) }
            return .failure(
                id: id,
                APIError(
                    .invalidRequest,
                    "The request isn't a JSON-RPC 2.0 request. It needs \"jsonrpc\": \"2.0\", an integer \"id\" and "
                        + "a \"method\"."
                ))
        }
        return handle(request, in: &workspace)
    }

    // MARK: - Methods

    private func run(_ method: APIMethod, _ params: JSONValue, _ workspace: inout Workspace) throws -> any Encodable {
        try authorize(method)
        switch method {
        case .hello:
            let hello: Hello.Params = try decode(params, for: method)
            guard hello.protocolVersion == LifrProtocol.version else {
                throw APIError.versionMismatch(client: hello.protocolVersion, app: LifrProtocol.version)
            }
            return Hello.Result(
                protocolVersion: LifrProtocol.version,
                scopes: grant.map { $0.scopes.sorted() } ?? [], expiresAt: grant?.expiresAt)

        case .unlock:
            guard confirm(.unlock) else { throw APIError.confirmationDenied }
            return Unlock.Result(expiresAt: context.now.addingTimeInterval(Unlock.idleTimeout))

        case .listAvenues:
            return visibleAvenues(workspace).map {
                AvenueSummary(
                    key: $0.key.rawValue, name: $0.name, agentAccess: $0.agentAccess,
                    needsRename: $0.key.needsRename)
            }

        case .listProjects:
            let list: ListProjects.Params = try decode(params, for: method)
            let avenues = try list.avenue.map { [try avenue(key: $0, workspace)] } ?? visibleAvenues(workspace)
            let ids = Set(avenues.map(\.id))
            return workspace.projects.values
                .filter { ids.contains($0.avenueID) && (list.includeClosed == true || $0.status == .active) }
                .sorted { ($0.sortOrder, $0.id.uuidString) < ($1.sortOrder, $1.id.uuidString) }
                .map {
                    ProjectSummary(
                        id: $0.id, avenue: workspace.avenues[$0.avenueID]!.key.rawValue,
                        name: $0.name, status: $0.status, dueDate: $0.dueDate)
                }

        case .searchTasks:
            return try search(decode(params, for: method), workspace)

        case .getTask:
            let ref: TaskRef.Params = try decode(params, for: method)
            return detail(try task(ref.task, workspace), workspace)

        case .createTask:
            let create: CreateTask.Params = try decode(params, for: method)
            var new = Workspace.NewTask(title: create.title)
            if let parent = create.parent {
                let parent = try task(parent, workspace, writing: true)
                new.parentID = parent.id
            } else {
                guard let key = create.avenue else {
                    throw APIError.invalidParams(
                        method.rawValue, "give \"avenue\" (a key such as HEALTH), or \"parent\" for a Sub task.")
                }
                new.avenueID = try avenue(key: key, workspace, writing: true).id
                new.projectID = create.project
            }
            new.details = create.details ?? ""
            new.status = create.status ?? .todo
            new.priority = create.priority ?? .none
            new.startDate = create.startDate
            new.dueDate = create.dueDate
            new.dueTime = create.dueTime
            new.labelIDs = try labelIDs(create.labels ?? [], workspace)
            try confirmWrite(method, task: nil)
            let change = try workspace.createTask(new, in: context)
            let created = change.previous.first { $0.value == nil }!.key
            return result(of: change, for: created, workspace)

        case .updateTask:
            let update: UpdateTask.Params = try decode(params, for: method)
            let target = try task(update.task, workspace, writing: true)
            let clear = Set(update.clear ?? [])
            let edit = Workspace.TaskEdit(
                title: update.title, details: update.details, priority: update.priority,
                startDate: clear.contains(.startDate) ? .some(nil) : update.startDate.map { $0 },
                dueDate: clear.contains(.dueDate) ? .some(nil) : update.dueDate.map { $0 },
                dueTime: clear.contains(.dueTime) ? .some(nil) : update.dueTime.map { $0 },
                labelIDs: try update.labels.map { try labelIDs($0, workspace) })
            try confirmWrite(method, task: update.task)
            return result(of: try workspace.edit(target.id, edit, in: context), for: target.id, workspace)

        case .setStatus:
            let set: SetStatus.Params = try decode(params, for: method)
            let target = try task(set.task, workspace, writing: true)
            try confirmWrite(method, task: set.task)
            let change = try workspace.setStatus(set.status, of: target.id, cascade: set.cascade ?? false, in: context)
            return result(of: change, for: target.id, workspace)

        case .moveTask:
            let move: MoveTask.Params = try decode(params, for: method)
            let target = try task(move.task, workspace, writing: true)
            let destinations = [move.avenue != nil, move.parent != nil, move.topLevel == true].filter { $0 }.count
            guard destinations == 1 else {
                throw APIError.invalidParams(
                    method.rawValue,
                    "give exactly one of \"avenue\" (with an optional \"project\"), \"parent\", or \"topLevel\": true.")
            }
            let change: Change
            if let key = move.avenue {
                let avenue = try avenue(key: key, workspace, writing: true)
                try confirmWrite(method, task: move.task)
                change = try workspace.move(target.id, toAvenue: avenue.id, project: move.project, in: context)
            } else if let parent = move.parent {
                let parent = try task(parent, workspace, writing: true)
                try confirmWrite(method, task: move.task)
                change = try workspace.setParent(parent.id, of: target.id, in: context)
            } else {
                try confirmWrite(method, task: move.task)
                change = try workspace.setParent(nil, of: target.id, in: context)
            }
            return result(of: change, for: target.id, workspace)

        case .addComment:
            let add: AddComment.Params = try decode(params, for: method)
            let target = try task(add.task, workspace, writing: true)
            try confirmWrite(method, task: add.task)
            let comment = try workspace.addComment(add.body, to: target.id, in: context)
            return AddComment.Result(
                id: comment.id, task: workspace.displayCode(of: target), createdAt: comment.createdAt)

        case .trashTask:
            let ref: TaskRef.Params = try decode(params, for: method)
            let target = try task(ref.task, workspace, writing: true)
            let trashed = [target] + workspace.subtasks(of: target.id)
            let codes = trashed.map { workspace.displayCode(of: $0) }
            guard confirm(.trash(codes)) else { throw APIError.confirmationDenied }
            _ = try workspace.moveToTrash(target.id, in: context)
            return TrashTask.Result(trashed: codes)

        case .export:
            guard confirm(.export) else { throw APIError.confirmationDenied }
            func sorted<T: Identifiable>(_ records: [UUID: T]) -> [T] where T.ID == UUID {
                records.values.sorted { $0.id.uuidString < $1.id.uuidString }
            }
            return Export(
                protocolVersion: LifrProtocol.version, avenues: sorted(workspace.avenues),
                projects: sorted(workspace.projects), tasks: sorted(workspace.tasks),
                labels: sorted(workspace.labels), comments: sorted(workspace.comments),
                reminders: sorted(workspace.reminders), history: workspace.history,
                settings: workspace.settings)
        }
    }

    // MARK: - Access

    private var isAgent: Bool { context.actor.kind == .agent }

    private func authorize(_ method: APIMethod) throws {
        if method == .export, grant?.isTerminalSession != true {
            throw APIError.notAuthorized(
                "Only a terminal session can export. Run `lifr unlock` in your terminal, approve with Touch ID, then "
                    + "run `lifr export` again."
            )
        }
        guard let scope = method.scope else { return }
        guard let grant else {
            throw APIError.notAuthorized(
                "This app isn't approved to use Lifr. Approve it in the sheet the Lifr app shows on this Mac, or run "
                    + "`lifr unlock` in a terminal."
            )
        }
        guard grant.expiresAt > context.now else {
            throw APIError.notAuthorized(
                grant.isTerminalSession
                    ? "The terminal session ended after 15 idle minutes. Run `lifr unlock` again."
                    : "\(grant.client)'s access expired. Approve it again from the sheet the Lifr app shows, or "
                        + "extend it in Settings > Connected apps."
            )
        }
        guard grant.scopes.contains(scope) else {
            throw APIError.notAuthorized(
                "\(grant.client) has \(grant.scopes.contains(.read) ? "read-only" : "no read") access. To let it "
                    + "\(scope == .write ? "make changes" : "read tasks"), open Settings > Connected apps > "
                    + "\(grant.client) and choose Read and write."
            )
        }
    }

    private func confirmWrite(_ method: APIMethod, task: String?) throws {
        guard grant?.confirmWrites == true else { return }
        guard confirm(.write(method, task: task)) else { throw APIError.confirmationDenied }
    }

    /// Avenues this caller may see, in sidebar order. Hidden ones never show
    /// up for agents, and a grant can narrow the list further.
    private func visibleAvenues(_ workspace: Workspace) -> [Avenue] {
        workspace.avenues.values
            .filter(isVisible)
            .sorted { ($0.sortOrder, $0.id.uuidString) < ($1.sortOrder, $1.id.uuidString) }
    }

    private func isVisible(_ avenue: Avenue) -> Bool {
        guard isAgent else { return true }
        return avenue.agentAccess != .hidden && (grant?.avenueIDs?.contains(avenue.id) ?? true)
    }

    private func checkWritable(_ avenue: Avenue) throws {
        guard isAgent, avenue.agentAccess == .readOnly else { return }
        throw APIError.notAuthorized(
            "\(avenue.name) is read-only for agents. To allow changes, set its agent access to Full in the Avenue's "
                + "settings."
        )
    }

    /// Hidden and out-of-grant Avenues answer `notFound`, so an agent can't
    /// tell they exist.
    private func avenue(key: String, _ workspace: Workspace, writing: Bool = false) throws -> Avenue {
        let key = try AvenueKey(key)
        guard let avenue = workspace.avenues.values.first(where: { $0.key == key }), isVisible(avenue) else {
            throw LifrError.notFound(.avenue, key.rawValue)
        }
        if writing { try checkWritable(avenue) }
        return avenue
    }

    private func task(_ code: String, _ workspace: Workspace, writing: Bool = false) throws -> LifrTask {
        let task = try workspace.task(code: code)
        guard task.deletedAt == nil, let avenue = workspace.avenues[task.avenueID], isVisible(avenue) else {
            throw LifrError.notFound(.task, code)
        }
        if writing { try checkWritable(avenue) }
        return task
    }

    private func labelIDs(_ names: [String], _ workspace: Workspace) throws -> Set<UUID> {
        Set(
            try names.map { name in
                guard let label = workspace.labels.values.first(where: { $0.name.lowercased() == name.lowercased() })
                else {
                    throw LifrError.notFound(.label, name)
                }
                return label.id
            })
    }

    private func decode<T: Decodable>(_ params: JSONValue, for method: APIMethod) throws -> T {
        do {
            return try params.decode(T.self)
        } catch let error as LifrError {
            throw APIError.invalidParams(method.rawValue, error.description)
        } catch let DecodingError.keyNotFound(key, _) {
            throw APIError.invalidParams(method.rawValue, "\"\(key.stringValue)\" is required.")
        } catch let DecodingError.typeMismatch(_, context), let DecodingError.valueNotFound(_, context) {
            throw APIError.invalidParams(
                method.rawValue,
                "\"\(context.codingPath.map(\.stringValue).joined(separator: "."))\" has the wrong type. `lifr mcp` "
                    + "lists each tool's schema."
            )
        } catch let DecodingError.dataCorrupted(context) {
            if let error = context.underlyingError as? LifrError {
                throw APIError.invalidParams(method.rawValue, error.description)
            }
            throw APIError.invalidParams(
                method.rawValue,
                "\"\(context.codingPath.map(\.stringValue).joined(separator: "."))\" isn't a valid value. "
                    + "\(context.debugDescription)"
            )
        }
    }

    // MARK: - Results

    private func search(_ params: SearchTasks.Params, _ workspace: Workspace) throws -> [TaskSummary] {
        let avenueID = try params.avenue.map { try avenue(key: $0, workspace).id }
        let visible = Set(visibleAvenues(workspace).map(\.id))
        let words = (params.query ?? "").lowercased().split(whereSeparator: \.isWhitespace)
        let statuses = params.statuses.map(Set.init)
        let limit = min(max(params.limit ?? SearchTasks.defaultLimit, 1), SearchTasks.maxLimit)
        return workspace.tasks.values
            .filter { task in
                guard task.deletedAt == nil, visible.contains(task.avenueID) else { return false }
                if let avenueID, task.avenueID != avenueID { return false }
                if let project = params.project, task.projectID != project { return false }
                let status = task.effectiveStatus(now: context.now, settings: workspace.settings)
                if let statuses {
                    guard statuses.contains(status) else { return false }
                } else if params.includeClosed != true, status.isClosed {
                    return false
                }
                let text = (task.title + "\n" + task.details).lowercased()
                return words.allSatisfy { text.contains($0) }
            }
            .sorted(by: LifrTask.listOrder)
            .prefix(limit)
            .map { summary($0, workspace) }
    }

    private func summary(_ task: LifrTask, _ workspace: Workspace) -> TaskSummary {
        let procrastination: String? =
            switch task.procrastination(now: context.now, settings: workspace.settings) {
            case .setByHand: "setByHand"
            case .inactive(let days): "inactive \(days) days"
            case nil: nil
            }
        return TaskSummary(
            code: workspace.displayCode(of: task), title: task.title,
            status: task.effectiveStatus(now: context.now, settings: workspace.settings),
            procrastination: procrastination, priority: task.priority,
            avenue: workspace.avenues[task.avenueID]?.key.rawValue ?? "", project: task.projectID,
            parent: task.parentID.flatMap { workspace.tasks[$0] }.map { workspace.displayCode(of: $0) },
            startDate: task.startDate, dueDate: task.dueDate, dueTime: task.dueTime,
            labels: task.labelIDs.compactMap { workspace.labels[$0]?.name }.sorted(),
            openSubtasks: workspace.openSubtasks(of: task.id).count)
    }

    private func detail(_ task: LifrTask, _ workspace: Workspace) -> TaskDetail {
        func actor(_ actor: Actor) -> ActorSummary {
            ActorSummary(kind: actor.kind, device: actor.device, client: actor.client)
        }
        let comments = workspace.comments.values.filter { $0.taskID == task.id }.map {
            TimelineEntry(
                kind: .comment, at: $0.createdAt, actor: actor($0.author),
                body: $0.deletedAt == nil ? $0.body : nil, commentID: $0.id)
        }
        let history = workspace.history.filter { $0.taskID == task.id }.map {
            TimelineEntry(
                kind: .history, at: $0.createdAt, actor: actor($0.actor), event: $0.kind,
                field: $0.field, oldValue: $0.oldValue, newValue: $0.newValue, reason: $0.reason)
        }
        // Stable for equal times: history before comments, each in its own order.
        let timeline = (history + comments).enumerated()
            .sorted { ($0.element.at, $0.offset) < ($1.element.at, $1.offset) }
            .map(\.element)
        return TaskDetail(
            task: summary(task, workspace), details: task.details, createdAt: task.createdAt,
            closedAt: task.closedAt,
            subtasks: workspace.subtasks(of: task.id).map { summary($0, workspace) },
            timeline: timeline)
    }

    private func result(of change: Change, for id: UUID, _ workspace: Workspace) -> ChangeResult {
        let others = change.previous.keys
            .filter { $0 != id && $0 != change.reopenedParent }
            .compactMap { workspace.tasks[$0] }
            .sorted(by: LifrTask.listOrder)
        return ChangeResult(
            task: summary(workspace.tasks[id]!, workspace),
            alsoChanged: others.map { summary($0, workspace) },
            reopenedParent: change.reopenedParent.flatMap { workspace.tasks[$0] }.map { workspace.displayCode(of: $0) })
    }

    // MARK: - Errors

    private func apiError(_ error: LifrError, _ workspace: Workspace) -> APIError {
        func codes(_ ids: [UUID]) -> [String] {
            ids.compactMap { workspace.tasks[$0] }.map { workspace.displayCode(of: $0) }
        }
        switch error {
        case .openSubtasks(_, let open):
            return APIError(.openSubtasks, error.description, tasks: codes(open))
        case .wontDoNeedsCascade(_, let open):
            return APIError(.confirmationRequired, error.description, tasks: codes(open))
        case .projectHasOpenTasks(_, let open):
            return APIError(.ruleViolation, error.description, tasks: codes(open))
        case .notFound:
            return APIError(.notFound, error.description)
        case .invalidTaskCode, .invalidAvenueKey, .invalidDate, .invalidTime:
            return APIError(.invalidParams, error.description)
        default:
            return APIError(.ruleViolation, error.description)
        }
    }
}
