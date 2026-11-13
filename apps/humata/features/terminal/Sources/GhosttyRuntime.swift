// Adapted from Ghostty's macOS app, MIT, Copyright (c) 2024 Mitchell
// Hashimoto, Ghostty contributors.

import AppKit
import GhosttyKit

/// Why Ghostty couldn't be started to draw terminals.
enum GhosttyRuntimeError: Error {
    /// `ghostty_init` answered `code`.
    case notInitialized(code: Int32)
    /// The config Ghostty reads couldn't be written to `path`.
    case configNotWritten(path: String, underlying: any Error)
    /// `ghostty_app_new` gave no app.
    case appNotCreated
}

extension GhosttyRuntimeError: LocalizedError {
    var errorDescription: String? {
        switch self {
        case .notInitialized(let code):
            "The terminal couldn't start (Ghostty answered \(code)). Quit Humata and open it again; if it "
                + "happens again, install Humata again."
        case .configNotWritten(let path, let underlying):
            "The terminal's settings couldn't be written to \(path): \(underlying.localizedDescription) "
                + "Check that your Caches folder can be written to and has room, then open the terminal again."
        case .appNotCreated:
            "The terminal couldn't start: Ghostty didn't accept its settings. Quit Humata and open it again; if "
                + "it happens again, install Humata again."
        }
    }
}

/// The one Ghostty app all terminals draw with, made when the first is
/// shown. It reads only the config written here from a `TerminalStyle`,
/// never Ghostty's own files.
@MainActor
final class GhosttyRuntime {
    /// The runtime, or why there is none; tried once.
    static let shared: Result<GhosttyRuntime, GhosttyRuntimeError> = make()

    let app: ghostty_app_t
    private var config: ghostty_config_t
    private var style: TerminalStyle
    private var observers: [any NSObjectProtocol] = []
    private var appearance: NSKeyValueObservation?

    private init(app: ghostty_app_t, config: ghostty_config_t, style: TerminalStyle) {
        self.app = app
        self.config = config
        self.style = style
    }

    private static func make() -> Result<GhosttyRuntime, GhosttyRuntimeError> {
        let code = ghostty_init(UInt(CommandLine.argc), CommandLine.unsafeArgv)
        guard code == 0 else { return .failure(.notInitialized(code: code)) }

        let style = TerminalStyle(defaults: .standard)
        let config: ghostty_config_t
        switch loadConfig(style) {
        case .success(let loaded): config = loaded
        case .failure(let error): return .failure(error)
        }

        var callbacks = ghostty_runtime_config_s(
            userdata: nil,
            supports_selection_clipboard: false,
            wakeup_cb: ghosttyWakeup,
            action_cb: ghosttyAction,
            read_clipboard_cb: ghosttyReadClipboard,
            confirm_read_clipboard_cb: ghosttyConfirmReadClipboard,
            write_clipboard_cb: ghosttyWriteClipboard,
            close_surface_cb: ghosttyCloseSurface)
        guard let app = ghostty_app_new(&callbacks, config) else {
            ghostty_config_free(config)
            return .failure(.appNotCreated)
        }
        let runtime = GhosttyRuntime(app: app, config: config, style: style)
        runtime.observe()
        return .success(runtime)
    }

    /// Whether the app draws dark now.
    private static var dark: Bool {
        NSApplication.shared.effectiveAppearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua
    }

    /// A config holding only what `style` says, read from the one file
    /// written here for it.
    private static func loadConfig(_ style: TerminalStyle) -> Result<ghostty_config_t, GhosttyRuntimeError> {
        let folder = URL.cachesDirectory.appending(path: Bundle.main.bundleIdentifier ?? "Humata")
        let file = folder.appending(path: "terminal.ghostty")
        do {
            try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
            try style.config(dark: dark).write(to: file, atomically: true, encoding: .utf8)
        } catch {
            return .failure(.configNotWritten(path: file.path, underlying: error))
        }
        guard let config = ghostty_config_new() else { return .failure(.appNotCreated) }
        file.path.withCString { ghostty_config_load_file(config, $0) }
        ghostty_config_finalize(config)
        return .success(config)
    }

    /// Lets Ghostty do the work it asked to be woken for.
    func tick() {
        ghostty_app_tick(app)
    }

    /// Gives every terminal the config of the style Settings keeps, for the
    /// appearance the app has now. A config that can't be written leaves them as they
    /// are.
    func updateConfig() {
        let style = TerminalStyle(defaults: .standard)
        guard case .success(let new) = Self.loadConfig(style) else { return }
        self.style = style
        ghostty_app_update_config(app, new)
        ghostty_app_set_color_scheme(app, Self.dark ? GHOSTTY_COLOR_SCHEME_DARK : GHOSTTY_COLOR_SCHEME_LIGHT)
        ghostty_config_free(config)
        config = new
    }

    /// Settings may have changed the style: every terminal follows it. The
    /// app's defaults change for much else too, which is left alone.
    private func defaultsChanged() {
        guard TerminalStyle(defaults: .standard) != style else { return }
        updateConfig()
    }

    private func observe() {
        let center = NotificationCenter.default
        let app = NSApplication.shared
        ghostty_app_set_focus(self.app, app.isActive)
        ghostty_app_set_color_scheme(self.app, Self.dark ? GHOSTTY_COLOR_SCHEME_DARK : GHOSTTY_COLOR_SCHEME_LIGHT)
        observers = [
            center.addObserver(forName: NSApplication.didBecomeActiveNotification, object: nil, queue: .main) { _ in
                MainActor.assumeIsolated { GhosttyRuntime.running { ghostty_app_set_focus($0.app, true) } }
            },
            center.addObserver(forName: NSApplication.didResignActiveNotification, object: nil, queue: .main) { _ in
                MainActor.assumeIsolated { GhosttyRuntime.running { ghostty_app_set_focus($0.app, false) } }
            },
            center.addObserver(
                forName: NSTextInputContext.keyboardSelectionDidChangeNotification, object: nil, queue: .main
            ) { _ in
                MainActor.assumeIsolated { GhosttyRuntime.running { ghostty_app_keyboard_changed($0.app) } }
            },
            center.addObserver(
                forName: UserDefaults.didChangeNotification, object: UserDefaults.standard, queue: .main
            ) { _ in
                MainActor.assumeIsolated { GhosttyRuntime.running { $0.defaultsChanged() } }
            },
        ]
        appearance = app.observe(\.effectiveAppearance) { _, _ in
            DispatchQueue.main.async {
                MainActor.assumeIsolated { GhosttyRuntime.running { $0.updateConfig() } }
            }
        }
    }

    /// Does `body` with the runtime, if it started.
    fileprivate static func running(_ body: (GhosttyRuntime) -> Void) {
        if case .success(let runtime) = shared { body(runtime) }
    }
}

// MARK: - What Ghostty calls

/// The view drawing the surface Ghostty names by its userdata.
@MainActor
private func surfaceView(_ userdata: UnsafeMutableRawPointer?) -> GhosttySurfaceView? {
    userdata.map { Unmanaged<GhosttySurfaceView>.fromOpaque($0).takeUnretainedValue() }
}

/// Ghostty has work to do; called from any thread.
private func ghosttyWakeup(_ userdata: UnsafeMutableRawPointer?) {
    DispatchQueue.main.async {
        MainActor.assumeIsolated { GhosttyRuntime.running { $0.tick() } }
    }
}

/// Something a surface or the app wants done; true when it was.
private func ghosttyAction(_ app: ghostty_app_t?, _ target: ghostty_target_s, _ action: ghostty_action_s) -> Bool {
    guard Thread.isMainThread else { return false }
    return MainActor.assumeIsolated {
        var view: GhosttySurfaceView?
        if target.tag == GHOSTTY_TARGET_SURFACE, let surface = target.target.surface {
            view = surfaceView(ghostty_surface_userdata(surface))
        }
        switch action.tag {
        case GHOSTTY_ACTION_RENDER:
            // Ghostty's renderer draws into the view's layer by itself.
            return true
        case GHOSTTY_ACTION_MOUSE_SHAPE:
            view?.setCursor(action.action.mouse_shape)
            return view != nil
        case GHOSTTY_ACTION_MOUSE_VISIBILITY:
            NSCursor.setHiddenUntilMouseMoves(action.action.mouse_visibility == GHOSTTY_MOUSE_HIDDEN)
            return true
        case GHOSTTY_ACTION_OPEN_URL:
            return openLink(action.action.open_url)
        case GHOSTTY_ACTION_SHOW_CHILD_EXITED:
            view?.processExited(code: Int32(truncatingIfNeeded: action.action.child_exited.exit_code))
            return view != nil
        case GHOSTTY_ACTION_START_SEARCH:
            let needle = action.action.start_search.needle.map { String(cString: $0) }
            view?.search?.started(needle: needle)
            return view?.search != nil
        case GHOSTTY_ACTION_END_SEARCH:
            view?.search?.ended()
            return view?.search != nil
        case GHOSTTY_ACTION_SEARCH_TOTAL:
            // Below zero while Ghostty can't say.
            let total = action.action.search_total.total
            view?.search?.counted(total >= 0 ? Int(total) : nil)
            return view?.search != nil
        case GHOSTTY_ACTION_SEARCH_SELECTED:
            let selected = action.action.search_selected.selected
            view?.search?.moved(to: selected >= 0 ? Int(selected) : nil)
            return view?.search != nil
        case GHOSTTY_ACTION_SET_TITLE, GHOSTTY_ACTION_SET_TAB_TITLE, GHOSTTY_ACTION_PWD, GHOSTTY_ACTION_RING_BELL,
            GHOSTTY_ACTION_CELL_SIZE, GHOSTTY_ACTION_SCROLLBAR, GHOSTTY_ACTION_MOUSE_OVER_LINK,
            GHOSTTY_ACTION_COLOR_CHANGE, GHOSTTY_ACTION_CONFIG_CHANGE, GHOSTTY_ACTION_RENDERER_HEALTH,
            GHOSTTY_ACTION_SELECTION_CHANGED:
            // The tabs name terminals by their folder, and nothing here
            // shows the rest.
            return true
        default:
            return false
        }
    }
}

/// Opens a link clicked in a terminal, which a program there chose: only
/// web and mail addresses, never a file or another app's scheme.
@MainActor
private func openLink(_ link: ghostty_action_open_url_s) -> Bool {
    guard let start = link.url else { return false }
    let text = String(decoding: UnsafeRawBufferPointer(start: start, count: Int(link.len)), as: UTF8.self)
    guard let url = URL(string: text), let scheme = url.scheme?.lowercased(),
        ["http", "https", "mailto"].contains(scheme)
    else { return false }
    return NSWorkspace.shared.open(url)
}

/// A surface asks for the clipboard, to paste: only its text.
private func ghosttyReadClipboard(
    _ userdata: UnsafeMutableRawPointer?, _ location: ghostty_clipboard_e, _ state: UnsafeMutableRawPointer?,
    _ mimes: UnsafePointer<UnsafePointer<CChar>?>?, _ count: Int, _ list: Bool
) -> ghostty_clipboard_read_result_e {
    guard Thread.isMainThread, location == GHOSTTY_CLIPBOARD_STANDARD else {
        return GHOSTTY_CLIPBOARD_READ_UNSUPPORTED
    }
    let started: Bool = MainActor.assumeIsolated {
        guard let surface = surfaceView(userdata)?.surface else { return false }
        var asked = false
        if let mimes {
            for index in 0..<count {
                if let mime = mimes[index], String(cString: mime) == "text/plain" { asked = true }
            }
        }
        let text = NSPasteboard.general.string(forType: .string)
        guard (asked && text != nil) || list else { return false }
        let bytes = Array((asked ? text ?? "" : "").utf8CString)
        "text/plain".withCString { mime in
            bytes.withUnsafeBufferPointer { data in
                var contents = [ghostty_clipboard_content_s]()
                if asked, text != nil {
                    contents.append(
                        ghostty_clipboard_content_s(mime: mime, data: data.baseAddress, len: bytes.count - 1))
                }
                let available: [UnsafePointer<CChar>?] = list && text != nil ? [mime] : []
                contents.withUnsafeBufferPointer { contents in
                    available.withUnsafeBufferPointer { available in
                        var complete = ghostty_clipboard_complete_s(
                            contents: contents.baseAddress, contents_len: contents.count,
                            available: available.baseAddress, available_len: available.count,
                            confirmed: false, remember: false)
                        ghostty_surface_complete_clipboard_request(surface, &complete, state)
                    }
                }
            }
        }
        return true
    }
    return started ? GHOSTTY_CLIPBOARD_READ_STARTED : GHOSTTY_CLIPBOARD_READ_UNAVAILABLE
}

/// Ghostty asks whether a read it holds unsafe may go on: a program
/// reading the clipboard, or a paste that could run commands. The surface
/// asks the person, and answers once they've chosen.
private func ghosttyConfirmReadClipboard(
    _ userdata: UnsafeMutableRawPointer?, _ confirm: UnsafePointer<ghostty_clipboard_confirm_s>?,
    _ state: UnsafeMutableRawPointer?, _ request: ghostty_clipboard_request_e
) {
    guard Thread.isMainThread else { return }
    MainActor.assumeIsolated {
        guard let view = surfaceView(userdata), let surface = view.surface else { return }
        guard let confirm = confirm?.pointee else {
            ghostty_surface_deny_clipboard_request(surface, state)
            return
        }
        // Ghostty's pointers last only for this call: the answer comes later.
        var contents: [GhosttySurfaceView.ClipboardRead.Content] = []
        if let entries = confirm.contents {
            for index in 0..<confirm.contents_len {
                let entry = entries[index]
                guard let mime = entry.mime else { continue }
                let data = entry.data.map { Data(bytes: $0, count: entry.len) } ?? Data()
                contents.append(.init(mime: String(cString: mime), data: data))
            }
        }
        var available: [String] = []
        if let mimes = confirm.available {
            for index in 0..<confirm.available_len {
                if let mime = mimes[index] { available.append(String(cString: mime)) }
            }
        }
        view.confirmClipboardRead(
            GhosttySurfaceView.ClipboardRead(
                contents: contents, available: available, program: confirm.name.map { String(cString: $0) },
                paste: request == GHOSTTY_CLIPBOARD_REQUEST_PASTE),
            state: state)
    }
}

/// A surface copies: its text goes to the clipboard, unless Ghostty wants
/// the write confirmed, which the config never asks.
private func ghosttyWriteClipboard(
    _ userdata: UnsafeMutableRawPointer?, _ location: ghostty_clipboard_e,
    _ content: UnsafePointer<ghostty_clipboard_content_s>?, _ count: Int, _ confirm: Bool
) {
    guard Thread.isMainThread, location == GHOSTTY_CLIPBOARD_STANDARD, !confirm, let content else { return }
    for index in 0..<count {
        let entry = content[index]
        guard let mime = entry.mime, String(cString: mime) == "text/plain", let data = entry.data else { continue }
        let text = String(decoding: UnsafeRawBufferPointer(start: data, count: entry.len), as: UTF8.self)
        MainActor.assumeIsolated {
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(text, forType: .string)
        }
        return
    }
}

/// A surface asks to be closed, as after a key once its process ended.
private func ghosttyCloseSurface(_ userdata: UnsafeMutableRawPointer?, _ processAlive: Bool) {
    guard Thread.isMainThread else { return }
    MainActor.assumeIsolated { surfaceView(userdata)?.processExited(code: nil) }
}
