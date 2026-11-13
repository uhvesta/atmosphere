import ComposableArchitecture
import Foundation
import LifrGlance

#if os(iOS)
    import ActivityKit
#endif

/// Why a Live Activity couldn't start.
package enum FocusActivityError: Error, CustomStringConvertible {
    /// Live Activities are off for Lifr, or this device has none.
    case unavailable
    /// The system refused it.
    case refused(any Error)

    package var description: String {
        switch self {
        case .unavailable:
            String(
                localized: "Live Activities are off for Lifr. Turn them on in Settings › Lifr, then try again.")
        case .refused(let error):
            String(
                localized: """
                    The lock screen couldn't show the task (\(error.localizedDescription)). \
                    End another app's Live Activity and try again.
                    """)
        }
    }
}

/// The Live Activity for a task the person is focusing on: started from
/// its page, kept up to date as it changes, and ended once it's no longer
/// In Progress.
@DependencyClient
package struct FocusActivityClient: Sendable {
    /// Whether this device can show one: an iPhone with Live Activities on.
    package var isAvailable: @Sendable () -> Bool = { false }
    package var start: @Sendable (_ task: UUID, _ state: FocusedTask) async throws -> Void
    /// The tasks that have one.
    package var running: @Sendable () async -> Set<UUID> = { [] }
    /// Shows `state`, or ends the activity when it's nil.
    package var update: @Sendable (_ task: UUID, _ state: FocusedTask?) async -> Void
}

extension FocusActivityClient: DependencyKey {
    package static var liveValue: FocusActivityClient {
        #if os(iOS)
            FocusActivityClient(
                isAvailable: { ActivityAuthorizationInfo().areActivitiesEnabled },
                start: { task, state in
                    guard ActivityAuthorizationInfo().areActivitiesEnabled else { throw FocusActivityError.unavailable }
                    // One at a time: focusing on another task ends the last.
                    for activity in Activity<FocusAttributes>.activities {
                        await activity.end(nil, dismissalPolicy: .immediate)
                    }
                    do {
                        let content = ActivityContent(state: state, staleDate: nil)
                        _ = try Activity.request(attributes: FocusAttributes(taskID: task), content: content)
                    } catch {
                        throw FocusActivityError.refused(error)
                    }
                },
                running: { Set(Activity<FocusAttributes>.activities.map(\.attributes.taskID)) },
                update: { task, state in
                    for activity in Activity<FocusAttributes>.activities where activity.attributes.taskID == task {
                        if let state {
                            guard activity.content.state != state else { continue }
                            await activity.update(ActivityContent(state: state, staleDate: nil))
                        } else {
                            await activity.end(nil, dismissalPolicy: .immediate)
                        }
                    }
                })
        #else
            FocusActivityClient(isAvailable: { false }, start: { _, _ in }, running: { [] }, update: { _, _ in })
        #endif
    }

    /// Live Activities aren't part of most tests, so there are none; tests
    /// of them use `inMemory`.
    package static let testValue = FocusActivityClient(
        isAvailable: { false }, start: { _, _ in }, running: { [] }, update: { _, _ in })
    package static let previewValue = testValue

    /// Activities kept in memory, for tests: what's showing for each task.
    package static func inMemory(available: Bool = true) -> (
        client: FocusActivityClient, showing: LockIsolated<[UUID: FocusedTask]>
    ) {
        let showing = LockIsolated<[UUID: FocusedTask]>([:])
        let client = FocusActivityClient(
            isAvailable: { available },
            start: { task, state in
                guard available else { throw FocusActivityError.unavailable }
                showing.setValue([task: state])
            },
            running: { Set(showing.value.keys) },
            update: { task, state in showing.withValue { $0[task] = state } })
        return (client, showing)
    }
}

extension DependencyValues {
    package var focusActivity: FocusActivityClient {
        get { self[FocusActivityClient.self] }
        set { self[FocusActivityClient.self] = newValue }
    }
}
