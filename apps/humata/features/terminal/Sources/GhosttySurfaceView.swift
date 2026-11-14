// Adapted from Ghostty's macOS app, MIT, Copyright (c) 2024 Mitchell
// Hashimoto, Ghostty contributors.

import AppKit
import Carbon
import GhosttyKit

/// One terminal drawn by Ghostty: a surface running a command on a pty of
/// its own, given this view's keys, mouse, size and focus.
final class GhosttySurfaceView: NSView {
    private(set) var surface: ghostty_surface_t?
    /// The command's line as C text. Ghostty keeps the pointer, not a copy,
    /// and reads it when it starts the command, so it lives as long as the
    /// surface does.
    private let line: CText
    /// Called once when the command ended, with its exit code if Ghostty
    /// told it.
    var exited: ((Int32?) -> Void)?
    /// The find bar's state, told what Ghostty's search finds.
    weak var search: TerminalSearch?
    private var reported = false
    private var focused = false
    private var cursor = NSCursor.iBeam
    private var occlusion: (any NSObjectProtocol)?

    /// Text an input method is composing, shown but not yet typed.
    private var markedText = NSMutableAttributedString()
    /// What `insertText` gave during the key press being handled; nil
    /// outside one.
    private var keyTextAccumulator: [String]?
    /// The screen's text as last read for accessibility, and when.
    private var screen: (text: String, read: TimeInterval)?
    /// The clipboard reads asked about and not answered yet.
    private var clipboardAlerts: [NSAlert] = []

    init(runtime: GhosttyRuntime, command: TerminalCommand) {
        line = CText(command.line)
        super.init(frame: NSRect(x: 0, y: 0, width: 640, height: 400))
        var config = ghostty_surface_config_new()
        config.platform_tag = GHOSTTY_PLATFORM_MACOS
        config.platform = ghostty_platform_u(
            macos: ghostty_platform_macos_s(nsview: Unmanaged.passUnretained(self).toOpaque()))
        config.userdata = Unmanaged.passUnretained(self).toOpaque()
        config.scale_factor = Double(NSScreen.main?.backingScaleFactor ?? 2)

        // C copies of the environment, freed once Ghostty has taken its own.
        let copies = command.environment.sorted { $0.key < $1.key }.map { (strdup($0.key), strdup($0.value)) }
        defer {
            for (key, value) in copies {
                free(key)
                free(value)
            }
        }
        var variables = copies.map { ghostty_env_var_s(key: $0.0, value: $0.1) }
        surface = variables.withUnsafeMutableBufferPointer { variables in
            config.command = UnsafePointer(line.pointer)
            config.env_vars = variables.baseAddress
            config.env_var_count = variables.count
            return ghostty_surface_new(runtime.app, &config)
        }
        sizeChanged()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not used")
    }

    deinit {
        // `close()` frees it while the view is still whole; this is for a
        // view dropped without it.
        guard let surface else { return }
        let freed = Freed(surface: surface, line: line)
        if Thread.isMainThread {
            ghostty_surface_free(freed.surface)
        } else {
            // The line goes with the closure, after the surface.
            DispatchQueue.main.async { ghostty_surface_free(freed.surface) }
        }
    }

    /// A surface on its way to being freed on the main thread, and the line
    /// it may still read until then.
    private struct Freed: @unchecked Sendable {
        let surface: ghostty_surface_t
        let line: CText
    }

    /// A C copy of a string, freed with the last reference to it.
    private final class CText {
        let pointer: UnsafeMutablePointer<CChar>?

        init(_ text: String) { pointer = strdup(text) }

        deinit { free(pointer) }
    }

    /// Ends the surface and its command; the view draws nothing after.
    func close() {
        if let occlusion { NotificationCenter.default.removeObserver(occlusion) }
        occlusion = nil
        exited = nil
        guard let surface else { return }
        self.surface = nil
        ghostty_surface_free(surface)
        // Their requests went with the surface: nothing is left to answer.
        let alerts = clipboardAlerts
        clipboardAlerts = []
        for alert in alerts { alert.window.sheetParent?.endSheet(alert.window) }
    }

    // MARK: - Clipboard

    /// A clipboard read Ghostty wants confirmed, copied out of its request.
    struct ClipboardRead {
        struct Content {
            var mime: String
            var data: Data
        }

        /// What the read gets, if it's allowed.
        var contents: [Content]
        /// The kinds of content on the clipboard, when a program asked for those.
        var available: [String]
        /// Who asks, as Ghostty names them.
        var program: String?
        /// A paste Ghostty distrusts, rather than a program reading.
        var paste: Bool
    }

    /// Ghostty's request, answered once from the sheet.
    private struct ClipboardRequest: @unchecked Sendable {
        let state: UnsafeMutableRawPointer?
    }

    /// Asks in a sheet on the window whether `read` may go on, and answers
    /// Ghostty with the choice; denied when there's no window to ask in.
    func confirmClipboardRead(_ read: ClipboardRead, state: UnsafeMutableRawPointer?) {
        guard let surface else { return }
        guard let window else {
            ghostty_surface_deny_clipboard_request(surface, state)
            return
        }
        let alert = NSAlert()
        alert.alertStyle = .warning
        if read.paste {
            let text =
                read.contents.first { $0.mime == "text/plain" }.map { String(decoding: $0.data, as: UTF8.self) } ?? ""
            let characters = text.count
            let lines = text.split(omittingEmptySubsequences: false, whereSeparator: \.isNewline).count
            let hidden = max(0, characters - Self.pasteShown)
            alert.messageText = String(localized: "Paste text that could run commands?")
            var warning = String(
                localized: """
                    What you’re pasting has line breaks or control characters, so the shell may run it as soon \
                    as it’s typed. Paste only if you’ve read all of it below and trust it. Control characters \
                    show as symbols such as ␛. Lines: \(lines). Characters: \(characters).
                    """)
            if hidden > 0 {
                warning += " "
                warning += String(
                    localized: """
                        Only the first \(Self.pasteShown) characters are shown: \(hidden) more aren’t, and \
                        would be pasted unread.
                        """)
            }
            alert.informativeText = warning
            alert.accessoryView = Self.pastePreview(String(text.prefix(Self.pasteShown)))
            alert.addButton(withTitle: String(localized: "Don’t Paste"))
            alert.addButton(withTitle: String(localized: "Paste"))
        } else {
            let who = read.program.map { "“\($0)”" } ?? String(localized: "A program in this terminal")
            alert.messageText = String(localized: "Let the terminal read the clipboard?")
            alert.informativeText = String(
                localized: """
                    \(who) asks for what you last copied, which could be a password or other private text. \
                    Allow it only if you started that program and trust it; Deny leaves the clipboard unread.
                    """)
            alert.addButton(withTitle: String(localized: "Deny"))
            alert.addButton(withTitle: String(localized: "Allow"))
        }
        let request = ClipboardRequest(state: state)
        clipboardAlerts.append(alert)
        alert.beginSheetModal(for: window) { [weak self] response in
            MainActor.assumeIsolated {
                guard let self, self.clipboardAlerts.contains(where: { $0 === alert }) else { return }
                self.clipboardAlerts.removeAll { $0 === alert }
                self.answerClipboardRead(read, allowed: response == .alertSecondButtonReturn, request: request)
            }
        }
    }

    /// How much of a paste its confirmation shows, in characters.
    private static let pasteShown = 200_000

    /// `text` to read before pasting it: all of it, scrolling, with each
    /// control character but line breaks and tabs shown as its symbol.
    private static func pastePreview(_ text: String) -> NSView {
        let scroll = NSTextView.scrollableTextView()
        scroll.frame = NSRect(x: 0, y: 0, width: 460, height: 180)
        scroll.borderType = .bezelBorder
        scroll.hasVerticalScroller = true
        scroll.hasHorizontalScroller = false
        if let view = scroll.documentView as? NSTextView {
            view.isEditable = false
            view.isSelectable = true
            view.isRichText = false
            view.font = .monospacedSystemFont(ofSize: NSFont.smallSystemFontSize, weight: .regular)
            view.textContainerInset = NSSize(width: 4, height: 4)
            view.string = visible(text)
            view.setAccessibilityLabel(String(localized: "The text to paste"))
        }
        return scroll
    }

    /// `text` with what a terminal would act on unseen made visible: each
    /// control character but a line break or tab becomes its picture, as ␛
    /// for escape.
    private static func visible(_ text: String) -> String {
        var shown = String.UnicodeScalarView()
        for scalar in text.unicodeScalars {
            let picture: UInt32? =
                switch scalar.value {
                case 0x0A, 0x09: nil
                case 0..<0x20: 0x2400 + scalar.value
                case 0x7F: 0x2421
                default: nil
                }
            shown.append(picture.flatMap(Unicode.Scalar.init) ?? scalar)
        }
        return String(shown)
    }

    /// Completes Ghostty's request with what it would read, or denies it.
    private func answerClipboardRead(_ read: ClipboardRead, allowed: Bool, request: ClipboardRequest) {
        guard let surface else { return }
        guard allowed else {
            ghostty_surface_deny_clipboard_request(surface, request.state)
            return
        }
        // C copies that last through the call, which copies them in turn.
        let mimes = read.contents.map { strdup($0.mime) }
        let datas = read.contents.map { content -> UnsafeMutablePointer<CChar> in
            let buffer = UnsafeMutablePointer<CChar>.allocate(capacity: max(content.data.count, 1))
            content.data.withUnsafeBytes { bytes in
                guard let base = bytes.baseAddress else { return }
                buffer.update(from: base.assumingMemoryBound(to: CChar.self), count: bytes.count)
            }
            return buffer
        }
        let available = read.available.map { strdup($0) }
        defer {
            for mime in mimes {
                free(mime)
            }
            for data in datas {
                data.deallocate()
            }
            for type in available {
                free(type)
            }
        }
        let contents = read.contents.indices.map { index in
            ghostty_clipboard_content_s(
                mime: UnsafePointer(mimes[index]), data: UnsafePointer(datas[index]),
                len: read.contents[index].data.count)
        }
        let names: [UnsafePointer<CChar>?] = available.map { $0.map { UnsafePointer($0) } }
        contents.withUnsafeBufferPointer { contents in
            names.withUnsafeBufferPointer { names in
                var complete = ghostty_clipboard_complete_s(
                    contents: contents.baseAddress, contents_len: contents.count,
                    available: names.baseAddress, available_len: names.count,
                    confirmed: true, remember: false)
                ghostty_surface_complete_clipboard_request(surface, &complete, request.state)
            }
        }
    }

    /// The command ended, or Ghostty asked to close the surface after it.
    func processExited(code: Int32?) {
        guard !reported else { return }
        reported = true
        exited?(code)
    }

    /// Does one of Ghostty's binding actions, such as `search:needle`, as
    /// if a key bound to it was pressed; false when Ghostty didn't.
    @discardableResult
    func perform(_ action: String) -> Bool {
        guard let surface else { return false }
        return action.withCString { ghostty_surface_binding_action(surface, $0, UInt(action.utf8.count)) }
    }

    // MARK: - Size, scale and visibility

    override var acceptsFirstResponder: Bool { true }

    override func setFrameSize(_ newSize: NSSize) {
        super.setFrameSize(newSize)
        sizeChanged()
    }

    private func sizeChanged() {
        guard let surface else { return }
        let pixels = convertToBacking(bounds.size)
        guard pixels.width >= 1, pixels.height >= 1 else { return }
        ghostty_surface_set_size(surface, UInt32(pixels.width), UInt32(pixels.height))
    }

    override func viewDidChangeBackingProperties() {
        super.viewDidChangeBackingProperties()
        if let window {
            // Ghostty scales what it draws itself: the layer mustn't again.
            CATransaction.begin()
            CATransaction.setDisableActions(true)
            layer?.contentsScale = window.backingScaleFactor
            CATransaction.commit()
        }
        guard let surface, bounds.width > 0, bounds.height > 0 else { return }
        let pixels = convertToBacking(bounds.size)
        ghostty_surface_set_content_scale(surface, pixels.width / bounds.width, pixels.height / bounds.height)
        sizeChanged()
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if let occlusion { NotificationCenter.default.removeObserver(occlusion) }
        occlusion = nil
        guard let window else { return }
        occlusion = NotificationCenter.default.addObserver(
            forName: NSWindow.didChangeOcclusionStateNotification, object: window, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.visibilityChanged() }
        }
        visibilityChanged()
        viewDidChangeBackingProperties()
    }

    private func visibilityChanged() {
        guard let surface, let window else { return }
        ghostty_surface_set_occlusion(surface, window.occlusionState.contains(.visible))
        if let number = window.screen?.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber {
            ghostty_surface_set_display_id(surface, number.uint32Value)
        }
    }

    // MARK: - Focus

    override func becomeFirstResponder() -> Bool {
        let result = super.becomeFirstResponder()
        if result { focusChanged(true) }
        return result
    }

    override func resignFirstResponder() -> Bool {
        let result = super.resignFirstResponder()
        if result { focusChanged(false) }
        return result
    }

    private func focusChanged(_ focused: Bool) {
        guard self.focused != focused else { return }
        self.focused = focused
        if let surface { ghostty_surface_set_focus(surface, focused) }
    }

    // MARK: - Mouse

    func setCursor(_ shape: ghostty_action_mouse_shape_e) {
        switch shape {
        case GHOSTTY_MOUSE_SHAPE_DEFAULT: cursor = .arrow
        case GHOSTTY_MOUSE_SHAPE_TEXT: cursor = .iBeam
        case GHOSTTY_MOUSE_SHAPE_VERTICAL_TEXT: cursor = .iBeamCursorForVerticalLayout
        case GHOSTTY_MOUSE_SHAPE_POINTER: cursor = .pointingHand
        case GHOSTTY_MOUSE_SHAPE_CROSSHAIR: cursor = .crosshair
        case GHOSTTY_MOUSE_SHAPE_GRAB: cursor = .openHand
        case GHOSTTY_MOUSE_SHAPE_GRABBING: cursor = .closedHand
        case GHOSTTY_MOUSE_SHAPE_NOT_ALLOWED: cursor = .operationNotAllowed
        case GHOSTTY_MOUSE_SHAPE_CONTEXT_MENU: cursor = .contextualMenu
        case GHOSTTY_MOUSE_SHAPE_EW_RESIZE, GHOSTTY_MOUSE_SHAPE_COL_RESIZE: cursor = .resizeLeftRight
        case GHOSTTY_MOUSE_SHAPE_NS_RESIZE, GHOSTTY_MOUSE_SHAPE_ROW_RESIZE: cursor = .resizeUpDown
        default: return
        }
        window?.invalidateCursorRects(for: self)
    }

    override func resetCursorRects() {
        addCursorRect(bounds, cursor: cursor)
    }

    override func updateTrackingAreas() {
        trackingAreas.forEach(removeTrackingArea)
        // Always active, so programs that follow the mouse still do while
        // another view has the keys.
        addTrackingArea(
            NSTrackingArea(
                rect: bounds, options: [.mouseEnteredAndExited, .mouseMoved, .inVisibleRect, .activeAlways],
                owner: self, userInfo: nil))
    }

    private func button(_ state: ghostty_input_mouse_state_e, _ button: ghostty_input_mouse_button_e, _ event: NSEvent)
        -> Bool
    {
        guard let surface else { return false }
        return ghostty_surface_mouse_button(surface, state, button, Self.mods(event.modifierFlags))
    }

    private static func otherButton(_ event: NSEvent) -> ghostty_input_mouse_button_e {
        switch event.buttonNumber {
        case 2: GHOSTTY_MOUSE_MIDDLE
        case 3: GHOSTTY_MOUSE_FOUR
        case 4: GHOSTTY_MOUSE_FIVE
        default: GHOSTTY_MOUSE_UNKNOWN
        }
    }

    override func mouseDown(with event: NSEvent) {
        window?.makeFirstResponder(self)
        _ = button(GHOSTTY_MOUSE_PRESS, GHOSTTY_MOUSE_LEFT, event)
    }

    override func mouseUp(with event: NSEvent) {
        _ = button(GHOSTTY_MOUSE_RELEASE, GHOSTTY_MOUSE_LEFT, event)
    }

    override func rightMouseDown(with event: NSEvent) {
        if !button(GHOSTTY_MOUSE_PRESS, GHOSTTY_MOUSE_RIGHT, event) { super.rightMouseDown(with: event) }
    }

    override func rightMouseUp(with event: NSEvent) {
        if !button(GHOSTTY_MOUSE_RELEASE, GHOSTTY_MOUSE_RIGHT, event) { super.rightMouseUp(with: event) }
    }

    override func otherMouseDown(with event: NSEvent) {
        _ = button(GHOSTTY_MOUSE_PRESS, Self.otherButton(event), event)
    }

    override func otherMouseUp(with event: NSEvent) {
        _ = button(GHOSTTY_MOUSE_RELEASE, Self.otherButton(event), event)
    }

    override func mouseMoved(with event: NSEvent) {
        guard let surface else { return }
        // Ghostty counts from the top left; this view from the bottom left.
        let point = convert(event.locationInWindow, from: nil)
        ghostty_surface_mouse_pos(surface, point.x, bounds.height - point.y, Self.mods(event.modifierFlags))
    }

    override func mouseEntered(with event: NSEvent) { mouseMoved(with: event) }
    override func mouseDragged(with event: NSEvent) { mouseMoved(with: event) }
    override func rightMouseDragged(with event: NSEvent) { mouseMoved(with: event) }
    override func otherMouseDragged(with event: NSEvent) { mouseMoved(with: event) }

    override func mouseExited(with event: NSEvent) {
        // A drag goes on reporting from outside; otherwise the pointer left.
        guard let surface, NSEvent.pressedMouseButtons == 0 else { return }
        ghostty_surface_mouse_pos(surface, -1, -1, Self.mods(event.modifierFlags))
    }

    override func scrollWheel(with event: NSEvent) {
        guard let surface else { return }
        var (x, y) = (event.scrollingDeltaX, event.scrollingDeltaY)
        let precise = event.hasPreciseScrollingDeltas
        // A trackpad's deltas feel slow as they are; Ghostty's app doubles
        // them too.
        if precise { (x, y) = (x * 2, y * 2) }
        let momentum: Int32 =
            switch event.momentumPhase {
            case .began: 1
            case .stationary: 2
            case .changed: 3
            case .ended: 4
            case .cancelled: 5
            case .mayBegin: 6
            default: 0
            }
        // The scroll mods' bits: precision, then the momentum phase.
        ghostty_surface_mouse_scroll(surface, x, y, (precise ? 1 : 0) | momentum << 1)
    }

    // MARK: - Keys

    private static func mods(_ flags: NSEvent.ModifierFlags) -> ghostty_input_mods_e {
        var mods = GHOSTTY_MODS_NONE.rawValue
        if flags.contains(.shift) { mods |= GHOSTTY_MODS_SHIFT.rawValue }
        if flags.contains(.control) { mods |= GHOSTTY_MODS_CTRL.rawValue }
        if flags.contains(.option) { mods |= GHOSTTY_MODS_ALT.rawValue }
        if flags.contains(.command) { mods |= GHOSTTY_MODS_SUPER.rawValue }
        if flags.contains(.capsLock) { mods |= GHOSTTY_MODS_CAPS.rawValue }
        // The device's bits for the right-hand keys.
        if flags.rawValue & RightKey.shift != 0 { mods |= GHOSTTY_MODS_SHIFT_RIGHT.rawValue }
        if flags.rawValue & RightKey.control != 0 { mods |= GHOSTTY_MODS_CTRL_RIGHT.rawValue }
        if flags.rawValue & RightKey.option != 0 { mods |= GHOSTTY_MODS_ALT_RIGHT.rawValue }
        if flags.rawValue & RightKey.command != 0 { mods |= GHOSTTY_MODS_SUPER_RIGHT.rawValue }
        return ghostty_input_mods_e(rawValue: mods)
    }

    /// IOKit's `NX_DEVICER…KEYMASK` bits in an event's modifier flags.
    private enum RightKey {
        static let control: UInt = 0x2000
        static let shift: UInt = 0x4
        static let command: UInt = 0x10
        static let option: UInt = 0x40
    }

    private static func flags(_ mods: ghostty_input_mods_e) -> NSEvent.ModifierFlags {
        var flags = NSEvent.ModifierFlags()
        if mods.rawValue & GHOSTTY_MODS_SHIFT.rawValue != 0 { flags.insert(.shift) }
        if mods.rawValue & GHOSTTY_MODS_CTRL.rawValue != 0 { flags.insert(.control) }
        if mods.rawValue & GHOSTTY_MODS_ALT.rawValue != 0 { flags.insert(.option) }
        if mods.rawValue & GHOSTTY_MODS_SUPER.rawValue != 0 { flags.insert(.command) }
        return flags
    }

    /// `event` as Ghostty takes keys, without its text.
    private static func key(
        _ event: NSEvent, _ action: ghostty_input_action_e, translationMods: NSEvent.ModifierFlags? = nil
    ) -> ghostty_input_key_s {
        var key = ghostty_input_key_s()
        key.action = action
        key.keycode = UInt32(event.keyCode)
        key.text = nil
        key.composing = false
        key.mods = mods(event.modifierFlags)
        // Control and Command never make text, so they aren't used up by it.
        key.consumed_mods = mods((translationMods ?? event.modifierFlags).subtracting([.control, .command]))
        key.unshifted_codepoint = 0
        if event.type == .keyDown || event.type == .keyUp,
            let scalar = event.characters(byApplyingModifiers: [])?.unicodeScalars.first
        {
            key.unshifted_codepoint = scalar.value
        }
        return key
    }

    /// The text a key event types, for Ghostty: not a control character,
    /// which Ghostty encodes itself, nor a function key's private-use one.
    private static func text(_ event: NSEvent) -> String? {
        guard let characters = event.characters else { return nil }
        if characters.count == 1, let scalar = characters.unicodeScalars.first {
            if scalar.value < 0x20 {
                return event.characters(byApplyingModifiers: event.modifierFlags.subtracting(.control))
            }
            if scalar.value >= 0xF700, scalar.value <= 0xF8FF { return nil }
        }
        return characters
    }

    private static func isControl(_ text: String?) -> Bool {
        guard let scalar = text?.unicodeScalars.first else { return false }
        return scalar.value < 0x20 || scalar.value == 0x7F
    }

    /// The keyboard layout or input method in use.
    private static var inputSource: String? {
        guard let source = TISCopyCurrentKeyboardInputSource()?.takeRetainedValue(),
            let pointer = TISGetInputSourceProperty(source, kTISPropertyInputSourceID)
        else { return nil }
        return Unmanaged<CFString>.fromOpaque(pointer).takeUnretainedValue() as String
    }

    @discardableResult
    private func send(
        _ action: ghostty_input_action_e, _ event: NSEvent, translation: NSEvent? = nil, text: String? = nil,
        composing: Bool = false
    ) -> Bool {
        guard let surface else { return false }
        var key = Self.key(event, action, translationMods: translation?.modifierFlags)
        key.composing = composing
        guard let text, !text.isEmpty, !Self.isControl(text) else { return ghostty_surface_key(surface, key) }
        return text.withCString {
            key.text = $0
            return ghostty_surface_key(surface, key)
        }
    }

    /// Text an input method committed, typed as it is.
    private func sendCommitted(_ text: String) {
        guard let surface, !text.isEmpty else { return }
        var key = ghostty_input_key_s()
        key.action = GHOSTTY_ACTION_PRESS
        key.mods = GHOSTTY_MODS_NONE
        key.consumed_mods = GHOSTTY_MODS_NONE
        text.withCString {
            key.text = $0
            _ = ghostty_surface_key(surface, key)
        }
    }

    override func keyDown(with event: NSEvent) {
        guard let surface else { return }
        // Option as Alt, as the config says: the text is then worked out
        // without Option.
        let translated = Self.flags(ghostty_surface_key_translation_mods(surface, Self.mods(event.modifierFlags)))
        var translationMods = event.modifierFlags
        for flag in [NSEvent.ModifierFlags.shift, .control, .option, .command] {
            if translated.contains(flag) { translationMods.insert(flag) } else { translationMods.remove(flag) }
        }
        // The same event when nothing changed: input methods such as
        // Korean's need the very one AppKit sent.
        var translation = event
        if translationMods != event.modifierFlags {
            translation =
                NSEvent.keyEvent(
                    with: event.type, location: event.locationInWindow, modifierFlags: translationMods,
                    timestamp: event.timestamp, windowNumber: event.windowNumber, context: nil,
                    characters: event.characters(byApplyingModifiers: translationMods) ?? "",
                    charactersIgnoringModifiers: event.charactersIgnoringModifiers ?? "",
                    isARepeat: event.isARepeat, keyCode: event.keyCode) ?? event
        }
        let action = event.isARepeat ? GHOSTTY_ACTION_REPEAT : GHOSTTY_ACTION_PRESS

        keyTextAccumulator = []
        defer { keyTextAccumulator = nil }
        let markedBefore = markedText.length > 0
        let sourceBefore = markedBefore ? nil : Self.inputSource
        interpretKeyEvents([translation])
        // The key switched the input method: it isn't for the terminal.
        if !markedBefore, sourceBefore != Self.inputSource { return }
        syncPreedit(clearIfNeeded: markedBefore)

        // Composing still, or this key just ended it, such as Backspace
        // over the last composed letter: then it isn't typed.
        let composing = markedText.length > 0 || markedBefore
        let committed = (keyTextAccumulator ?? []).filter { !(composing && Self.isControl($0)) }
        if markedBefore, !(keyTextAccumulator ?? []).isEmpty {
            committed.forEach(sendCommitted)
        } else if !(keyTextAccumulator ?? []).isEmpty {
            for text in committed { send(action, event, translation: translation, text: text) }
        } else if !(composing && Self.isControl(event.characters)) {
            send(action, event, translation: translation, text: Self.text(translation), composing: composing)
        }
    }

    override func keyUp(with event: NSEvent) {
        send(GHOSTTY_ACTION_RELEASE, event)
    }

    override func flagsChanged(with event: NSEvent) {
        let (mod, right): (UInt32, UInt?) =
            switch event.keyCode {
            case 0x39: (GHOSTTY_MODS_CAPS.rawValue, nil)
            case 0x38: (GHOSTTY_MODS_SHIFT.rawValue, nil)
            case 0x3C: (GHOSTTY_MODS_SHIFT.rawValue, RightKey.shift)
            case 0x3B: (GHOSTTY_MODS_CTRL.rawValue, nil)
            case 0x3E: (GHOSTTY_MODS_CTRL.rawValue, RightKey.control)
            case 0x3A: (GHOSTTY_MODS_ALT.rawValue, nil)
            case 0x3D: (GHOSTTY_MODS_ALT.rawValue, RightKey.option)
            case 0x37: (GHOSTTY_MODS_SUPER.rawValue, nil)
            case 0x36: (GHOSTTY_MODS_SUPER.rawValue, RightKey.command)
            default: (0, nil)
            }
        guard mod != 0, !hasMarkedText() else { return }
        // Pressed if its modifier is now held, and for a right-hand key if
        // it's that side that is.
        var pressed = Self.mods(event.modifierFlags).rawValue & mod != 0
        if pressed, let right { pressed = event.modifierFlags.rawValue & right != 0 }
        send(pressed ? GHOSTTY_ACTION_PRESS : GHOSTTY_ACTION_RELEASE, event)
    }

    /// Takes a ⌘ key only when the terminal's config binds it, which is
    /// copy, paste, select all and find; every other shortcut stays
    /// Humata's.
    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        guard event.type == .keyDown, focused, let surface else { return false }
        var key = Self.key(event, GHOSTTY_ACTION_PRESS)
        var flags = ghostty_binding_flags_e(rawValue: 0)
        let bound = (event.characters ?? "").withCString {
            key.text = $0
            return ghostty_surface_key_is_binding(surface, key, &flags)
        }
        guard bound else { return false }
        keyDown(with: event)
        return true
    }

    /// Keys AppKit names as commands, such as arrows, reach the terminal
    /// from `keyDown`; nothing more to do, and no beep.
    override func doCommand(by selector: Selector) {}

    /// Shows what's being composed in the terminal, or clears it.
    private func syncPreedit(clearIfNeeded: Bool = true) {
        guard let surface else { return }
        if markedText.length > 0 {
            let text = markedText.string
            text.withCString { ghostty_surface_preedit(surface, $0, UInt(text.utf8.count)) }
        } else if clearIfNeeded {
            ghostty_surface_preedit(surface, nil, 0)
        }
    }

    // MARK: - Accessibility

    override func isAccessibilityElement() -> Bool { true }
    override func accessibilityRole() -> NSAccessibility.Role? { .textArea }
    override func accessibilityLabel() -> String? { "Terminal" }

    /// What the terminal shows now, for VoiceOver and the flows' trees.
    override func accessibilityValue() -> Any? { screenText() }

    override func accessibilityNumberOfCharacters() -> Int {
        (screenText() as NSString).length
    }

    /// All of it: the value is only what's on screen.
    override func accessibilityVisibleCharacterRange() -> NSRange {
        NSRange(location: 0, length: (screenText() as NSString).length)
    }

    override func accessibilityString(for range: NSRange) -> String? {
        let text = screenText() as NSString
        guard range.location >= 0, range.length >= 0, range.location + range.length <= text.length else { return nil }
        return text.substring(with: range)
    }

    /// The line a character is on, counted from 0, so VoiceOver can read
    /// line by line.
    override func accessibilityLine(for index: Int) -> Int {
        let text = screenText() as NSString
        let before = text.substring(to: min(max(index, 0), text.length))
        return before.components(separatedBy: .newlines).count - 1
    }

    override func accessibilityRange(forLine line: Int) -> NSRange {
        let text = screenText() as NSString
        var (start, number) = (0, 0)
        while start <= text.length {
            let range = text.lineRange(for: NSRange(location: start, length: 0))
            if number == line { return range }
            guard NSMaxRange(range) > start else { break }
            start = NSMaxRange(range)
            number += 1
        }
        return NSRange(location: NSNotFound, length: 0)
    }

    override func accessibilitySelectedText() -> String? {
        guard let surface, ghostty_surface_has_selection(surface) else { return nil }
        var text = ghostty_text_s()
        guard ghostty_surface_read_selection(surface, &text) else { return nil }
        defer { ghostty_surface_free_text(surface, &text) }
        return Self.string(text)
    }

    /// What's on screen, read at most twice a second: VoiceOver asks many
    /// times over for one reading.
    private func screenText() -> String {
        let now = ProcessInfo.processInfo.systemUptime
        if let screen, now - screen.read < 0.5 { return screen.text }
        let text = readScreen()
        screen = (text, now)
        return text
    }

    private func readScreen() -> String {
        guard let surface else { return "" }
        let corner = { (coord: ghostty_point_coord_e) in
            ghostty_point_s(tag: GHOSTTY_POINT_VIEWPORT, coord: coord, x: 0, y: 0)
        }
        let all = ghostty_selection_s(
            top_left: corner(GHOSTTY_POINT_COORD_TOP_LEFT), bottom_right: corner(GHOSTTY_POINT_COORD_BOTTOM_RIGHT),
            rectangle: false)
        var text = ghostty_text_s()
        guard ghostty_surface_read_text(surface, all, &text) else { return "" }
        defer { ghostty_surface_free_text(surface, &text) }
        return Self.string(text)
    }

    private static func string(_ text: ghostty_text_s) -> String {
        guard let start = text.text else { return "" }
        return String(decoding: UnsafeRawBufferPointer(start: start, count: Int(text.text_len)), as: UTF8.self)
    }
}

// MARK: - Input methods

extension GhosttySurfaceView: NSTextInputClient {
    func hasMarkedText() -> Bool { markedText.length > 0 }

    func markedRange() -> NSRange {
        guard markedText.length > 0 else { return NSRange(location: NSNotFound, length: 0) }
        return NSRange(location: 0, length: markedText.length)
    }

    func selectedRange() -> NSRange { NSRange(location: 0, length: 0) }

    func setMarkedText(_ string: Any, selectedRange: NSRange, replacementRange: NSRange) {
        switch string {
        case let text as NSAttributedString: markedText = NSMutableAttributedString(attributedString: text)
        case let text as String: markedText = NSMutableAttributedString(string: text)
        default: return
        }
        // Outside a key press, as when the layout changes while composing,
        // nothing else will show it.
        if keyTextAccumulator == nil { syncPreedit() }
    }

    func unmarkText() {
        guard markedText.length > 0 else { return }
        markedText.mutableString.setString("")
        syncPreedit()
    }

    func validAttributesForMarkedText() -> [NSAttributedString.Key] { [] }

    func attributedSubstring(forProposedRange range: NSRange, actualRange: NSRangePointer?) -> NSAttributedString? {
        nil
    }

    func characterIndex(for point: NSPoint) -> Int { 0 }

    /// Where the cursor is on screen, for the input method's window.
    func firstRect(forCharacterRange range: NSRange, actualRange: NSRangePointer?) -> NSRect {
        guard let surface, let window else { return .zero }
        var (x, y, width, height) = (0.0, 0.0, 0.0, 0.0)
        ghostty_surface_ime_point(surface, &x, &y, &width, &height)
        let rect = NSRect(x: x, y: bounds.height - y, width: width, height: height)
        return window.convertToScreen(convert(rect, to: nil))
    }

    func insertText(_ string: Any, replacementRange: NSRange) {
        let text: String
        switch string {
        case let value as NSAttributedString: text = value.string
        case let value as String: text = value
        default: return
        }
        // Committing ends the composing.
        unmarkText()
        if keyTextAccumulator != nil {
            keyTextAccumulator?.append(text)
        } else {
            // Not from a key press: the emoji picker, dictation.
            sendCommitted(text)
        }
    }
}
