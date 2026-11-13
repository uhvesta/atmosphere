import AppKit
import ComposableArchitecture
import SwiftUI

/// Where the terminals' style is kept, and which fonts suit a terminal.
package struct TerminalStyleClient: Sendable {
    package var load: @Sendable () -> TerminalStyle
    package var save: @Sendable (_ style: TerminalStyle) -> Void
    /// The installed families whose letters are all one width, by name.
    package var fontFamilies: @MainActor @Sendable () -> [String]

    package init(
        load: @escaping @Sendable () -> TerminalStyle,
        save: @escaping @Sendable (_ style: TerminalStyle) -> Void,
        fontFamilies: @escaping @MainActor @Sendable () -> [String]
    ) {
        self.load = load
        self.save = save
        self.fontFamilies = fontFamilies
    }
}

extension TerminalStyleClient: DependencyKey {
    /// The app's own defaults, which the terminals' runtime watches, and
    /// the fonts the Mac has.
    package static var liveValue: TerminalStyleClient {
        TerminalStyleClient(
            load: { TerminalStyle(defaults: .standard) },
            save: { $0.save(to: .standard) },
            fontFamilies: {
                let names = NSFontManager.shared.availableFontNames(with: .fixedPitchFontMask) ?? []
                let families = Set(names.compactMap { NSFont(name: $0, size: 12)?.familyName })
                // A leading dot marks the system's private fonts.
                return families.filter { !$0.hasPrefix(".") }.sorted {
                    $0.localizedCaseInsensitiveCompare($1) == .orderedAscending
                }
            })
    }

    /// The default style, kept nowhere.
    package static var testValue: TerminalStyleClient {
        TerminalStyleClient(load: { TerminalStyle() }, save: { _ in }, fontFamilies: { ["Menlo"] })
    }

    package static var previewValue: TerminalStyleClient { testValue }
}

extension DependencyValues {
    package var terminalStyle: TerminalStyleClient {
        get { self[TerminalStyleClient.self] }
        set { self[TerminalStyleClient.self] = newValue }
    }
}

/// Settings › Terminal: the font, its size and the color theme every
/// terminal is drawn with. Each change is kept at once, and open terminals
/// follow it.
@Reducer
package struct TerminalSettingsFeature {
    @ObservableState
    package struct State: Equatable {
        package var style = TerminalStyle()
        /// The fonts to pick from, once listed.
        package var fontFamilies: [String] = []

        package init() {}
    }

    package enum Action {
        /// The pane appeared: reads the kept style and lists the fonts.
        case task
        case fontFamiliesListed([String])
        case fontFamilyPicked(String)
        case fontSizeChanged(Double)
        case themePicked(TerminalTheme.ID)
    }

    @Dependency(\.terminalStyle) var terminalStyle

    package init() {}

    package var body: some ReducerOf<Self> {
        Reduce { state, action in
            switch action {
            case .task:
                state.style = terminalStyle.load()
                return .run { send in
                    await send(.fontFamiliesListed(await terminalStyle.fontFamilies()))
                }

            case .fontFamiliesListed(let families):
                // The kept font stays offered even when it isn't installed
                // now, so the picker shows what's in use.
                state.fontFamilies =
                    families.contains(state.style.fontFamily) ? families : [state.style.fontFamily] + families
                return .none

            case .fontFamilyPicked(let family):
                guard !family.isEmpty else { return .none }
                state.style.fontFamily = family
                return save(state)

            case .fontSizeChanged(let size):
                let sizes = TerminalStyle.fontSizes
                state.style.fontSize = min(max(size.rounded(), sizes.lowerBound), sizes.upperBound)
                return save(state)

            case .themePicked(let id):
                guard let theme = TerminalTheme.all.first(where: { $0.id == id }) else { return .none }
                state.style.theme = theme
                return save(state)
            }
        }
    }

    private func save(_ state: State) -> Effect<Action> {
        let style = state.style
        return .run { _ in terminalStyle.save(style) }
    }
}

/// Settings › Terminal.
package struct TerminalSettingsView: View {
    @State private var store = Store(initialState: TerminalSettingsFeature.State()) {
        TerminalSettingsFeature()
    }

    package init() {}

    package var body: some View {
        Form {
            Section("Font") {
                Picker(
                    "Family",
                    selection: Binding(
                        get: { store.style.fontFamily }, set: { store.send(.fontFamilyPicked($0)) })
                ) {
                    // Before the fonts are listed, the one in use alone.
                    ForEach(store.fontFamilies.isEmpty ? [store.style.fontFamily] : store.fontFamilies, id: \.self) {
                        Text($0).tag($0)
                    }
                }
                .help("Only fonts whose letters are all one width are listed.")
                Stepper(
                    "Size: \(Int(store.style.fontSize)) pt",
                    value: Binding(get: { store.style.fontSize }, set: { store.send(.fontSizeChanged($0)) }),
                    in: TerminalStyle.fontSizes, step: 1)
            }
            Section("Colors") {
                Picker(
                    "Theme",
                    selection: Binding(get: { store.style.theme.id }, set: { store.send(.themePicked($0)) })
                ) {
                    ForEach(TerminalTheme.all) { Text($0.name).tag($0.id) }
                }
                .help("Each theme has light and dark colors, and follows the Mac's appearance.")
                HStack(spacing: 12) {
                    PaletteSample(title: "Light", palette: store.style.theme.light, style: store.style)
                    PaletteSample(title: "Dark", palette: store.style.theme.dark, style: store.style)
                }
            }
        }
        .formStyle(.grouped)
        .task { await store.send(.task).finish() }
    }
}

/// A few words and the ANSI colors as one appearance draws them.
private struct PaletteSample: View {
    let title: String
    let palette: TerminalStyle.Palette
    let style: TerminalStyle

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("\(title)  $ ls -la")
                .font(.custom(style.fontFamily, size: style.fontSize))
                .foregroundStyle(Color(hex: palette.foreground))
            HStack(spacing: 3) {
                ForEach(Array(palette.colors.prefix(16).enumerated()), id: \.offset) { _, color in
                    RoundedRectangle(cornerRadius: 2)
                        .fill(Color(hex: color))
                        .frame(width: 12, height: 12)
                }
            }
        }
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color(hex: palette.background), in: RoundedRectangle(cornerRadius: 6))
        .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(.separator))
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(title) colors")
    }
}

extension Color {
    /// `#rrggbb` as a color; gray when it isn't one.
    fileprivate init(hex: String) {
        guard hex.hasPrefix("#"), hex.count == 7, let value = UInt32(hex.dropFirst(), radix: 16) else {
            self = .gray
            return
        }
        self.init(
            red: Double(value >> 16 & 0xFF) / 255, green: Double(value >> 8 & 0xFF) / 255,
            blue: Double(value & 0xFF) / 255)
    }
}
