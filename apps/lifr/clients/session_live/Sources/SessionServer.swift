#if os(macOS)
    import Foundation
    import LifrAPI
    import LifrCore
    import LifrSessionClient
    import LifrStoreClient
    import LocalAuthentication
    import os

    /// The local API: listens on the socket, lets only the bundled `lifr` in,
    /// binds grants to the app that ran it, and runs each request through the
    /// API's handler and the store, asking for Touch ID where the person must
    /// agree.
    package actor SessionServer {
        struct Environment: Sendable {
            var socket: URL
            var disabledMarker: URL
            var peers: PeerCheck
            var grants: GrantStorage
            /// Touch ID or the password, with what it's for: "Lifr is trying to
            /// <reason>".
            var authenticate: @Sendable (_ reason: String) async -> Authentication
            var store: StoreClient
            /// This Mac's name, for history.
            var device: String
            var now: @Sendable () -> Date
            var log: AccessLog
            var cli: CLILink
            var loginItem: LoginItem
        }

        /// An answer to an app waiting for approval.
        enum Answer: Sendable {
            case granted(Grant)
            case denied
            case withdrawn
        }

        private struct Pending {
            var request: ApprovalRequest
            var launcher: Launcher
            /// The requests waiting on it; another connection from the same
            /// app waits on the same sheet.
            var waiters: [UUID: CheckedContinuation<Answer, Never>] = [:]
        }

        private let environment: Environment
        private var grants: [String: StoredGrant] = [:]
        private var pending: [UUID: Pending] = [:]
        /// Unlocked terminals by launcher and tty, with their last use.
        private var terminals: [String: Date] = [:]
        /// Open connections, with the launcher each belongs to.
        private var connections: [ObjectIdentifier: (connection: Connection, identity: String)] = [:]
        private var listeners: [UUID: AsyncStream<SessionEvent>.Continuation] = [:]
        private var listener: SocketListener?
        private var started = false
        /// Whether `grants` holds what the Keychain keeps.
        private var grantsLoaded = false

        init(_ environment: Environment) {
            self.environment = environment
        }

        /// The server for the app: its socket in the App Group container, grants
        /// in the Keychain and Touch ID.
        package static func live(store: StoreClient, device: String) -> SessionServer {
            let home = FileManager.default.homeDirectoryForCurrentUser
            let helper = Bundle.main.bundleURL.appendingPathComponent("Contents/Helpers/lifr")
            let support = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
                .appendingPathComponent(Bundle.main.bundleIdentifier ?? "Lifr", isDirectory: true)
            return SessionServer(
                Environment(
                    socket: LocalSocket.socket(home: home), disabledMarker: LocalSocket.disabledMarker(home: home),
                    peers: .codeSigning(cli: helper, team: LocalSocket.team, identifier: LocalSocket.cliIdentifier),
                    grants: .keychain(service: "\(Bundle.main.bundleIdentifier ?? "Lifr").grants"),
                    authenticate: { reason in
                        do {
                            let agreed = try await LAContext().evaluatePolicy(
                                .deviceOwnerAuthentication, localizedReason: reason)
                            return agreed ? .agreed : .declined
                        } catch let error as LAError
                            where [LAError.Code.userCancel, .appCancel, .systemCancel].contains(error.code)
                        {
                            return .declined
                        } catch {
                            return .failed(reason: error.localizedDescription)
                        }
                    },
                    store: store, device: device, now: { Date() },
                    log: AccessLog(file: support.appendingPathComponent("access.log")),
                    cli: .live(helper: helper, home: home), loginItem: .mainApp))
        }

        /// Loads the grants and starts listening, unless the API is turned off.
        package func start() {
            guard !started else { return }
            started = true
            // A Keychain that won't open leaves no grants: every app asks
            // again, and saving reads it first so no grant is overwritten.
            grantsLoaded = (try? loadGrants()) != nil
            environment.log.prune(now: environment.now())
            if isAPIEnabled { try? listen() }
        }

        package nonisolated var client: SessionClient {
            SessionClient(
                apps: { await self.apps() },
                events: { await self.events() },
                approve: { try await self.approve($0, $1) },
                deny: { try await self.deny($0) },
                update: { try await self.update($0, $1) },
                revoke: { try await self.revoke($0) },
                cliInstallation: { self.environment.cli.installation() },
                installCLI: { try self.environment.cli.install() },
                isAPIEnabled: { await self.isListening },
                setAPIEnabled: { try await self.setAPIEnabled($0) },
                launchesAtLogin: { self.environment.loginItem.isEnabled() },
                setLaunchesAtLogin: { try self.environment.loginItem.setEnabled($0) })
        }

        // MARK: - The socket

        private var isAPIEnabled: Bool {
            !FileManager.default.fileExists(atPath: environment.disabledMarker.path)
        }

        /// Whether the API is on and answering: on, but with a socket that
        /// couldn't open at launch, it reads as off, so turning it on tries
        /// again and says why it can't.
        private var isListening: Bool {
            isAPIEnabled && (!started || listener != nil)
        }

        private func listen() throws(SessionError) {
            guard listener == nil else { return }
            listener = try SocketListener(path: environment.socket) { [weak self] connection in
                Task { await self?.serve(connection) }
            }
        }

        private func setAPIEnabled(_ enabled: Bool) throws(SessionError) {
            let files = FileManager.default
            if enabled {
                if files.fileExists(atPath: environment.disabledMarker.path) {
                    do {
                        try files.removeItem(at: environment.disabledMarker)
                    } catch {
                        throw .apiSwitchFailed(reason: error.localizedDescription)
                    }
                }
                try listen()
            } else {
                do {
                    try files.createDirectory(
                        at: environment.disabledMarker.deletingLastPathComponent(), withIntermediateDirectories: true)
                    try Data().write(to: environment.disabledMarker)
                } catch {
                    throw .apiSwitchFailed(reason: error.localizedDescription)
                }
                listener?.stop()
                listener = nil
                for (connection, _) in connections.values { connection.close() }
            }
            send(.apiEnabledChanged(enabled))
        }

        private nonisolated func serve(_ connection: Connection) async {
            let peer: Peer
            switch environment.peers.identify(connection.descriptor) {
            case .failure(let error):
                if let line = try? Wire.line(RPCResponse.failure(id: nil, error)) { connection.write(line) }
                connection.close()
                environment.log.append(
                    AccessLog.Entry(
                        at: environment.now(), client: nil, method: nil, tasks: [], result: error.code.rawValue))
                return
            case .success(let identified):
                peer = identified
            }
            guard await opened(connection, by: peer) else {
                connection.close()
                return
            }
            while let line = await connection.readLine() {
                let response = await respond(to: line, from: peer, over: connection)
                guard let data = try? Wire.line(response), connection.write(data) else { break }
            }
            connection.close()
            await closed(connection)
        }

        /// Records an open connection, or `false` when the API was turned off
        /// after it was accepted.
        private func opened(_ connection: Connection, by peer: Peer) -> Bool {
            guard listener != nil else { return false }
            connections[ObjectIdentifier(connection)] = (connection, peer.launcher.identity)
            appsChanged()
            return true
        }

        private func closed(_ connection: Connection) {
            connections[ObjectIdentifier(connection)] = nil
            appsChanged()
        }

        // MARK: - Requests

        private func respond(to line: Data, from peer: Peer, over connection: Connection) async -> RPCResponse {
            guard let request = try? Wire.decoder.decode(RPCRequest.self, from: line), request.jsonrpc == "2.0" else {
                // The handler words every malformed request.
                var empty = Workspace()
                return Handler(grant: nil, context: context(.cli), confirm: { _ in false }).handle(
                    line: line, in: &empty)
            }
            let method = APIMethod(rawValue: request.method)
            let response: RPCResponse
            let acting: Grant?
            switch await grant(for: peer, method, over: connection) {
            case .failure(let error):
                response = .failure(id: request.id, error)
                acting = nil
            case .success(let found):
                acting = found
                response = await run(request, method, acting, peer)
            }
            if response.error == nil {
                used(acting, by: peer, method)
            }
            environment.log.append(
                AccessLog.Entry(
                    at: environment.now(), client: peer.launcher.name, method: request.method,
                    tasks: tasks(request, response), result: response.error?.data.code.rawValue ?? "ok"))
            return response
        }

        /// The grant `peer` acts under, `nil` before it needs one, or why it's
        /// refused. An unknown app waits here for the person to approve it.
        private func grant(for peer: Peer, _ method: APIMethod?, over connection: Connection) async
            -> Result<Grant?, APIError>
        {
            let launcher = peer.launcher
            if launcher.isTerminal {
                if method == .unlock {
                    guard peer.tty != nil else {
                        return .failure(
                            .notAuthorized(
                                "lifr unlock needs a terminal window, and this lifr has none. Run it in Terminal "
                                    + "directly."))
                    }
                    return .success(nil)
                }
                guard method?.scope != nil else { return .success(nil) }
                guard let key = terminalKey(peer), let lastUse = terminals[key] else {
                    return .failure(
                        .notAuthorized(
                            "This terminal isn't unlocked. Run `lifr unlock`, approve with Touch ID, then retry."))
                }
                let now = environment.now()
                let expiresAt: Date
                if lastUse + Unlock.idleTimeout > now {
                    terminals[key] = now
                    expiresAt = now + Unlock.idleTimeout
                } else {
                    // The handler says the session ended.
                    terminals[key] = nil
                    expiresAt = lastUse + Unlock.idleTimeout
                }
                return .success(
                    Grant(
                        client: launcher.name, scopes: [.read, .write], expiresAt: expiresAt, isTerminalSession: true))
            }
            if method == .unlock {
                return .failure(
                    .notAuthorized(
                        "`lifr unlock` works only in a terminal app, such as Terminal or iTerm2. \(launcher.name) "
                            + "gets access from the approval sheet in the Lifr app instead."))
            }
            if let stored = grants[launcher.identity] {
                guard stored.team == launcher.team else {
                    return .failure(
                        .notAuthorized(
                            "\(launcher.name) is signed by a different developer than the one you approved, so Lifr "
                                + "refuses it. If you trust this version, revoke it in Settings > Connected apps, then "
                                + "approve it again."))
                }
                return .success(stored.grant)
            }
            guard let method, method.scope != nil else { return .success(nil) }
            switch await approval(for: launcher, method, over: connection) {
            case .granted(let grant):
                return .success(grant)
            case .denied:
                return .failure(
                    .notAuthorized(
                        "You declined \(launcher.name) in the Lifr app. Run the command again to ask anew."))
            case .withdrawn:
                return .failure(
                    .notAuthorized("The approval request was withdrawn before you answered. Run the command again."))
            }
        }

        /// Shows the approval sheet, or joins the one up for the same app, and
        /// waits for the answer. A CLI that hangs up withdraws its part.
        private func approval(for launcher: Launcher, _ method: APIMethod, over connection: Connection) async -> Answer
        {
            let id: UUID
            if let existing = pending.first(where: { $0.value.launcher.identity == launcher.identity })?.key {
                id = existing
            } else {
                id = UUID()
                let request = ApprovalRequest(
                    id: id, identity: launcher.identity, client: launcher.name, method: method,
                    requestedAt: environment.now(), isInterpreter: launcher.isInterpreter)
                pending[id] = Pending(request: request, launcher: launcher)
                send(.approvalRequested(request))
            }
            let waiter = UUID()
            let watch = Task { [weak self] in
                while !Task.isCancelled {
                    try? await Task.sleep(for: .milliseconds(200))
                    if connection.hasHungUp {
                        await self?.withdraw(waiter, from: id)
                        return
                    }
                }
            }
            defer { watch.cancel() }
            return await withCheckedContinuation { continuation in
                pending[id]?.waiters[waiter] = continuation
            }
        }

        private func withdraw(_ waiter: UUID, from id: UUID) {
            guard let continuation = pending[id]?.waiters.removeValue(forKey: waiter) else { return }
            continuation.resume(returning: .withdrawn)
            if pending[id]?.waiters.isEmpty == true {
                pending[id] = nil
                send(.approvalWithdrawn(id))
            }
        }

        /// Runs `request`. The first run is on a copy, to learn whether it needs
        /// the person's say; a change then runs again inside a save, so it's
        /// all or nothing against the records as they are by then.
        private func run(_ request: RPCRequest, _ method: APIMethod?, _ grant: Grant?, _ peer: Peer) async
            -> RPCResponse
        {
            let loaded: Workspace
            do {
                loaded = try await environment.store.load()
            } catch {
                return .failure(
                    id: request.id,
                    APIError(
                        .internalError,
                        "Lifr couldn't read your tasks: \(error.localizedDescription). Open Lifr to see why, "
                            + "then retry."))
            }
            let asked = OSAllocatedUnfairLock<Confirmation?>(initialState: nil)
            var copy = loaded
            let surface = request.surface
            let first = Handler(
                grant: grant, context: context(surface),
                confirm: { confirmation in
                    asked.withLock { $0 = confirmation }
                    return false
                }
            ).handle(request, in: &copy)
            let approved = asked.withLock { $0 }
            if let approved {
                switch await environment.authenticate(reason(for: approved, peer.launcher)) {
                case .agreed: break
                case .declined: return .failure(id: request.id, .confirmationDenied)
                case .failed(let reason):
                    return .failure(
                        id: request.id,
                        APIError(
                            .confirmationDenied,
                            "Touch ID or the password couldn't confirm the change on the Mac: \(reason). Nothing "
                                + "changed; once the person can confirm it there, run the command again."))
                }
            } else if first.error != nil || copy == loaded {
                return first
            }
            let confirm: @Sendable (Confirmation) -> Bool = { $0 == approved }
            guard method?.scope == .write else {
                // Unlock and export change nothing.
                var again = loaded
                return Handler(grant: grant, context: context(surface), confirm: confirm).handle(request, in: &again)
            }
            let response = OSAllocatedUnfairLock<RPCResponse?>(initialState: nil)
            do {
                let saved = try await environment.store.save { workspace, context in
                    var context = context
                    context.actor = .user(device: context.actor.device, surface: surface)
                    let handler = Handler(grant: grant, context: context, confirm: confirm)
                    var copy = workspace
                    let answer = handler.handle(request, in: &copy)
                    response.withLock { $0 = answer }
                    if answer.error == nil { workspace = copy }
                    // The journal credits the client, not the app.
                    var change = Change()
                    change.actor = handler.context.actor
                    return change
                }
                if response.withLock({ $0?.error }) == nil {
                    send(.recordsChanged(saved.workspace, number: saved.number))
                }
            } catch {
                return .failure(
                    id: request.id,
                    APIError(
                        .internalError,
                        "Lifr couldn't save the change: \(error.localizedDescription). Nothing changed; retry."))
            }
            return response.withLock { $0 } ?? first
        }

        /// Records the use: an unlock opens the terminal's session, and agents'
        /// last use shows in Settings.
        private func used(_ grant: Grant?, by peer: Peer, _ method: APIMethod?) {
            if method == .unlock, let key = terminalKey(peer) {
                terminals[key] = environment.now()
                return
            }
            guard grant?.isTerminalSession == false, var stored = grants[peer.launcher.identity] else { return }
            stored.lastUsedAt = environment.now()
            grants[peer.launcher.identity] = stored
            try? persist()
            appsChanged()
        }

        private func reason(for confirmation: Confirmation, _ launcher: Launcher) -> String {
            switch confirmation {
            case .write(let method, let task):
                "let \(launcher.name) \(method.rawValue.replacingOccurrences(of: "_", with: " ")) \(task ?? "")"
                    .trimmingCharacters(in: .whitespaces)
            case .trash(let codes):
                "let \(launcher.name) move \(codes.joined(separator: ", ")) to the trash"
            case .unlock:
                "unlock lifr in \(launcher.name) for 15 idle minutes"
            case .export:
                "export every Lifr record to \(launcher.name)"
            }
        }

        private func context(_ surface: Actor.Surface) -> Context {
            Context(actor: .user(device: environment.device, surface: surface), now: environment.now())
        }

        private func terminalKey(_ peer: Peer) -> String? {
            peer.tty.map { "\(peer.launcher.identity) \($0)" }
        }

        private func tasks(_ request: RPCRequest, _ response: RPCResponse) -> [String] {
            var tasks: [String] = []
            if case .object(let params)? = request.params {
                for key in ["task", "parent"] {
                    if case .string(let code)? = params[key] { tasks.append(code) }
                }
            }
            return tasks + (response.error?.data.tasks ?? [])
        }

        // MARK: - Grants

        private func apps() -> [ConnectedApp] {
            let connected = Set(connections.values.map(\.identity))
            return grants.values
                .map {
                    ConnectedApp(
                        id: $0.identity, grant: $0.grant, lastUsedAt: $0.lastUsedAt,
                        isConnected: connected.contains($0.identity))
                }
                .sorted { ($0.lastUsedAt ?? .distantPast, $1.name) > ($1.lastUsedAt ?? .distantPast, $0.name) }
        }

        private func approve(_ id: UUID, _ grant: Grant) async throws(SessionError) {
            guard let request = pending[id]?.request else { throw .unknownRequest(id) }
            try await authenticate("approve \(request.client) to use your tasks")
            // The app may have stopped waiting during Touch ID.
            guard let answered = pending.removeValue(forKey: id) else { throw .unknownRequest(id) }
            let previous = grants[answered.launcher.identity]
            grants[answered.launcher.identity] = StoredGrant(
                identity: answered.launcher.identity, team: answered.launcher.team, grant: grant)
            do {
                try persist()
            } catch {
                grants[answered.launcher.identity] = previous
                pending[id] = answered
                throw error
            }
            for waiter in answered.waiters.values { waiter.resume(returning: .granted(grant)) }
            appsChanged()
        }

        /// Asks for Touch ID or the password, throwing unless the person agreed.
        private func authenticate(_ reason: String) async throws(SessionError) {
            switch await environment.authenticate(reason) {
            case .agreed: return
            case .declined: throw .notAuthenticated
            case .failed(let reason): throw .authenticationFailed(reason: reason)
            }
        }

        private func deny(_ id: UUID) throws(SessionError) {
            guard let answered = pending.removeValue(forKey: id) else { throw .unknownRequest(id) }
            for waiter in answered.waiters.values { waiter.resume(returning: .denied) }
        }

        private func update(_ app: String, _ grant: Grant) async throws(SessionError) {
            guard let name = grants[app]?.grant.client else { throw .unknownApp(app) }
            try await authenticate("change \(name)'s access to your tasks")
            guard let previous = grants[app] else { throw .unknownApp(app) }
            grants[app]?.grant = grant
            do {
                try persist()
            } catch {
                grants[app] = previous
                throw error
            }
            appsChanged()
        }

        private func revoke(_ app: String) throws(SessionError) {
            guard let previous = grants.removeValue(forKey: app) else { throw .unknownApp(app) }
            do {
                try persist()
            } catch {
                grants[app] = previous
                throw error
            }
            appsChanged()
        }

        private func persist() throws(SessionError) {
            if !grantsLoaded {
                // Kept grants this run hasn't seen yet, under the changes
                // made since.
                let changed = grants
                try loadGrants()
                grants.merge(changed) { $1 }
                grantsLoaded = true
            }
            try environment.grants.save(grants.values.sorted { $0.identity < $1.identity })
        }

        private func loadGrants() throws(SessionError) {
            grants = Dictionary(try environment.grants.load().map { ($0.identity, $0) }, uniquingKeysWith: { $1 })
        }

        // MARK: - Events

        private func events() -> AsyncStream<SessionEvent> {
            let (stream, continuation) = AsyncStream<SessionEvent>.makeStream()
            let id = UUID()
            listeners[id] = continuation
            continuation.onTermination = { [weak self] _ in
                Task { await self?.stopListening(id) }
            }
            return stream
        }

        private func stopListening(_ id: UUID) {
            listeners[id] = nil
        }

        private func appsChanged() {
            send(.appsChanged(apps()))
        }

        private func send(_ event: SessionEvent) {
            for listener in listeners.values { listener.yield(event) }
        }
    }

    /// What Touch ID or the password answered.
    enum Authentication: Equatable, Sendable {
        case agreed
        /// The person cancelled.
        case declined
        /// It couldn't ask, such as with Touch ID locked out.
        case failed(reason: String)
    }
#endif
