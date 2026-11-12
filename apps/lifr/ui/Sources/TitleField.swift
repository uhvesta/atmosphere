import SwiftUI

extension View {
    /// A title typed from the leading edge, as on iPhone. A Mac form would
    /// put the field's label on the left and push what's typed to the right;
    /// the placeholder still says what goes there.
    package func titleField() -> some View {
        #if os(macOS)
            labelsHidden().multilineTextAlignment(.leading)
        #else
            self
        #endif
    }
}
