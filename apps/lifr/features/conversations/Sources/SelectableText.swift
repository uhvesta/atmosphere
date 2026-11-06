import LifrUI
import SwiftUI

#if os(macOS)
    import AppKit
#else
    import UIKit
#endif

/// Puts text on the clipboard.
func copyToClipboard(_ text: String) {
    #if os(macOS)
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
    #else
        UIPasteboard.general.string = text
    #endif
}

/// Text to select part of, in its own sheet.
struct TextToSelect: Identifiable {
    let id = UUID()
    let text: String
}

/// A message's text, to select any part of and copy.
struct SelectTextSheet: View {
    let text: String

    var body: some View {
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
