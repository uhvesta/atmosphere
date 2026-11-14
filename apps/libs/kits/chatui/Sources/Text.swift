import SwiftUI
import UniformTypeIdentifiers

#if os(macOS)
    import AppKit
#else
    import UIKit
#endif

/// Puts text on the clipboard.
public func copyToClipboard(_ text: String) {
    #if os(macOS)
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
    #else
        UIPasteboard.general.string = text
    #endif
}

/// Puts a message on the clipboard as formatted text, for apps such as
/// Notes and Mail, and as its Markdown, for apps that only take plain text.
public func copyMessage(_ markdown: String) {
    let html = Markdown.html(markdown)
    #if os(macOS)
        NSPasteboard.general.clearContents()
        NSPasteboard.general.declareTypes([.html, .string], owner: nil)
        NSPasteboard.general.setString(html, forType: .html)
        NSPasteboard.general.setString(markdown, forType: .string)
    #else
        UIPasteboard.general.items = [[UTType.html.identifier: html, UTType.plainText.identifier: markdown]]
    #endif
}

/// Marks what a model wrote, such as its replies, so a test on a real model
/// can leave out words that differ from one GPU to the next. It's FlowKit's
/// `modelTextIdentifier`, so flow steps don't show it.
public enum ModelText {
    public static let identifier = "model-text"
}

/// Text to select part of, in its own sheet.
public struct TextToSelect: Identifiable {
    public let id = UUID()
    public let text: String

    public init(text: String) {
        self.text = text
    }
}

/// Copy and Select Text, for a message or its thoughts.
public struct TextMenu: View {
    let text: String
    let select: (String) -> Void

    public init(text: String, select: @escaping (String) -> Void) {
        self.text = text
        self.select = select
    }

    public var body: some View {
        Button("Copy", systemImage: "doc.on.doc") { copyMessage(text) }
        Button("Select Text", systemImage: "selection.pin.in.out") { select(text) }
        ShareLink(item: text)
    }
}

/// A message's text, to select any part of and copy.
public struct SelectTextSheet: View {
    let text: String

    public init(text: String) {
        self.text = text
    }

    public var body: some View {
        NavigationStack {
            SelectableText(text: text)
                .navigationTitle("Select Text")
                #if os(iOS)
                    .navigationBarTitleDisplayMode(.inline)
                #endif
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        Button("Copy All", systemImage: "doc.on.doc") { copyToClipboard(text) }
                    }
                    ToolbarItem(placement: .confirmationAction) {
                        DismissButton()
                    }
                }
        }
        .sheetLayout(height: 480)
    }
}

#if os(iOS)
    /// Text that can't be changed, where dragging selects any part of it,
    /// which SwiftUI's text selection doesn't on iOS.
    struct SelectableText: UIViewRepresentable {
        let text: String

        func makeUIView(context: Context) -> UITextView {
            let view = UITextView()
            view.isEditable = false
            view.isSelectable = true
            view.backgroundColor = .clear
            view.font = .preferredFont(forTextStyle: .body)
            view.adjustsFontForContentSizeCategory = true
            view.textColor = .label
            view.textContainerInset = UIEdgeInsets(top: 16, left: 12, bottom: 16, right: 12)
            view.accessibilityIdentifier = "selectable-text"
            return view
        }

        func updateUIView(_ view: UITextView, context: Context) {
            if view.text != text { view.text = text }
        }
    }
#else
    /// Text that can't be changed, to select any part of. A text view lays
    /// out only what's on screen, so even a long log opens at once, where
    /// SwiftUI's Text lays all of it out first.
    struct SelectableText: NSViewRepresentable {
        let text: String

        func makeNSView(context: Context) -> NSScrollView {
            let scroll = NSTextView.scrollableTextView()
            scroll.drawsBackground = false
            if let view = scroll.documentView as? NSTextView {
                view.isEditable = false
                view.isSelectable = true
                view.isRichText = false
                view.drawsBackground = false
                view.textContainerInset = NSSize(width: 12, height: 16)
                view.setAccessibilityIdentifier("selectable-text")
            }
            return scroll
        }

        func updateNSView(_ scroll: NSScrollView, context: Context) {
            // Set again only for other text: setting a long one lays it out
            // again.
            guard context.coordinator.text != text, let view = scroll.documentView as? NSTextView else { return }
            context.coordinator.text = text
            view.textStorage?.setAttributedString(
                NSAttributedString(
                    string: text,
                    attributes: [
                        .font: NSFont.preferredFont(forTextStyle: .body), .foregroundColor: NSColor.labelColor,
                    ]))
        }

        func makeCoordinator() -> Shown { Shown() }

        /// The text last put in the view.
        final class Shown {
            var text: String?
        }
    }
#endif

/// The start of a long text, for a chat to show without laying all of it
/// out: all of a short text, and whether what shows is tall enough to
/// scroll in a box of its own. Worked out once, as a text can be megabytes.
public struct TextPreview: Hashable, Sendable {
    /// What shows: the text, or its start, ending at a whole character.
    public let shown: String
    /// The whole text's size in bytes of UTF-8 when `shown` is only its
    /// start; nil when it's all of it. Bytes, not characters: counting the
    /// characters of a text of megabytes reads all of it.
    public let length: Int?
    /// Whether `shown` has more lines, or more text, than reads well in the
    /// chat's flow, so it goes in a box that scrolls.
    public let isTall: Bool

    /// Whether `shown` leaves some of the text out.
    public var isCut: Bool { length != nil }

    /// The start of `text`, up to `limit` bytes of UTF-8; tall past `lines`
    /// lines or `tallBytes` bytes, as one long line wraps to many.
    public init(_ text: String, limit: Int = 20_000, lines: Int = 24, tallBytes: Int = 2_000) {
        let utf8 = text.utf8
        if utf8.count <= limit {
            shown = text
            length = nil
        } else {
            var end = utf8.index(utf8.startIndex, offsetBy: max(limit, 0))
            // Back to the start of a character, not into the middle of one.
            while end > utf8.startIndex, UTF8.isContinuation(utf8[end]) { end = utf8.index(before: end) }
            shown = String(text[..<end])
            length = utf8.count
        }
        isTall = shown.utf8.count > tallBytes || Self.lines(in: shown, atMost: lines + 1) > lines
    }

    /// How many lines `text` has, counting no further than `limit`, so a
    /// long text isn't read to its end to learn that it's long.
    public static func lines(in text: String, atMost limit: Int) -> Int {
        var lines = 1
        guard lines < limit else { return limit }
        for byte in text.utf8 where byte == UInt8(ascii: "\n") {
            lines += 1
            if lines >= limit { return limit }
        }
        return lines
    }
}

extension View {
    /// Lets the Mac select any part of the text by dragging. On iOS a long
    /// press opens the message's menu instead, whose Select Text does.
    ///
    /// On the Mac the text becomes selectable once the pointer rests on
    /// it: selectable text costs AppKit views for each run of text, and
    /// made for every row as a chat opens or scrolls, they held up its
    /// first frames. A pointer passing over while the chat scrolls doesn't
    /// rest, so it costs nothing.
    @ViewBuilder public func selectableOnMac() -> some View {
        #if os(macOS)
            modifier(SelectableWhenPointedAt())
        #else
            self
        #endif
    }
}

#if os(macOS)
    private struct SelectableWhenPointedAt: ViewModifier {
        @State private var pointedAt = false
        @State private var selectable = false

        func body(content: Content) -> some View {
            Group {
                if selectable {
                    content.textSelection(.enabled)
                } else {
                    content
                        .onHover { pointedAt = $0 }
                        .task(id: pointedAt) {
                            guard pointedAt else { return }
                            // Long enough that a pointer carried past by a
                            // scroll doesn't count, short enough to be
                            // ready before a drag starts.
                            try? await Task.sleep(for: .milliseconds(120))
                            guard !Task.isCancelled else { return }
                            selectable = true
                        }
                }
            }
        }
    }
#endif

/// A text button in a chat row, accent-coloured as a borderless one is but
/// drawn by SwiftUI: a borderless button on the Mac is an AppKit button,
/// and measuring those as rows were laid out held up a long chat's frames.
public struct ChatRowButtonStyle: ButtonStyle {
    public init() {}

    public func makeBody(configuration: Configuration) -> some View {
        RowButton(configuration: configuration)
    }

    private struct RowButton: View {
        let configuration: Configuration
        @Environment(\.isEnabled) private var enabled
        @State private var hovering = false

        var body: some View {
            configuration.label
                .foregroundStyle(enabled ? AnyShapeStyle(.tint) : AnyShapeStyle(.tertiary))
                .opacity(configuration.isPressed ? 0.6 : hovering ? 0.85 : 1)
                .contentShape(.rect)
                .onHover { hovering = $0 }
        }
    }
}
