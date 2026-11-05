import SwiftUI

extension View {
    /// Lays out a sheet the same way on iPhone and Mac. A Mac sheet doesn't
    /// take its window's form style, so its form falls back to columns with
    /// the labels against the edge, and it's only as big as its content
    /// asks, so it gets a size of its own.
    package func sheetLayout(width: CGFloat = 480, height: CGFloat = 420) -> some View {
        formStyle(.grouped)
            #if os(macOS)
                .frame(minWidth: width, idealWidth: width, minHeight: height, idealHeight: height)
            #endif
    }
}
