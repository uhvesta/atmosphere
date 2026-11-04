import ComposableArchitecture
import Foundation

#if canImport(UIKit)
    import UIKit
#endif

/// Keeps work going after the person leaves Lifr, for as long as the
/// system allows. iOS gives an app about 30 seconds in the background;
/// the Mac doesn't suspend apps, so there it just runs the work.
struct BackgroundTime: Sendable {
    var run: @Sendable (_ name: String, _ work: @Sendable () async throws -> Void) async throws -> Void
}

extension BackgroundTime: DependencyKey {
    static let liveValue: BackgroundTime = {
        #if canImport(UIKit)
            BackgroundTime { name, work in
                let token = await BackgroundToken.begin(name)
                do {
                    try await work()
                } catch {
                    await token.end()
                    throw error
                }
                await token.end()
            }
        #else
            BackgroundTime { _, work in try await work() }
        #endif
    }()

    static let testValue = BackgroundTime { _, work in try await work() }
}

#if canImport(UIKit)
    /// One request for background time, ended when the work finishes or
    /// the time runs out, whichever is first.
    @MainActor
    private final class BackgroundToken {
        private var id = UIBackgroundTaskIdentifier.invalid

        static func begin(_ name: String) -> BackgroundToken {
            let token = BackgroundToken()
            token.id = UIApplication.shared.beginBackgroundTask(withName: name) {
                MainActor.assumeIsolated { token.end() }
            }
            return token
        }

        func end() {
            guard id != .invalid else { return }
            UIApplication.shared.endBackgroundTask(id)
            id = .invalid
        }
    }
#endif

extension DependencyValues {
    var backgroundTime: BackgroundTime {
        get { self[BackgroundTime.self] }
        set { self[BackgroundTime.self] = newValue }
    }
}
