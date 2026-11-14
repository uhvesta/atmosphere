import ComposableArchitecture
import Foundation

/// The keys that open Quick Add over any app on the Mac, or none. Off
/// until picked, since any keys may already mean something else. A few
/// common ones are offered, and any other can be recorded.
package struct QuickAddHotKey: Hashable, Sendable {
    /// The modifier keys held with the key.
    package struct Modifiers: OptionSet, Hashable, Sendable {
        package let rawValue: Int
        package init(rawValue: Int) { self.rawValue = rawValue }

        package static let control = Self(rawValue: 1 << 0)
        package static let option = Self(rawValue: 1 << 1)
        package static let shift = Self(rawValue: 1 << 2)
        package static let command = Self(rawValue: 1 << 3)
    }

    /// The key's virtual key code on the Mac, or nil when off.
    package var keyCode: UInt16?
    package var modifiers: Modifiers
    /// What the key shows as, such as "Space" or "K".
    package var keyName: String

    package init(keyCode: UInt16?, modifiers: Modifiers, keyName: String) {
        self.keyCode = keyCode
        self.modifiers = modifiers
        self.keyName = keyName
    }

    /// The Space bar's key code.
    private static let space: UInt16 = 49

    package static let off = Self(keyCode: nil, modifiers: [], keyName: "")
    package static let controlSpace = Self(keyCode: space, modifiers: .control, keyName: "Space")
    package static let optionSpace = Self(keyCode: space, modifiers: .option, keyName: "Space")
    package static let controlOptionSpace = Self(keyCode: space, modifiers: [.control, .option], keyName: "Space")
    package static let shiftCommandSpace = Self(keyCode: space, modifiers: [.shift, .command], keyName: "Space")

    /// The ones offered without recording.
    package static let presets: [Self] = [.off, .controlSpace, .optionSpace, .controlOptionSpace, .shiftCommandSpace]

    package var isOff: Bool { keyCode == nil }

    /// Whether the keys can open Quick Add: a key held with Control,
    /// Option or Command, so typing in other apps never opens it.
    package var isUsable: Bool {
        keyCode != nil && !modifiers.isDisjoint(with: [.control, .option, .command])
    }

    package var title: String {
        guard !isOff else { return String(localized: "Off") }
        var symbols = ""
        if modifiers.contains(.control) { symbols += "⌃" }
        if modifiers.contains(.option) { symbols += "⌥" }
        if modifiers.contains(.shift) { symbols += "⇧" }
        if modifiers.contains(.command) { symbols += "⌘" }
        return symbols + (keyCode == Self.space ? String(localized: "Space") : keyName)
    }
}

extension QuickAddHotKey: RawRepresentable {
    /// "keyCode:modifiers:name", or "off". The names stored before keys
    /// could be recorded still read.
    package init?(rawValue: String) {
        switch rawValue {
        case "off": self = .off
        case "controlSpace": self = .controlSpace
        case "optionSpace": self = .optionSpace
        case "controlOptionSpace": self = .controlOptionSpace
        case "shiftCommandSpace": self = .shiftCommandSpace
        default:
            let parts = rawValue.split(separator: ":", maxSplits: 2, omittingEmptySubsequences: false)
            guard parts.count == 3, let code = UInt16(parts[0]), let modifiers = Int(parts[1]), !parts[2].isEmpty
            else { return nil }
            self.init(keyCode: code, modifiers: Modifiers(rawValue: modifiers), keyName: String(parts[2]))
        }
    }

    package var rawValue: String {
        guard let keyCode else { return "off" }
        return "\(keyCode):\(modifiers.rawValue):\(keyName)"
    }
}

extension SharedKey where Self == AppStorageKey<QuickAddHotKey>.Default {
    package static var quickAddHotKey: Self {
        Self[.appStorage("lifrQuickAddHotKey"), default: .off]
    }
}
