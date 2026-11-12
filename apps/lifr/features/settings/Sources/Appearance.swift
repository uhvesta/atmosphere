import ComposableArchitecture
import LifrCore
import LifrUI
import SwiftUI

/// The app's icon on this iPhone: the standard one or a recolored one.
package enum AppIconChoice: String, CaseIterable, Sendable {
    case standard, sunset, forest, graphite

    /// The icon's name in the app's asset catalog; nil for the primary one.
    var iconName: String? {
        switch self {
        case .standard: nil
        case .sunset: "AppIconSunset"
        case .forest: "AppIconForest"
        case .graphite: "AppIconGraphite"
        }
    }

    var title: String {
        switch self {
        case .standard: String(localized: "Ocean")
        case .sunset: String(localized: "Sunset")
        case .forest: String(localized: "Forest")
        case .graphite: String(localized: "Graphite")
        }
    }

    /// The icon's background, from its top corner to its bottom one.
    var gradient: [Color] {
        switch self {
        case .standard: [Color(red: 0.11, green: 0.71, blue: 0.67), Color(red: 0.25, green: 0.32, blue: 0.85)]
        case .sunset: [Color(red: 0.85, green: 0.55, blue: 0.80), Color(red: 0.72, green: 0.30, blue: 0.10)]
        case .forest: [Color(red: 0.70, green: 0.70, blue: 0.10), Color(red: 0.05, green: 0.60, blue: 0.15)]
        case .graphite: [Color(white: 0.65), Color(white: 0.35)]
        }
    }

    init(iconName: String?) {
        self = Self.allCases.first { $0.iconName == iconName } ?? .standard
    }
}

extension SharedKey where Self == AppStorageKey<String>.Default {
    /// The color buttons and selections take on this device, by an Avenue
    /// color's name; empty for the system's own.
    package static var accentColor: Self {
        Self[.appStorage("lifrAccentColor"), default: ""]
    }
}

/// Tints a window's controls in the accent the person picked on this
/// device, or leaves the system's.
package struct AccentTint: ViewModifier {
    @Shared(.accentColor) private var accent

    package init() {}

    package func body(content: Content) -> some View {
        if accent.isEmpty {
            content
        } else {
            content.tint(Color(avenue: accent))
        }
    }
}

/// The accent color and, on iPhone, the app icon. Both are this device's
/// only.
struct AppearanceSection: View {
    @Shared(.accentColor) private var accent
    #if os(iOS)
        @State private var icon = AppIconChoice(iconName: UIApplication.shared.alternateIconName)
        @State private var iconError: String?
    #endif

    var body: some View {
        Section {
            // A menu draws its images in one color, so the row's icon shows
            // the accent picked.
            Picker(selection: Binding($accent)) {
                Text("Default").tag("")
                ForEach(Avenue.colors, id: \.self) { Text($0.capitalized).tag($0) }
            } label: {
                Label {
                    Text("Accent color")
                } icon: {
                    Image(systemName: "paintpalette.fill")
                        .foregroundStyle(accent.isEmpty ? Color.accentColor : Color(avenue: accent))
                }
            }
        } header: {
            Text("Appearance")
        } footer: {
            #if os(macOS)
                Text("This Mac only.")
            #endif
        }
        #if os(iOS)
            Section {
                Picker("App icon", selection: $icon) {
                    ForEach(AppIconChoice.allCases, id: \.self) { choice in
                        Label {
                            Text(choice.title)
                        } icon: {
                            RoundedRectangle(cornerRadius: 6)
                                .fill(
                                    LinearGradient(
                                        colors: choice.gradient, startPoint: .topLeading, endPoint: .bottomTrailing)
                                )
                                .frame(width: 26, height: 26)
                                .accessibilityHidden(true)
                        }
                        .tag(choice)
                    }
                }
                .pickerStyle(.inline)
                .labelsHidden()
                .onChange(of: icon) { _, choice in
                    guard UIApplication.shared.alternateIconName != choice.iconName else { return }
                    Task {
                        do {
                            try await UIApplication.shared.setAlternateIconName(choice.iconName)
                            iconError = nil
                        } catch {
                            icon = AppIconChoice(iconName: UIApplication.shared.alternateIconName)
                            iconError = String(
                                localized: "Lifr couldn't change its icon: \(error.localizedDescription) Try again.")
                        }
                    }
                }
            } header: {
                Text("App Icon")
            } footer: {
                if let iconError {
                    Text(iconError).foregroundStyle(.red)
                } else {
                    Text("The accent and icon are for this iPhone only.")
                }
            }
        #endif
    }
}
