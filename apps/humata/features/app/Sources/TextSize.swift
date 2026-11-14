import ChatUI
import ComposableArchitecture
import Foundation
import HumataTerminal

/// Where the window's text scale is kept from one run to the next.
package struct TextScaleKeeper: Sendable {
    package var load: @Sendable () -> TextScale
    package var save: @Sendable (_ scale: TextScale) -> Void

    package init(load: @escaping @Sendable () -> TextScale, save: @escaping @Sendable (_ scale: TextScale) -> Void) {
        self.load = load
        self.save = save
    }

    /// The scale kept in `defaults`; the usual size when none is, and the
    /// nearest there is to one that isn't a scale.
    package static func read(from defaults: UserDefaults) -> TextScale {
        // None kept reads as 0.
        let percent = defaults.integer(forKey: TerminalStyle.textPercentKey)
        return percent > 0 ? TextScale(percent: percent) : .actual
    }

    /// Keeps `scale` in `defaults`, under the key the terminals read their
    /// font's scale from.
    package static func write(_ scale: TextScale, to defaults: UserDefaults) {
        defaults.set(scale.percent, forKey: TerminalStyle.textPercentKey)
    }
}

extension TextScaleKeeper: DependencyKey {
    /// The app's own defaults, which the terminals' runtime watches, so
    /// their font follows the scale as it's kept.
    package static var liveValue: TextScaleKeeper {
        TextScaleKeeper(load: { read(from: .standard) }, save: { write($0, to: .standard) })
    }

    /// The usual size, kept nowhere.
    package static var testValue: TextScaleKeeper {
        TextScaleKeeper(load: { .actual }, save: { _ in })
    }

    package static var previewValue: TextScaleKeeper { testValue }
}

extension DependencyValues {
    package var textScaleKeeper: TextScaleKeeper {
        get { self[TextScaleKeeper.self] }
        set { self[TextScaleKeeper.self] = newValue }
    }
}

/// The View menu's Bigger Text, Smaller Text and Actual Size: how big the
/// text of the chat, the message box, the review, the files, the spec and
/// the terminals is drawn. Each change is kept at once.
@Reducer
package struct TextSizeFeature {
    @ObservableState
    package struct State: Equatable {
        /// The app sets it from what was kept before the window shows, so
        /// the window never opens at one size and jumps to another.
        package var scale = TextScale.actual

        package init() {}
    }

    package enum Action {
        case biggerTapped
        case smallerTapped
        case actualSizeTapped
    }

    @Dependency(\.textScaleKeeper) var keeper

    package init() {}

    package var body: some ReducerOf<Self> {
        Reduce { state, action in
            let scale: TextScale
            switch action {
            case .biggerTapped: scale = state.scale.bigger
            case .smallerTapped: scale = state.scale.smaller
            case .actualSizeTapped: scale = .actual
            }
            // At a limit, or already the usual size: nothing to keep.
            guard scale != state.scale else { return .none }
            state.scale = scale
            return .run { _ in keeper.save(scale) }
        }
    }
}
