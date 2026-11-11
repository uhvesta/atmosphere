import ComposableArchitecture
import Foundation
import LifrCore

#if canImport(UIKit)
    import UIKit
#endif

/// A haptic for something the person did.
package enum Feedback: Hashable, Sendable {
    /// A task closed as Done.
    case success
    /// Any other status, a drop, a snap into place or an undo.
    case tap
    /// A change that was refused.
    case warning

    /// What picking `status` feels like.
    package init(status: TaskStatus) {
        self = status == .done ? .success : .tap
    }
}

/// Plays haptics. The system's haptics setting decides whether any are
/// felt.
package struct FeedbackClient: Sendable {
    package var play: @Sendable (Feedback) async -> Void = { _ in }

    package init(play: @escaping @Sendable (Feedback) async -> Void = { _ in }) {
        self.play = play
    }

    package func callAsFunction(_ feedback: Feedback) async {
        await play(feedback)
    }

    /// Records what was played, for tests.
    package static func recording() -> (client: Self, played: LockIsolated<[Feedback]>) {
        let played = LockIsolated<[Feedback]>([])
        return (Self { feedback in played.withValue { $0.append(feedback) } }, played)
    }
}

extension FeedbackClient: DependencyKey {
    package static let liveValue: Self = {
        #if canImport(UIKit) && !os(watchOS)
            return Self { feedback in
                await MainActor.run {
                    switch feedback {
                    case .success: UINotificationFeedbackGenerator().notificationOccurred(.success)
                    case .warning: UINotificationFeedbackGenerator().notificationOccurred(.warning)
                    case .tap: UIImpactFeedbackGenerator(style: .light).impactOccurred()
                    }
                }
            }
        #else
            return Self()
        #endif
    }()
    package static let testValue = Self()
}

extension DependencyValues {
    package var feedback: FeedbackClient {
        get { self[FeedbackClient.self] }
        set { self[FeedbackClient.self] = newValue }
    }
}
