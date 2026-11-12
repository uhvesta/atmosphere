import ComposableArchitecture
import Foundation

/// Keeps humatad running for the app: as a login item when the app is signed
/// by a team, otherwise as the app's own child, stopped when it quits.
@DependencyClient
package struct DaemonService: Sendable {
    /// Starts humatad unless one answers already, and waits until it does.
    package var start: @Sendable () async throws -> Void
    /// Whether the daemon answering is the build inside this app.
    package var isCurrent: @Sendable () async throws -> Bool = { true }
    /// Stops the daemon and starts this app's, waiting until it answers.
    /// Turns in progress end.
    package var restart: @Sendable () async throws -> Void
    /// Opens Login Items in System Settings.
    package var openLoginItems: @Sendable () -> Void
}

extension DaemonService: TestDependencyKey {
    package static let testValue = DaemonService()
    package static let previewValue = DaemonService(
        start: {}, isCurrent: { true }, restart: {}, openLoginItems: {})
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
    /// A humatad the app didn't start answers, from another build, so the app
    /// can't replace it.
    case notManaged(socket: String)

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
            "Humata's daemon started but didn't answer at \(socket) within \(seconds) seconds. If another humatad "
                + "is running, quit it (for example with `pkill -x humatad`; its turns in progress stop), then try "
                + "again."
        case .notManaged(let socket):
            "A humatad from another build of Humata, which this app didn't start, is answering at \(socket). Quit "
                + "it (for example with `pkill -x humatad`; its turns in progress stop), then try again."
        }
    }
}
