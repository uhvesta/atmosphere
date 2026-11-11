import SwiftUI

extension View {
    /// Lays out a sheet the same way on iPhone and Mac. A Mac sheet doesn't
    /// take its window's form style, so its form falls back to columns with
    /// the labels against the edge, and it's only as big as its content
    /// asks, which for a form is nothing, so on a Mac it grows or shrinks
    /// to fit its form, up to `height`, and scrolls past that.
    public func sheetLayout(width: CGFloat = 480, height: CGFloat = 640) -> some View {
        formStyle(.grouped)
            // VoiceOver stays in the sheet, not the screen behind it.
            .accessibilityAddTraits(.isModal)
            #if os(macOS)
                .modifier(FitsForm(width: width, maxHeight: height))
                .modifier(ClosesSheet())
            #endif
    }
}

extension EnvironmentValues {
    /// Closes the sheet the view is in, when the sheet says how.
    @Entry fileprivate var closeSheet: DismissAction?
}

#if os(macOS)
    /// Hands the sheet's own dismiss to what's inside it: in a Mac sheet's
    /// toolbar, the environment's dismiss doesn't close the sheet.
    private struct ClosesSheet: ViewModifier {
        @Environment(\.dismiss) private var dismiss

        func body(content: Content) -> some View {
            content.environment(\.closeSheet, dismiss)
        }
    }

    /// Sizes a sheet to the form in it: the height it has plus however much
    /// the form overflows or falls short, up to `maxHeight`. Each change in
    /// the form's size sizes it again, so it settles once the form has laid
    /// all its rows out. A sheet with nothing that scrolls keeps
    /// `maxHeight`.
    private struct FitsForm: ViewModifier {
        let width: CGFloat
        let maxHeight: CGFloat
        @State private var height: CGFloat?

        func body(content: Content) -> some View {
            content
                .onScrollGeometryChange(for: CGFloat.self) { geometry in
                    geometry.contentSize.height + geometry.contentInsets.top - geometry.containerSize.height
                } action: { _, overflow in
                    guard abs(overflow) >= 1 else { return }
                    height = min(max((height ?? maxHeight) + overflow, 160), maxHeight)
                }
                .frame(width: width, height: height ?? maxHeight)
        }
    }
#endif

/// Done, closing the sheet it's in.
public struct DismissButton: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(\.closeSheet) private var closeSheet

    public init() {}

    public var body: some View {
        Button("Done") { (closeSheet ?? dismiss)() }
    }
}

extension Binding {
    /// The value, or `fallback` once it's gone, with writes dropped then,
    /// while SwiftUI catches up. SwiftUI's own `Binding(_:)` over an
    /// optional force-unwraps it, which traps if the view reads it once more
    /// after it's set to nil, as a sheet's content does while it closes.
    public func or<Wrapped>(_ fallback: Wrapped) -> Binding<Wrapped> where Value == Wrapped? {
        Binding<Wrapped>(
            get: { wrappedValue ?? fallback },
            set: { if wrappedValue != nil { wrappedValue = $0 } })
    }
}

extension Binding where Value == String? {
    /// The text, with empty text as `nil`: for a field left empty to mean
    /// "unchanged".
    public var orEmpty: Binding<String> {
        Binding<String>(
            get: { wrappedValue ?? "" },
            set: { wrappedValue = $0.isEmpty ? nil : $0 })
    }
}

extension View {
    /// The chat's title, without the Markdown marks a first message may
    /// have, with a line in gray under it on systems that show one.
    public func chatTitle(_ title: String, subtitle: String) -> some View {
        modifier(ChatTitle(title: Markdown.plainLine(title), subtitle: subtitle))
    }

    /// The chat's title, without the Markdown marks a first message may
    /// have.
    public func chatTitle(_ title: String) -> some View {
        modifier(ChatTitle(title: Markdown.plainLine(title), subtitle: nil))
    }
}

private struct ChatTitle: ViewModifier {
    let title: String
    let subtitle: String?

    func body(content: Content) -> some View {
        #if os(iOS)
            if #available(iOS 26, *), let subtitle {
                content.navigationTitle(title).navigationSubtitle(subtitle)
                    .navigationBarTitleDisplayMode(.inline)
            } else {
                content.navigationTitle(title).navigationBarTitleDisplayMode(.inline)
            }
        #else
            if let subtitle {
                content.navigationTitle(title).navigationSubtitle(subtitle)
            } else {
                content.navigationTitle(title)
            }
        #endif
    }
}
