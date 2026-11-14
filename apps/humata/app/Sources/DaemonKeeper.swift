import AppKit
import CryptoKit
import Darwin
import Foundation
import HumataAppFeature
import HumataDaemonClient
import HumataDaemonLive
import Security
import ServiceManagement

extension DaemonService {
    /// Keeps the humatad inside this app running on `socket`.
    static func live(socket: String, programVersion: String) -> DaemonService {
        let keeper = DaemonKeeper(socket: socket, programVersion: programVersion)
        return DaemonService(
            start: { try await keeper.start() },
            build: { try await keeper.build() },
            restart: { try await keeper.restart() },
            openLoginItems: { SMAppService.openSystemSettingsLoginItems() },
            stop: { try await keeper.stop() })
    }
}

/// Starts, checks and replaces humatad. A team-signed app registers the
/// login agent in Contents/Library/LaunchAgents; an ad hoc one, such as a
/// development build, which launchd won't run as an agent, runs humatad
/// itself and leaves it running when it quits, as a login item would be, so
/// turns going on aren't cut off. It notes which humatad it ran, so a later
/// launch of a newer build replaces that one once its sessions are idle.
private final class DaemonKeeper: @unchecked Sendable {
    private let socket: String
    private let programVersion: String
    private let executable = Bundle.main.bundleURL.appending(path: "Contents/Helpers/humatad")
    private let agent = SMAppService.agent(plistName: "daemon.plist")
    private let asLoginItem = DaemonKeeper.isSignedByATeam()
    private let lock = NSLock()
    /// The humatad this app started, when it isn't a login item.
    private var child: Process?
    /// Where the process ID of the humatad an ad hoc build started is kept,
    /// for a later launch to know it as its own.
    private var pidFile: URL { URL(filePath: socket + ".pid") }

    /// How long humatad has to answer after it is started. Before it
    /// listens it reads the login shell's environment, which may take 10
    /// seconds on its own, and restores its sessions; a child that exits, or
    /// a login item launchd won't run, ends the wait sooner.
    private static let answerSeconds = 60

    init(socket: String, programVersion: String) {
        self.socket = socket
        self.programVersion = programVersion
    }

    func start() async throws {
        if asLoginItem {
            try register()
        } else if await answers() {
            return
        } else {
            try spawn()
        }
        try await waitForAnswer()
    }

    /// How the daemon answering stands against this app's build. A humatad
    /// no build of this app started, which speaks its protocol, is used as
    /// it is, since only whoever started it can replace it. Another build is
    /// replaced unless it's a newer Humata's, whose store this copy's
    /// humatad couldn't open.
    func build() async throws -> DaemonBuild {
        let build: String
        do {
            build = try await DaemonClient.daemonBuild(socket: socket, programVersion: programVersion)
        } catch DaemonError.versionMismatch(let daemon) {
            return isNewer(daemon) ? .newer : .older
        }
        if build.hasSuffix("+" + (try bundledDigest())) { return .current }
        if !asLoginItem && runningChild() == nil && startedBefore() == nil { return .current }
        return isNewer(build) ? .newer : .older
    }

    /// Whether the daemon whose hello says `build` is a newer Humata's than
    /// this app: by the version before the `+` in it, or, as humatad built
    /// outside a release says 0.0.0, by the app each humatad running sits in.
    private func isNewer(_ build: String) -> Bool {
        let said = String(build.prefix { $0 != "+" })
        if BundleVersion(short: said, build: "0") > BundleVersion(short: programVersion, build: "0") { return true }
        guard let info = Bundle.main.infoDictionary, let mine = BundleVersion(info: info) else { return false }
        return runningAppVersions().contains { $0 > mine }
    }

    /// The versions of the apps, other than this one, whose humatad runs:
    /// read from the Info.plist of the bundle each sits in, as
    /// `Humata.app/Contents/Helpers/humatad`.
    private func runningAppVersions() -> [BundleVersion] {
        let count = proc_listallpids(nil, 0)
        guard count > 0 else { return [] }
        var pids = [pid_t](repeating: 0, count: Int(count) + 16)
        let filled = pids.withUnsafeMutableBytes { proc_listallpids($0.baseAddress, Int32($0.count)) }
        guard filled > 0 else { return [] }
        let ours = executable.resolvingSymlinksInPath().path(percentEncoded: false)
        return pids.prefix(Int(filled)).compactMap { pid -> BundleVersion? in
            guard pid > 0, let running = Self.path(of: pid), running.hasSuffix("/Contents/Helpers/humatad"),
                URL(filePath: running).resolvingSymlinksInPath().path(percentEncoded: false) != ours
            else { return nil }
            let contents = URL(filePath: running).deletingLastPathComponent().deletingLastPathComponent()
            guard let data = try? Data(contentsOf: contents.appending(path: "Info.plist")),
                let info = (try? PropertyListSerialization.propertyList(from: data, format: nil)) as? [String: Any]
            else { return nil }
            return BundleVersion(info: info)
        }
    }

    /// The executable process `pid` runs, if it can be read.
    private static func path(of pid: pid_t) -> String? {
        var path = [CChar](repeating: 0, count: 4 * 1024)
        let length = proc_pidpath(pid, &path, UInt32(path.count))
        guard length > 0 else { return nil }
        return String(bytes: path.prefix(Int(length)).map { UInt8(bitPattern: $0) }, encoding: .utf8)
    }

    func restart() async throws {
        if asLoginItem {
            try await agent.unregister()
            try await waitUntilGone()
            try register()
        } else {
            if runningChild() != nil {
                stopChild()
            } else if let earlier = startedBefore() {
                kill(earlier, SIGTERM)
            } else {
                throw DaemonServiceError.notManaged(socket: socket)
            }
            try await waitUntilGone()
            try spawn()
        }
        try await waitForAnswer()
    }

    /// Stops the daemon so the app can be replaced: the login item is
    /// taken off, so launchd doesn't start it again, and the next launch
    /// adds it back from where the app then is. One this app started
    /// itself is stopped. One nothing here started can't be, and if it
    /// answers, this throws `stillRunning`, as replacing the app under it
    /// isn't safe. True when a daemon was stopped.
    func stop() async throws -> Bool {
        let stopping: Bool
        if asLoginItem {
            if agent.status == .enabled {
                do {
                    try await agent.unregister()
                } catch {
                    throw DaemonServiceError.unregisterFailed(underlying: error)
                }
                stopping = true
            } else {
                // Another copy, such as the one in Applications, registered
                // the login item: launchd keeps it under the same label.
                stopping = Self.bootOutLoginItem()
            }
        } else if runningChild() != nil {
            stopChild()
            stopping = true
        } else if let earlier = startedBefore() {
            kill(earlier, SIGTERM)
            stopping = true
        } else {
            stopping = false
        }
        if stopping {
            try await waitUntilGone()
        } else if await answers() {
            throw DaemonServiceError.stillRunning(seconds: 0)
        }
        return stopping
    }

    // MARK: Login item

    /// Takes the login item another copy of this app registered out of
    /// launchd, so it isn't started again while this copy replaces that
    /// one. True when launchd had it; the copy that next opens adds it back.
    private static func bootOutLoginItem() -> Bool {
        guard let identifier = Bundle.main.bundleIdentifier else { return false }
        let process = Process()
        process.executableURL = URL(filePath: "/bin/launchctl")
        process.arguments = ["bootout", "gui/\(getuid())/\(identifier).daemon"]
        process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice
        do {
            try process.run()
        } catch {
            return false
        }
        process.waitUntilExit()
        return process.terminationStatus == 0
    }

    private func register() throws {
        switch agent.status {
        case .enabled:
            return
        case .requiresApproval:
            throw DaemonServiceError.needsApproval
        case .notRegistered, .notFound:
            do {
                try agent.register()
            } catch {
                if agent.status == .requiresApproval { throw DaemonServiceError.needsApproval }
                throw DaemonServiceError.registerFailed(underlying: error)
            }
            if agent.status == .requiresApproval { throw DaemonServiceError.needsApproval }
        @unknown default:
            try agent.register()
        }
    }

    // MARK: Child

    private func spawn() throws {
        guard FileManager.default.isExecutableFile(atPath: executable.path(percentEncoded: false)) else {
            throw DaemonServiceError.missingExecutable(path: executable.path(percentEncoded: false))
        }
        let process = Process()
        process.executableURL = executable
        process.arguments = ["daemon"]
        do {
            try process.run()
        } catch {
            throw DaemonServiceError.launchFailed(path: executable.path(percentEncoded: false), underlying: error)
        }
        lock.withLock { child = process }
        try? Data(String(process.processIdentifier).utf8).write(to: pidFile, options: .atomic)
    }

    /// The humatad a launch of this app before started, if it still runs:
    /// the process it noted, running the humatad at this app's path.
    private func startedBefore() -> pid_t? {
        guard let noted = try? String(contentsOf: pidFile, encoding: .utf8),
            let pid = pid_t(noted.trimmingCharacters(in: .whitespacesAndNewlines)), pid > 0
        else { return nil }
        guard let running = Self.path(of: pid) else { return nil }
        let ours = executable.resolvingSymlinksInPath().path(percentEncoded: false)
        return URL(filePath: running).resolvingSymlinksInPath().path(percentEncoded: false) == ours ? pid : nil
    }

    private func runningChild() -> Process? {
        lock.withLock { child?.isRunning == true ? child : nil }
    }

    /// Asks the child to stop, which ends its turns, and waits for it.
    private func stopChild() {
        let process = lock.withLock {
            defer { child = nil }
            return child
        }
        guard let process, process.isRunning else { return }
        process.terminate()
        process.waitUntilExit()
    }

    // MARK: Answers

    private func answers() async -> Bool {
        do {
            _ = try await DaemonClient.daemonBuild(socket: socket, programVersion: programVersion)
            return true
        } catch DaemonError.versionMismatch {
            return true
        } catch {
            return false
        }
    }

    private func waitForAnswer() async throws {
        for _ in 0..<(Self.answerSeconds * 10) {
            if await answers() { return }
            if !asLoginItem, runningChild() == nil { break }
            if asLoginItem, agent.status != .enabled { break }
            try await Task.sleep(for: .milliseconds(100))
        }
        throw DaemonServiceError.noAnswer(socket: socket, seconds: Self.answerSeconds)
    }

    /// Waits until nothing answers, and throws `stillRunning` if something
    /// still does after `answerSeconds`, so nothing is put over it.
    private func waitUntilGone() async throws {
        for _ in 0..<(Self.answerSeconds * 10) {
            if !(await answers()) { return }
            try await Task.sleep(for: .milliseconds(100))
        }
        if await answers() { throw DaemonServiceError.stillRunning(seconds: Self.answerSeconds) }
    }

    // MARK: Builds

    /// The start of the bundled humatad's SHA-256, as it puts in its hello.
    private func bundledDigest() throws -> String {
        let data: Data
        do {
            data = try Data(contentsOf: executable, options: .mappedIfSafe)
        } catch {
            throw DaemonServiceError.missingExecutable(path: executable.path(percentEncoded: false))
        }
        return SHA256.hash(data: data).prefix(8).map { String(format: "%02x", $0) }.joined()
    }

    /// Whether this app is signed with a team's certificate, as a login
    /// agent needs; an ad hoc signature has no team.
    private static func isSignedByATeam() -> Bool {
        var code: SecCode?
        var staticCode: SecStaticCode?
        var information: CFDictionary?
        guard SecCodeCopySelf([], &code) == errSecSuccess, let code,
            SecCodeCopyStaticCode(code, [], &staticCode) == errSecSuccess, let staticCode,
            SecCodeCopySigningInformation(staticCode, SecCSFlags(rawValue: kSecCSSigningInformation), &information)
                == errSecSuccess,
            let information = information as? [String: Any]
        else { return false }
        return information[kSecCodeInfoTeamIdentifier as String] is String
    }
}
