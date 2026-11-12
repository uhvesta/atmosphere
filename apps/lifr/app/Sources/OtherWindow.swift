#if os(macOS)
    import ComposableArchitecture
    import LifrRoot
    import SwiftUI

    /// One of the Mac's other windows: a store of its own, so it goes where
    /// it likes while sharing the records, locked and unlocked with the
    /// first window, and reopened where it was when Lifr opens again.
    struct OtherWindow: View {
        let main: StoreOf<RootFeature>
        @Binding var place: WindowPlace?
        @State private var store: StoreOf<RootFeature>

        init(main: StoreOf<RootFeature>, place: Binding<WindowPlace?>, store: (Place) -> StoreOf<RootFeature>) {
            self.main = main
            _place = place
            _store = State(initialValue: store(place.wrappedValue?.place ?? Place(sidebar: .today)))
        }

        var body: some View {
            RootView(store: store)
                .onChange(of: main.isLocked, initial: true) { _, locked in store.send(.lockMirrored(locked)) }
                .onChange(of: store.isLocked) { _, locked in main.send(.lockMirrored(locked)) }
                .onChange(of: store.here) { _, here in place?.place = here }
                // New Tab opened it: it joins the window in front.
                .joinsTab()
        }
    }
#endif
