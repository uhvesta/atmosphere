import ComposableArchitecture
import Foundation
import LifrModelsClient

#if os(iOS)
    import BackgroundTasks
    import UIKit
#endif

/// Keeps work going after the person leaves Lifr, for as long as the
/// system allows. iOS gives an app about 30 seconds in the background, or,
/// when the person lets the model answer there, as long as the work moves
/// on, with its progress on the Lock Screen. The Mac doesn't suspend apps,
/// so there it just runs the work.
package struct BackgroundTime: Sendable {
    /// Runs `work`, which calls `progressed` each time it moves on.
    package var run:
        @Sendable (
            _ title: String, _ work: @Sendable (_ progressed: @escaping @Sendable () -> Void) async throws -> Void
        )
            async throws -> Void
    /// How many times Lifr has come back on screen: work waiting in the
    /// background doesn't count as stalled.
    package var returns: @Sendable () -> Int

    package init(
        run:
            @escaping @Sendable (
                _ title: String, _ work: @Sendable (_ progressed: @escaping @Sendable () -> Void) async throws -> Void
            ) async throws -> Void,
        returns: @escaping @Sendable () -> Int
    ) {
        self.run = run
        self.returns = returns
    }
}

extension BackgroundTime: DependencyKey {
    package static let liveValue: BackgroundTime = {
        #if os(iOS)
            let returns = Returns()
            return BackgroundTime(
                run: { title, work in
                    let token = await BackgroundToken.begin(
                        title, continued: Shared(.answersInBackground).wrappedValue)
                    do {
                        try await work { token.progressed() }
                    } catch {
                        await token.end()
                        throw error
                    }
                    await token.end()
                },
                returns: { returns.count.value })
        #else
            BackgroundTime(run: { _, work in try await work {} }, returns: { 0 })
        #endif
    }()

    package static let testValue = BackgroundTime(run: { _, work in try await work {} }, returns: { 0 })
}

#if os(iOS)
    /// Counts each time Lifr comes back on screen.
    private final class Returns: Sendable {
        let count = LockIsolated(0)
        private let observer: LockIsolated<(any NSObjectProtocol)?> = LockIsolated(nil)

        init() {
            let count = count
            observer.setValue(
                NotificationCenter.default.addObserver(
                    forName: UIApplication.willEnterForegroundNotification, object: nil, queue: nil
                ) { _ in count.withValue { $0 += 1 } })
        }
    }

    /// One request for background time, ended when the work finishes or
    /// the time runs out, whichever is first: a continued-processing task
    /// when it's `continued` and the system starts one, which keeps Lifr
    /// running while the work moves on, or else the usual 30 seconds.
    private final class BackgroundToken: Sendable {
        private struct State {
            var id = UIBackgroundTaskIdentifier.invalid
            /// The continued-processing task, on iOS 26 and later.
            var task: AnyObject?
            var ended = false
        }

        /// The steps a task's progress counts in. The work's length isn't
        /// known, so each step takes it part of the way left.
        private static let steps: Int64 = 1_000

        private let state = LockIsolated(State())

        @MainActor
        static func begin(_ title: String, continued: Bool) async -> BackgroundToken {
            let token = BackgroundToken()
            let id = UIApplication.shared.beginBackgroundTask(withName: title) {
                MainActor.assumeIsolated { token.endTime() }
            }
            token.state.withValue { $0.id = id }
            if continued, #available(iOS 26.0, *) { token.continueProcessing(title) }
            return token
        }

        /// Asks for a continued-processing task, which the system runs at
        /// once or not at all.
        @available(iOS 26.0, *)
        @MainActor
        private func continueProcessing(_ title: String) {
            guard let bundle = Bundle.main.bundleIdentifier else { return }
            let identifier = "\(bundle).continued.\(UUID().uuidString)"
            let launch: @Sendable (BGTask) -> Void = { [self] task in
                guard let task = task as? BGContinuedProcessingTask else { return }
                task.progress.totalUnitCount = Self.steps
                task.expirationHandler = { [self] in end(task, success: false) }
                let ended = state.withValue { state in
                    if !state.ended { state.task = task }
                    return state.ended
                }
                if ended { task.setTaskCompleted(success: true) }
            }
            let registered = BGTaskScheduler.shared.register(
                forTaskWithIdentifier: identifier, using: nil, launchHandler: launch)
            guard registered else { return }
            let request = BGContinuedProcessingTaskRequest(
                identifier: identifier, title: title, subtitle: "Lifr keeps answering while you're away.")
            request.strategy = .fail
            try? BGTaskScheduler.shared.submit(request)
        }

        func progressed() {
            guard #available(iOS 26.0, *), let task = state.value.task as? BGContinuedProcessingTask else { return }
            let progress = task.progress
            progress.completedUnitCount = min(Self.steps - 1, progress.completedUnitCount + 1)
        }

        @MainActor
        func end() {
            endTime()
            let task = state.withValue { state in
                state.ended = true
                defer { state.task = nil }
                return state.task
            }
            if #available(iOS 26.0, *), let task = task as? BGContinuedProcessingTask { end(task, success: true) }
        }

        @available(iOS 26.0, *)
        private func end(_ task: BGContinuedProcessingTask, success: Bool) {
            task.progress.completedUnitCount = Self.steps
            task.setTaskCompleted(success: success)
            state.withValue { if $0.task === task { $0.task = nil } }
        }

        @MainActor
        private func endTime() {
            let id = state.withValue { state in
                defer { state.id = .invalid }
                return state.id
            }
            guard id != .invalid else { return }
            UIApplication.shared.endBackgroundTask(id)
        }
    }
#endif

extension DependencyValues {
    package var backgroundTime: BackgroundTime {
        get { self[BackgroundTime.self] }
        set { self[BackgroundTime.self] = newValue }
    }
}
