import SwiftUI

#if os(macOS)
    import AppKit
#else
    import UIKit
#endif

/// How big text is drawn, as a percentage of its usual size: a step is ten
/// points, from 80% to 200%. Whole percentages, so steps up and back down
/// land on the same sizes.
public struct TextScale: Hashable, Sendable {
    /// The sizes a scale can be.
    public static let percents: ClosedRange<Int> = 80...200
    /// Text at its usual size.
    public static let actual = TextScale(percent: 100)

    private static let step = 10

    public let percent: Int

    /// The scale nearest `percent` that's a whole step within `percents`.
    public init(percent: Int) {
        let held = min(max(percent, Self.percents.lowerBound), Self.percents.upperBound)
        // To the nearest step, as a kept value may be one this never wrote.
        self.percent = (held + Self.step / 2) / Self.step * Self.step
    }

    /// One step bigger; itself at the biggest.
    public var bigger: TextScale { TextScale(percent: percent + Self.step) }
    /// One step smaller; itself at the smallest.
    public var smaller: TextScale { TextScale(percent: percent - Self.step) }

    public var canGrow: Bool { percent < Self.percents.upperBound }
    public var canShrink: Bool { percent > Self.percents.lowerBound }

    /// What a size in points is multiplied by.
    public var factor: CGFloat { CGFloat(percent) / 100 }
}

extension EnvironmentValues {
    /// How big the text of a chat and of what's read beside it is drawn.
    /// Views take their fonts with `scaledFont` to follow it.
    @Entry public var textScale = TextScale.actual
}

/// A font that follows the text scale, written as a `Font` is: `.caption`,
/// `.callout.weight(.medium)`, `.system(size: 12, design: .monospaced)`.
///
/// At the usual size it's the system's own font for the style, so nothing
/// changes, and on iOS it still follows Dynamic Type. At any other it's
/// the system font at the style's size in points times the scale: on the
/// Mac a text style has one size, and nothing in SwiftUI scales it.
public struct ScaledFont: Equatable, Sendable {
    private enum Base: Equatable, Sendable {
        case style(Font.TextStyle)
        case size(CGFloat)
    }

    private var base: Base
    private var design: Font.Design?
    /// The weight asked for, over the style's own.
    private var asked: Font.Weight?
    private var isItalic = false
    private var isMonospaced = false
    private var isMonospacedDigit = false

    public static let largeTitle = ScaledFont(base: .style(.largeTitle))
    public static let title = ScaledFont(base: .style(.title))
    public static let title2 = ScaledFont(base: .style(.title2))
    public static let title3 = ScaledFont(base: .style(.title3))
    public static let headline = ScaledFont(base: .style(.headline))
    public static let subheadline = ScaledFont(base: .style(.subheadline))
    public static let body = ScaledFont(base: .style(.body))
    public static let callout = ScaledFont(base: .style(.callout))
    public static let footnote = ScaledFont(base: .style(.footnote))
    public static let caption = ScaledFont(base: .style(.caption))
    public static let caption2 = ScaledFont(base: .style(.caption2))

    public static func system(_ style: Font.TextStyle, design: Font.Design? = nil) -> ScaledFont {
        ScaledFont(base: .style(style), design: design)
    }

    public static func system(size: CGFloat, design: Font.Design? = nil) -> ScaledFont {
        ScaledFont(base: .size(size), design: design)
    }

    public func weight(_ weight: Font.Weight) -> ScaledFont {
        var font = self
        font.asked = weight
        return font
    }

    public func bold() -> ScaledFont { weight(.bold) }

    public func italic() -> ScaledFont {
        var font = self
        font.isItalic = true
        return font
    }

    public func monospaced() -> ScaledFont {
        var font = self
        font.isMonospaced = true
        return font
    }

    public func monospacedDigit() -> ScaledFont {
        var font = self
        font.isMonospacedDigit = true
        return font
    }

    /// The font to draw with at `scale`.
    public func font(at scale: TextScale) -> Font {
        var font: Font
        switch base {
        case .style(let style):
            if scale == .actual {
                font = .system(style, design: design)
            } else {
                font = .system(
                    size: Self.points(of: style) * scale.factor, weight: Self.weight(of: style), design: design)
            }
        case .size(let size):
            font = .system(size: size * scale.factor, design: design)
        }
        if let asked { font = font.weight(asked) }
        if isItalic { font = font.italic() }
        if isMonospaced { font = font.monospaced() }
        if isMonospacedDigit { font = font.monospacedDigit() }
        return font
    }

    /// The size in points the system draws `style` at, before any scale.
    private static func points(of style: Font.TextStyle) -> CGFloat {
        #if os(macOS)
            NSFont.preferredFont(forTextStyle: platformStyle(style)).pointSize
        #else
            UIFont.preferredFont(forTextStyle: platformStyle(style)).pointSize
        #endif
    }

    /// The weight the system draws `style` in, where it isn't regular.
    private static func weight(of style: Font.TextStyle) -> Font.Weight? {
        switch style {
        case .headline: return headlineWeight
        case .caption2: return caption2Weight
        default: return nil
        }
    }

    #if os(macOS)
        private typealias PlatformStyle = NSFont.TextStyle
        private static let headlineWeight: Font.Weight? = .bold
        private static let caption2Weight: Font.Weight? = .medium
    #else
        private typealias PlatformStyle = UIFont.TextStyle
        private static let headlineWeight: Font.Weight? = .semibold
        private static let caption2Weight: Font.Weight? = nil
    #endif

    private static func platformStyle(_ style: Font.TextStyle) -> PlatformStyle {
        switch style {
        case .largeTitle: .largeTitle
        case .title: .title1
        case .title2: .title2
        case .title3: .title3
        case .headline: .headline
        case .subheadline: .subheadline
        case .callout: .callout
        case .footnote: .footnote
        case .caption: .caption1
        case .caption2: .caption2
        default: .body
        }
    }
}

extension View {
    /// Sets the font as `font(_:)` does, at the size the text scale gives.
    public func scaledFont(_ font: ScaledFont) -> some View {
        modifier(FontAtScale(font: font))
    }

    /// Draws text that's given no font of its own at the text scale: for
    /// the top of what scales, as `scaledFont` is for each font in it. It
    /// leaves a font set further out as it is, and at the usual size sets
    /// none, so controls keep the fonts of their sizes.
    public func scaledBody() -> some View {
        modifier(BodyAtScale())
    }

    /// A fixed width that holds text, such as line numbers, widened and
    /// narrowed with the text scale.
    public func scaledFrame(width: CGFloat, alignment: Alignment = .center) -> some View {
        modifier(WidthAtScale(width: width, alignment: alignment))
    }
}

private struct FontAtScale: ViewModifier {
    let font: ScaledFont
    @Environment(\.textScale) private var scale

    func body(content: Content) -> some View {
        content.font(font.font(at: scale))
    }
}

private struct BodyAtScale: ViewModifier {
    @Environment(\.textScale) private var scale
    @Environment(\.font) private var set

    func body(content: Content) -> some View {
        content.font(set ?? (scale == .actual ? nil : ScaledFont.body.font(at: scale)))
    }
}

private struct WidthAtScale: ViewModifier {
    let width: CGFloat
    let alignment: Alignment
    @Environment(\.textScale) private var scale

    func body(content: Content) -> some View {
        content.frame(width: width * scale.factor, alignment: alignment)
    }
}
