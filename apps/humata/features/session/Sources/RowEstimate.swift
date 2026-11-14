import ChatUI
import CoreGraphics
import CoreText
import Foundation

/// Guesses at how tall the chat's rows are before they're drawn, so the
/// scroll bar is about right for rows not yet seen. Text is set with Core
/// Text, not laid out as a row, so it runs off the main thread; a row once
/// measured keeps its measured height and is never guessed at again.
enum RowEstimate {
    /// About how tall each of `lines` is at `width`, the chat's own width,
    /// with text at `scale`. Takes no actor: run it detached.
    static func heights(lines: [ChatLine], items: [ChatItem], width: CGFloat, scale: TextScale) -> [CGFloat] {
        let fonts = Fonts(scale: scale)
        return lines.map { line in
            guard line.item < items.count else { return constant(for: nil, factor: fonts.factor) }
            return height(of: items[line.item], part: line.part, width: width, fonts: fonts)
        }
    }

    /// The height a row of `item`'s kind is given until its guess comes:
    /// the kind's usual height, its text ignored.
    static func constant(for item: ChatItem?, scale: TextScale) -> CGFloat {
        constant(for: item, factor: scale.factor)
    }

    // MARK: - Private

    private static func constant(for item: ChatItem?, factor: CGFloat) -> CGFloat {
        switch item {
        case .person, .fromSession: return 60 * factor
        case .reply: return 80 * factor
        case .tool(let call): return toolHeight(call, factor: factor)
        case .permission: return Chrome.permission * factor
        case .thinking: return Chrome.thinking * factor
        case .cost: return Chrome.cost * factor
        case .rewound: return Chrome.rewound * factor
        case .stopped, .compacted, .notice, .failure, nil: return Chrome.note * factor
        }
    }

    /// The heights, at the usual text size, of what a row draws around its
    /// text, or of all of it for rows whose text doesn't wrap. Taken from
    /// the rows as drawn; only the scroll bar depends on them.
    private enum Chrome {
        static let toolClosed: CGFloat = 26
        static let permission: CGFloat = 88
        static let thinking: CGFloat = 22
        static let cost: CGFloat = 16
        static let rewound: CGFloat = 24
        static let note: CGFloat = 17
        /// Around a message's bubble: its padding.
        static let bubble: CGFloat = 20
        /// Over another session's message: who sent it.
        static let sender: CGFloat = 20
        /// A sent image's thumbnail with the space under it.
        static let thumbnail: CGFloat = 72
        /// A code block's border and its header.
        static let codeBlock: CGFloat = 40
        /// A diagram's place while it's drawn.
        static let diagram: CGFloat = 200
        /// Left of a note's or failure's text: its symbol.
        static let symbol: CGFloat = 22
    }

    /// The share of the chat's width the person's bubble may take.
    private static let bubbleShare: CGFloat = 0.8

    /// The fonts rows draw with, at one text scale.
    private struct Fonts {
        let body: CTFont
        let footnote: CTFont
        /// The height of a line of code.
        let codeLine: CGFloat
        let factor: CGFloat

        init(scale: TextScale) {
            factor = scale.factor
            body = Self.font(.system, size: 13 * factor)
            footnote = Self.font(.system, size: 10 * factor)
            let mono = Self.font(.userFixedPitch, size: 12 * factor)
            codeLine = (CTFontGetAscent(mono) + CTFontGetDescent(mono) + CTFontGetLeading(mono)).rounded(.up)
        }

        private static func font(_ type: CTFontUIFontType, size: CGFloat) -> CTFont {
            CTFontCreateUIFontForLanguage(type, size, nil)
                ?? CTFontCreateWithName("Helvetica" as CFString, size, nil)
        }
    }

    private static func toolHeight(_ call: ToolCall, factor: CGFloat) -> CGFloat {
        (Chrome.toolClosed + (call.permission == nil ? 0 : Chrome.permission + 8)) * factor
    }

    private static func height(
        of item: ChatItem, part: Range<String.Index>?, width: CGFloat, fonts: Fonts
    ) -> CGFloat {
        let factor = fonts.factor
        let room = max(40, width - ChatRowLayout.padding * 2)
        switch item {
        case .person(_, let text, let images):
            let inside = max(40, room * bubbleShare - Chrome.bubble)
            let thumbnails = images.isEmpty ? 0 : Chrome.thumbnail * factor
            return markdown(text, width: inside, fonts: fonts) + Chrome.bubble * factor + thumbnails
        case .fromSession(_, let sender):
            let inside = max(40, room * bubbleShare - Chrome.bubble)
            return markdown(sender.text, width: inside, fonts: fonts) + (Chrome.bubble + Chrome.sender) * factor
        case .reply(_, let text):
            if let part, part.upperBound <= text.endIndex {
                return markdown(String(text[part]), width: room, fonts: fonts)
            }
            return markdown(text, width: room, fonts: fonts)
        case .notice(_, let text), .failure(_, let text):
            let inside = max(40, room - Chrome.symbol * factor)
            return max(Chrome.note * factor, set(text, font: fonts.footnote, width: inside))
        case .tool, .permission, .thinking, .cost, .rewound, .stopped, .compacted:
            return constant(for: item, factor: factor)
        }
    }

    /// How tall `text`, Markdown, is drawn `width` wide: its prose set
    /// with its marks taken out, its code a line height a line.
    private static func markdown(_ text: String, width: CGFloat, fonts: Fonts) -> CGFloat {
        var prose = ""
        var height: CGFloat = 0
        var fence: (diagram: Bool, lines: Int)?
        for line in text.split(separator: "\n", omittingEmptySubsequences: false) {
            let trimmed = line.drop { $0 == " " }
            if trimmed.hasPrefix("```") || trimmed.hasPrefix("~~~") {
                if let open = fence {
                    height += open.diagram ? Chrome.diagram : CGFloat(open.lines) * fonts.codeLine
                    height += Chrome.codeBlock * fonts.factor
                    fence = nil
                } else {
                    fence = (trimmed.dropFirst(3).hasPrefix("mermaid"), 0)
                }
            } else if let open = fence {
                fence = (open.diagram, open.lines + 1)
            } else {
                prose += unmarked(trimmed)
                prose += "\n"
            }
        }
        // A fence still open runs to the end, as it's drawn.
        if let open = fence { height += CGFloat(open.lines) * fonts.codeLine + Chrome.codeBlock * fonts.factor }
        return height + set(prose, font: fonts.body, width: width)
    }

    /// `line` without the marks Markdown draws as styles: emphasis, code
    /// spans, headings, quotes and a link's address.
    private static func unmarked(_ line: Substring) -> String {
        var line = line
        while let mark = line.first, mark == "#" || mark == ">" { line = line.dropFirst() }
        var kept = ""
        kept.reserveCapacity(line.utf8.count)
        var inAddress = false
        var previous: Character?
        for character in line {
            defer { previous = character }
            if inAddress {
                if character == ")" { inAddress = false }
                continue
            }
            if character == "(", previous == "]" {
                inAddress = true
                continue
            }
            switch character {
            case "*", "_", "`", "[", "]": continue
            default: kept.append(character)
            }
        }
        return kept
    }

    /// How tall `text` is set in `font`, `width` wide.
    private static func set(_ text: String, font: CTFont, width: CGFloat) -> CGFloat {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return 0 }
        let attributes = [kCTFontAttributeName as String: font] as CFDictionary
        guard let string = CFAttributedStringCreate(nil, trimmed as CFString, attributes) else { return 0 }
        let setter = CTFramesetterCreateWithAttributedString(string)
        let size = CTFramesetterSuggestFrameSizeWithConstraints(
            setter, CFRange(location: 0, length: 0), nil,
            CGSize(width: width, height: .greatestFiniteMagnitude), nil)
        return size.height.rounded(.up)
    }
}
