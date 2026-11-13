import ComposableArchitecture
import Foundation
import LifrGlance
import LifrWatchLink

/// The Watch's end of the iPhone: Today as the iPhone sends it, and the
/// changes the Watch asks it to save.
@DependencyClient
package struct WatchClient: Sendable {
    /// Today as the Watch last kept it, for showing at once on opening.
    package var saved: @Sendable () -> TodayGlance? = { nil }
    /// Today each time the iPhone sends it.
    package var glances: @Sendable () -> AsyncStream<TodayGlance> = { .finished }
    /// Queues a change for the iPhone to save.
    package var send: @Sendable (WatchRequest) async throws -> Void
}

extension WatchClient {
    /// Sends each of `glances` once asked, and keeps each request sent,
    /// for tests.
    package static func inMemory(
        saved: TodayGlance? = nil, glances: [TodayGlance] = []
    ) -> (client: Self, sent: LockIsolated<[WatchRequest]>) {
        let sent = LockIsolated<[WatchRequest]>([])
        return (
            WatchClient(
                saved: { saved },
                glances: {
                    AsyncStream { continuation in
                        for glance in glances { continuation.yield(glance) }
                        continuation.finish()
                    }
                },
                send: { request in sent.withValue { $0.append(request) } }),
            sent
        )
    }
}

extension WatchClient: TestDependencyKey {
    package static let testValue = WatchClient()
    package static let previewValue = WatchClient.inMemory().client
}

extension DependencyValues {
    package var watch: WatchClient {
        get { self[WatchClient.self] }
        set { self[WatchClient.self] = newValue }
    }
}
