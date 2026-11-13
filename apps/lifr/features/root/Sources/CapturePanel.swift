#if os(macOS)
    import AppKit
    import Carbon.HIToolbox
    import Combine
    import ComposableArchitecture
    import LifrQuickAdd
    import LifrSettings
    import SwiftUI

    /// Quick Add from any app: the keys picked in Settings open a small
    /// floating panel over whatever is in front, with no Lifr window
    /// needed. Return adds the task and closes it.
    @MainActor
    package final class CapturePanel: NSObject, NSWindowDelegate {
        private let store: StoreOf<RootFeature>
        private var panel: NSPanel?
        private var hotKey: EventHotKeyRef?
        private var handler: EventHandlerRef?
        private var observation: AnyCancellable?
        @Shared(.quickAddHotKey) private var keys

        package init(store: StoreOf<RootFeature>) {
            self.store = store
        }

        /// Listens for the keys, and again each time Settings changes them.
        package func start() {
            guard handler == nil else { return }
            var pressed = EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed))
            let context = Unmanaged.passUnretained(self).toOpaque()
            InstallEventHandler(
                GetApplicationEventTarget(),
                { _, _, context in
                    guard let context else { return OSStatus(eventNotHandledErr) }
                    let capture = Unmanaged<CapturePanel>.fromOpaque(context).takeUnretainedValue()
                    MainActor.assumeIsolated { capture.show() }
                    return noErr
                }, 1, &pressed, context, &handler)
            register(keys)
            observation = $keys.publisher.removeDuplicates().sink { [weak self] keys in
                MainActor.assumeIsolated { self?.register(keys) }
            }
        }

        private func register(_ keys: QuickAddHotKey) {
            if let hotKey {
                UnregisterEventHotKey(hotKey)
                self.hotKey = nil
            }
            guard let modifiers = keys.carbonModifiers else { return }
            // "LIFR", so the press is told apart from other apps' keys.
            let id = EventHotKeyID(signature: OSType(0x4C49_4652), id: 1)
            // Taken by the system or another app: nothing happens, and
            // Settings says to pick other keys.
            RegisterEventHotKey(UInt32(kVK_Space), modifiers, id, GetApplicationEventTarget(), 0, &hotKey)
        }

        /// While locked, Lifr comes forward to be unlocked instead, so the
        /// panel never shows Avenue names to whoever is at the Mac.
        private func show() {
            guard !store.isLocked else {
                NSApplication.shared.activate()
                return
            }
            store.send(.captureOpened)
            guard store.capture != nil else { return }
            let panel = panel ?? makePanel()
            panel.center()
            panel.makeKeyAndOrderFront(nil)
        }

        private func makePanel() -> NSPanel {
            let panel = KeyPanel(
                contentRect: NSRect(x: 0, y: 0, width: 480, height: 440),
                styleMask: [.titled, .closable, .fullSizeContentView, .nonactivatingPanel], backing: .buffered,
                defer: true)
            panel.level = .floating
            panel.isFloatingPanel = true
            panel.hidesOnDeactivate = false
            panel.isReleasedWhenClosed = false
            panel.titleVisibility = .hidden
            panel.titlebarAppearsTransparent = true
            panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
            panel.delegate = self
            panel.contentView = NSHostingView(
                rootView: CaptureView(store: store) { [weak panel] in panel?.orderOut(nil) })
            self.panel = panel
            return panel
        }

        package func windowWillClose(_ notification: Notification) {
            if store.capture != nil { store.send(.capture(.dismiss)) }
        }
    }

    /// A panel that takes typing without bringing Lifr's windows forward.
    private final class KeyPanel: NSPanel {
        override var canBecomeKey: Bool { true }
    }

    private struct CaptureView: View {
        let store: StoreOf<RootFeature>
        let close: () -> Void

        var body: some View {
            Group {
                if let capture = store.scope(state: \.capture, action: \.capture.presented) {
                    NavigationStack { QuickAddView(store: capture) }
                }
            }
            .frame(width: 480, height: 440)
            .onChange(of: store.capture == nil) { _, closed in
                if closed { close() }
            }
        }
    }

    extension QuickAddHotKey {
        fileprivate var carbonModifiers: UInt32? {
            switch self {
            case .off: nil
            case .controlSpace: UInt32(controlKey)
            case .optionSpace: UInt32(optionKey)
            case .controlOptionSpace: UInt32(controlKey | optionKey)
            case .shiftCommandSpace: UInt32(shiftKey | cmdKey)
            }
        }
    }
#endif
