import ComposableArchitecture
import Foundation
import LifrCore

#if canImport(UIKit)
    import AudioToolbox
    import UIKit
#elseif canImport(AppKit)
    import AppKit
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

/// Plays haptics, and a soft sound for a task closed as Done when the
/// person turned it on. The system's haptics setting decides whether any
/// are felt, and the iPhone's silent switch silences the sound.
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
                if feedback == .success, Self.playsCompletionSound {
                    // The keyboard's soft click, which the silent switch mutes.
                    AudioServicesPlaySystemSound(1104)
                }
                await MainActor.run {
                    switch feedback {
                    case .success: UINotificationFeedbackGenerator().notificationOccurred(.success)
                    case .warning: UINotificationFeedbackGenerator().notificationOccurred(.warning)
                    case .tap: UIImpactFeedbackGenerator(style: .light).impactOccurred()
                    }
                }
            }
        #elseif canImport(AppKit)
            return Self { feedback in
                guard feedback == .success, Self.playsCompletionSound else { return }
                await MainActor.run { _ = NSSound(named: "Pop")?.play() }
            }
        #else
            return Self()
        #endif
    }()
    package static let testValue = Self()

    private static var playsCompletionSound: Bool {
        @Shared(.completionSound) var on
        return on
    }
}

extension SharedKey where Self == AppStorageKey<Bool>.Default {
    /// Whether finishing a task plays a soft sound. This device only; off
    /// unless the person turns it on.
    package static var completionSound: Self {
        Self[.appStorage("lifrCompletionSound"), default: false]
    }
}

extension DependencyValues {
    package var feedback: FeedbackClient {
        get { self[FeedbackClient.self] }
        set { self[FeedbackClient.self] = newValue }
    }
}
