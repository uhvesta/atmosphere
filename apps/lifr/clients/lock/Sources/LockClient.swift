import ComposableArchitecture
import Foundation

/// How this device confirms it's the person: what Settings names the lock
/// after.
package enum Biometry: Hashable, Sendable {
    case faceID, touchID, opticID
    /// No biometrics set up: the device's passcode, or the Mac's password.
    case passcode
}

/// Why the app couldn't be unlocked, or the lock turned on or off.
package enum LockError: Error, Hashable, Sendable, CustomStringConvertible {
    /// The person closed the prompt. Nothing to report.
    case cancelled
    /// The device has no passcode, so there's nothing to confirm with.
    case noPasscode
    /// Confirming failed for another reason, which the system gave.
    case failed(String)

    package var description: String {
        switch self {
        case .cancelled:
            String(localized: "Unlocking was cancelled. Tap Unlock to try again.")
        case .noPasscode:
            #if os(macOS)
                String(
                    localized: """
                        This Mac has no login password, so Lifr can't lock. Set one in System Settings › Users & \
                        Groups, then turn the lock on.
                        """)
            #else
                String(
                    localized: """
                        This iPhone has no passcode, so Lifr can't lock. Set one in Settings › Face ID & Passcode, \
                        then turn the lock on.
                        """)
            #endif
        case .failed(let reason):
            String(localized: "Couldn't confirm it's you (\(reason)). Try again.")
        }
    }
}

/// Confirms it's the person with Face ID, Touch ID or the device's
/// passcode, before Lifr shows anything.
@DependencyClient
package struct LockClient: Sendable {
    /// What this device confirms with.
    package var biometry: @Sendable () -> Biometry = { .passcode }
    /// Asks the person to confirm, saying `reason`. Throws `LockError`.
    package var authenticate: @Sendable (_ reason: String) async throws -> Void

    /// Confirms every time with `biometry`, or always fails with `error`.
    package static func always(_ biometry: Biometry = .faceID, failing error: LockError? = nil) -> Self {
        LockClient(
            biometry: { biometry },
            authenticate: { _ in
                if let error { throw error }
            })
    }

    /// Confirms or fails in turn as `attempts` says, then confirms.
    package static func scripted(_ attempts: [LockError?], biometry: Biometry = .faceID) -> Self {
        let attempts = LockIsolated(attempts)
        return LockClient(
            biometry: { biometry },
            authenticate: { _ in
                let next = attempts.withValue { $0.isEmpty ? nil : $0.removeFirst() }
                if let next { throw next }
            })
    }
}

extension LockClient: TestDependencyKey {
    /// Settings asks what the lock is called whenever it opens, so that
    /// answers; confirming is up to each test.
    package static let testValue = {
        var client = LockClient()
        client.biometry = { .faceID }
        return client
    }()
    package static let previewValue = LockClient.always()
}

extension DependencyValues {
    package var lock: LockClient {
        get { self[LockClient.self] }
        set { self[LockClient.self] = newValue }
    }
}

extension SharedKey where Self == AppStorageKey<Bool>.Default {
    /// Whether Lifr asks for Face ID, Touch ID or the passcode when it
    /// opens. This device only; off unless the person turns it on.
    package static var locksApp: Self {
        Self[.appStorage("lifrLocksApp"), default: false]
    }
}

/// When an open app locks again.
package enum AppLock {
    /// How long Lifr can be away before it asks again: long enough to copy
    /// something from another app, short enough that a phone left unlocked
    /// on a table doesn't show the records.
    package static let grace: TimeInterval = 60

    /// Whether Lifr should lock on coming back at `now`, after going away
    /// at `away`.
    package static func locks(away: Date?, now: Date) -> Bool {
        guard let away else { return false }
        return now.timeIntervalSince(away) >= grace
    }
}

/// Its description is written for the person, so alerts show it.
extension LockError: LocalizedError {
    package var errorDescription: String? { description }
}
