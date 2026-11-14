import ComposableArchitecture
import Foundation

/// Keeps humatad running for the app: as a login item when the app is signed
/// by a team, otherwise as the app's own child, stopped when it quits.
@DependencyClient
package struct DaemonService: Sendable {
    /// Starts humatad unless one answers already, and waits until it does.
    package var start: @Sendable () async throws -> Void
    /// How the daemon answering stands against the build inside this app.
    package var build: @Sendable () async throws -> DaemonBuild = { .current }
    /// Stops the daemon and starts this app's, waiting until it answers.
    /// Turns in progress end.
    package var restart: @Sendable () async throws -> Void
    /// Opens Login Items in System Settings.
    package var openLoginItems: @Sendable () -> Void
    /// Stops the daemon for the app's files to be replaced, as the
    /// installer does, and waits until it no longer answers. Turns in
    /// progress end. True when a daemon was running and was stopped; throws
    /// `stillRunning` when one still answers.
    package var stop: @Sendable () async throws -> Bool
}

/// How the daemon answering stands against the build inside this app.
package enum DaemonBuild: Equatable, Sendable {
    /// This app's own build, or one it uses as it is: a humatad no build of
    /// this app started.
    case current
    /// Another build, no newer than this app: it's replaced once no session
    /// is in a turn.
    case older
    /// A newer Humata's: this copy can't use it, and leaves it running.
    case newer
}

extension DaemonService: TestDependencyKey {
    package static let testValue = DaemonService()
    package static let previewValue = DaemonService(
        start: {}, build: { .current }, restart: {}, openLoginItems: {}, stop: { false })
}

extension DependencyValues {
    package var daemonService: DaemonService {
        get { self[DaemonService.self] }
        set { self[DaemonService.self] = newValue }
    }
}

/// Why humatad couldn't be started or replaced.
package enum DaemonServiceError: LocalizedError, CustomStringConvertible {
    /// Its login item is turned off in System Settings.
    case needsApproval
    /// macOS refused to add the login item.
    case registerFailed(underlying: any Error)
    /// The app has no humatad inside it.
    case missingExecutable(path: String)
    /// humatad couldn't be run.
    case launchFailed(path: String, underlying: any Error)
    /// humatad was started but nothing answered on the socket in time.
    case noAnswer(socket: String, seconds: Int)
    /// A humatad the app didn't start answers, so the app can't stop it to
    /// start its own.
    case notManaged(socket: String)
    /// macOS didn't take the login item off, so its humatad still runs.
    case unregisterFailed(underlying: any Error)
    /// A daemon still answers after it was asked to stop, or when nothing
    /// here could stop it; `seconds` is how long it was waited for.
    case stillRunning(seconds: Int)

    package var errorDescription: String? { description }

    package var description: String {
        switch self {
        case .needsApproval:
            "Humata's daemon is turned off in Login Items. Turn on Humata in System Settings › General › Login "
                + "Items, then try again."
        case .registerFailed(let underlying):
            "macOS didn't add Humata's daemon to Login Items: \(underlying.localizedDescription) Move Humata to the "
                + "Applications folder, open it from there, and try again."
        case .missingExecutable(let path):
            "This copy of Humata has no daemon at \(path). Download Humata again and replace this copy."
        case .launchFailed(let path, let underlying):
            "Couldn't run Humata's daemon at \(path): \(underlying.localizedDescription) Download Humata again and "
                + "replace this copy."
        case .noAnswer(let socket, let seconds):
            "Humata's daemon started but didn't answer at \(socket) within \(seconds) seconds. Check that your "
                + "login shell starts without waiting for input, since humatad reads its environment first. If "
                + "another humatad is running, quit it (for example with `pkill -x humatad`; its turns in progress "
                + "stop), then try again."
        case .notManaged(let socket):
            "Humata didn't start the humatad answering at \(socket), so it can't restart it, and it's still "
                + "running. Stop it yourself (for example with `pkill -x humatad`; its turns in progress stop) and "
                + "Humata starts its own: within a few seconds, or when you press Try Again if that's offered."
        case .unregisterFailed(let underlying):
            "macOS didn't stop Humata's daemon: \(underlying.localizedDescription) Turn off Humata in System "
                + "Settings › General › Login Items, then try again; or choose Run from Here."
        case .stillRunning(let seconds):
            "Humata's daemon is still running" + (seconds > 0 ? " after \(seconds) seconds" : "")
                + ", so nothing in Applications was changed. Quit every other copy of Humata that's open; if the "
                + "daemon comes back, turn Humata off in System Settings › General › Login Items. Then try again."
        }
    }
}
