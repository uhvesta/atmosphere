import SwiftUI

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
        Button("Copy", systemImage: "doc.on.doc") { copyToClipboard(text) }
        Button("Select Text", systemImage: "selection.pin.in.out") { select(text) }
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
    private struct SelectableText: UIViewRepresentable {
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
    private struct SelectableText: View {
        let text: String

        var body: some View {
            ScrollView {
                Text(text)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding()
            }
        }
    }
#endif

extension View {
    /// Lets the Mac select any part of the text by dragging. On iOS a long
    /// press opens the message's menu instead, whose Select Text does.
    @ViewBuilder public func selectableOnMac() -> some View {
        #if os(macOS)
            textSelection(.enabled)
        #else
            self
        #endif
    }
}
