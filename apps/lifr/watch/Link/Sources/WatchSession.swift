#if canImport(WatchConnectivity)
    import Foundation
    import LifrGlance
    import OSLog
    import Synchronization
    import WatchConnectivity

    /// The iPhone's and the Watch's ends of the system's connection between
    /// them. The iPhone sends Today, the latest replacing any not yet
    /// delivered; the Watch sends requests, each delivered once, in order,
    /// even when the iPhone is away or Lifr isn't running there.
    package final class WatchSession: NSObject, WCSessionDelegate, Sendable {
        package static let shared = WatchSession()

        private struct Handlers {
            var request: (@Sendable (WatchRequest) async -> Void)?
            var glance: [UUID: @Sendable (TodayGlance) -> Void] = [:]
        }

        private let handlers = Mutex(Handlers())

        /// Starts the connection; on the iPhone, `onRequest` saves what the
        /// Watch asks for. Does nothing on a device with no Watch, such as
        /// an iPad.
        package func activate(onRequest: (@Sendable (WatchRequest) async -> Void)? = nil) {
            guard WCSession.isSupported() else { return }
            handlers.withLock { $0.request = onRequest }
            WCSession.default.delegate = self
            WCSession.default.activate()
        }

        #if os(iOS)
            /// Sends Today to the Watch, when one with Lifr is paired.
            package func send(_ glance: TodayGlance) {
                guard WCSession.isSupported() else { return }
                let session = WCSession.default
                guard session.activationState == .activated, session.isPaired, session.isWatchAppInstalled else {
                    return
                }
                do {
                    try session.updateApplicationContext([
                        WatchMessage.glanceKey: try WatchMessage.encode(glance.forWatch())
                    ])
                } catch {
                    Self.log.error("\(String(describing: error), privacy: .public)")
                }
            }
        #endif

        #if os(watchOS)
            /// Today as the iPhone last sent it, then each time it sends
            /// again.
            package func glances() -> AsyncStream<TodayGlance> {
                AsyncStream { continuation in
                    let id = UUID()
                    handlers.withLock { $0.glance[id] = { continuation.yield($0) } }
                    continuation.onTermination = { [weak self] _ in
                        self?.handlers.withLock { _ = $0.glance.removeValue(forKey: id) }
                    }
                    if WCSession.default.activationState == .activated,
                        let glance = Self.glance(in: WCSession.default.receivedApplicationContext)
                    {
                        continuation.yield(glance)
                    }
                }
            }

            /// Queues `request` for the iPhone, which saves it when it next
            /// can.
            package func send(_ request: WatchRequest) throws {
                WCSession.default.transferUserInfo([WatchMessage.requestKey: try WatchMessage.encode(request)])
            }
        #endif

        package func session(
            _ session: WCSession, activationDidCompleteWith activationState: WCSessionActivationState,
            error: (any Error)?
        ) {
            if let error { Self.log.error("\(String(describing: error), privacy: .public)") }
            #if os(watchOS)
                if let glance = Self.glance(in: session.receivedApplicationContext) { deliver(glance) }
            #endif
        }

        #if os(iOS)
            package func sessionDidBecomeInactive(_ session: WCSession) {}

            /// After the person switches to another Watch, connects to it.
            package func sessionDidDeactivate(_ session: WCSession) {
                session.activate()
            }

            package func session(_ session: WCSession, didReceiveUserInfo userInfo: [String: Any]) {
                let request: WatchRequest?
                do {
                    request = try WatchMessage.request(in: userInfo)
                } catch {
                    Self.log.error("\(error.description, privacy: .public)")
                    return
                }
                guard let request, let save = handlers.withLock({ $0.request }) else { return }
                Task { await save(request) }
            }
        #endif

        #if os(watchOS)
            package func session(_ session: WCSession, didReceiveApplicationContext context: [String: Any]) {
                if let glance = Self.glance(in: context) { deliver(glance) }
            }

            private func deliver(_ glance: TodayGlance) {
                for handler in handlers.withLock({ Array($0.glance.values) }) { handler(glance) }
            }

            private static func glance(in context: [String: Any]) -> TodayGlance? {
                do {
                    return try WatchMessage.glance(in: context)
                } catch {
                    log.error("\(error.description, privacy: .public)")
                    return nil
                }
            }
        #endif

        private static let log = Logger(subsystem: "lifr", category: "watch")
    }
#endif
