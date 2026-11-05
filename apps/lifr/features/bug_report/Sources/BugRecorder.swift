import ComposableArchitecture
import Foundation
import os

/// Records the steps for a bug report: every action with when it happened
/// and what it changed, the app's state when recording started, and the
/// screen as it changes.
package final class BugRecorder: Sendable {
    /// The most steps one recording keeps; later ones are counted.
    package static let stepLimit = 300
    /// The most screenshots one recording keeps.
    package static let screenshotLimit = 40
    /// How long after a change the screen is taken, so it shows the change.
    static let settle = Duration.milliseconds(350)
    /// The longest action or change kept, in characters.
    static let textLimit = 4_000

    private struct Session: Sendable {
        var recording: BugReport.Recording
        var capturing: Task<Void, Never>?
    }

    private let session = OSAllocatedUnfairLock<Session?>(initialState: nil)
    private let capture: @Sendable @MainActor () -> Data?
    private let clock: any Clock<Duration>
    private let now: @Sendable () -> Date

    /// - Parameter capture: The screen as a JPEG, or nil when there's
    ///   nothing to show.
    package init(
        capture: @escaping @Sendable @MainActor () -> Data? = { nil },
        clock: any Clock<Duration> = ContinuousClock(),
        now: @escaping @Sendable () -> Date = Date.init
    ) {
        self.capture = capture
        self.clock = clock
        self.now = now
    }

    package var isRecording: Bool { session.withLock { $0 != nil } }

    /// Starts recording from `state`, dropping any recording in progress.
    package func start(from state: some Any) {
        let recording = BugReport.Recording(startedAt: now(), start: Self.limit(String(customDumping: state)))
        let previous = session.withLock { session in
            defer { session = Session(recording: recording) }
            return session?.capturing
        }
        previous?.cancel()
        captureSoon()
    }

    /// Adds `action`, and what it changed from `before` to `after`. Does
    /// nothing unless recording.
    package func log<State>(_ action: some Any, from before: State, to after: State) {
        guard isRecording else { return }
        let changes = diff(before, after).map(Self.limit)
        let offset = offset()
        let added = session.withLock { session -> Bool in
            guard session != nil else { return false }
            guard session?.recording.steps.count ?? 0 < Self.stepLimit else {
                session?.recording.dropped += 1
                return false
            }
            session?.recording.steps.append(
                BugReport.Step(offset: offset, action: Self.limit(String(customDumping: action)), changes: changes))
            return true
        }
        if added, changes != nil { captureSoon() }
    }

    /// Ends the recording and returns it, or nil when there wasn't one.
    package func stop() -> BugReport.Recording? {
        let ended = session.withLock { session in
            defer { session = nil }
            return session
        }
        ended?.capturing?.cancel()
        return ended?.recording
    }

    /// Takes the screen once it has settled, unless a screenshot is already
    /// on its way or there are enough.
    private func captureSoon() {
        session.withLock { current in
            guard let running = current, running.capturing == nil,
                running.recording.screenshots.count < Self.screenshotLimit
            else { return }
            current?.capturing = Task { [weak self] in
                guard let self else { return }
                try? await clock.sleep(for: Self.settle)
                guard !Task.isCancelled else { return }
                let image = await capture()
                let offset = offset()
                self.session.withLock { session in
                    guard session != nil else { return }
                    session?.capturing = nil
                    guard let image, let steps = session?.recording.steps.count else { return }
                    session?.recording.screenshots.append(
                        BugReport.Screenshot(offset: offset, afterSteps: steps, image: image))
                }
            }
        }
    }

    private func offset() -> TimeInterval {
        let startedAt = session.withLock { $0?.recording.startedAt } ?? now()
        return max(0, now().timeIntervalSince(startedAt))
    }

    private static func limit(_ text: String) -> String {
        guard text.count > textLimit else { return text }
        return String(text.prefix(textLimit)) + "\n… \(text.count - textLimit) more characters"
    }
}

/// Records each action `base` handles while a recording is running.
package struct BugRecording<Base: Reducer>: Reducer {
    let base: Base
    @Dependency(\.bugRecorder) private var recorder

    init(base: Base) {
        self.base = base
    }

    package func reduce(into state: inout Base.State, action: Base.Action) -> Effect<Base.Action> {
        guard recorder.isRecording else { return base.reduce(into: &state, action: action) }
        let before = state
        let effect = base.reduce(into: &state, action: action)
        recorder.log(action, from: before, to: state)
        return effect
    }
}

extension Reducer {
    /// Logs each action this reducer runs while steps are being recorded.
    package func recordingBugs() -> BugRecording<Self> {
        BugRecording(base: self)
    }
}

extension BugRecorder: DependencyKey {
    package static let liveValue = BugRecorder(capture: { Screen.capture() })
    package static var testValue: BugRecorder { BugRecorder() }
}

extension DependencyValues {
    package var bugRecorder: BugRecorder {
        get { self[BugRecorder.self] }
        set { self[BugRecorder.self] = newValue }
    }
}
