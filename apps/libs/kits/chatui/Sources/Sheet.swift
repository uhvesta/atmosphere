import SwiftUI

extension View {
    /// Lays out a sheet the same way on iPhone and Mac. A Mac sheet doesn't
    /// take its window's form style, so its form falls back to columns with
    /// the labels against the edge, and it's only as big as its content
    /// asks, so it gets a size of its own.
    public func sheetLayout(width: CGFloat = 480, height: CGFloat = 420) -> some View {
        formStyle(.grouped)
            #if os(macOS)
                .frame(minWidth: width, idealWidth: width, minHeight: height, idealHeight: height)
            #endif
    }
}

/// Done, closing the sheet it's in.
public struct DismissButton: View {
    @Environment(\.dismiss) private var dismiss

    public init() {}

    public var body: some View {
        Button("Done") { dismiss() }
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
