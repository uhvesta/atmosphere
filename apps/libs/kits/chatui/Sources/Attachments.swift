import SwiftUI

#if os(macOS)
    import AppKit
#else
    import UIKit
#endif

/// An attachment opened from the chat: a photo to look at closely, or a
/// file's text to read.
public enum OpenAttachment: Identifiable {
    case photo(URL)
    case file(name: String, text: String)

    public var id: String {
        switch self {
        case .photo(let url): "photo:\(url.absoluteString)"
        case .file(let name, let text): "file:\(name):\(text.hashValue)"
        }
    }
}

extension View {
    /// Shows the attachment `item` opens, until it's closed.
    public func attachmentViewer(_ item: Binding<OpenAttachment?>) -> some View {
        sheet(item: item) { attachment in
            switch attachment {
            case .photo(let url): PhotoViewer(file: url)
            case .file(let name, let text): FileViewer(name: name, text: text)
            }
        }
    }
}

/// A photo from the chat on its own: pinch or double-tap to zoom, and
/// Share to send it on.
struct PhotoViewer: View {
    let file: URL
    @State private var zoom: CGFloat = 1
    @GestureState private var pinch: CGFloat = 1

    var body: some View {
        NavigationStack {
            Group {
                if let image = PhotoFile.image(file) {
                    ScrollView([.horizontal, .vertical]) {
                        image
                            .resizable()
                            .scaledToFit()
                            .containerRelativeFrame([.horizontal, .vertical]) { length, _ in
                                length * min(max(zoom * pinch, 1), 5)
                            }
                    }
                    .scrollIndicators(.hidden)
                    .defaultScrollAnchor(.center)
                    .gesture(
                        MagnifyGesture()
                            .updating($pinch) { value, state, _ in state = value.magnification }
                            .onEnded { zoom = min(max(zoom * $0.magnification, 1), 5) }
                    )
                    .onTapGesture(count: 2) {
                        withAnimation(.snappy) { zoom = zoom > 1 ? 1 : 2.5 }
                    }
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel(Text("Photo"))
                    .accessibilityAddTraits(.isImage)
                } else {
                    ContentUnavailableView(
                        "Can't Show This Photo", systemImage: "photo",
                        description: Text("Its file couldn't be read. Try attaching the photo again."))
                }
            }
            .navigationTitle("Photo")
            #if os(iOS)
                .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    ShareLink(item: file, preview: SharePreview(Text("Photo")))
                }
                ToolbarItem(placement: .confirmationAction) {
                    DismissButton()
                }
            }
        }
        .sheetLayout(width: 640, height: 560)
    }
}

/// A file sent in the chat: the text the model read from it, to read and
/// select, formatted when it's Markdown, with Copy All and Share.
struct FileViewer: View {
    let name: String
    let text: String

    var body: some View {
        NavigationStack {
            Group {
                if text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    ContentUnavailableView(
                        "No Text", systemImage: "doc",
                        description: Text("No text could be read from this file, so the model didn't see any."))
                } else if Self.isMarkdown(name) {
                    ScrollView {
                        MarkdownText(text: text)
                            .selectableOnMac()
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding()
                    }
                } else {
                    SelectableText(text: text)
                }
            }
            .navigationTitle(Text(verbatim: name))
            #if os(iOS)
                .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItemGroup(placement: .cancellationAction) {
                    // A Mac sheet's bar shows one item of a group, so the
                    // two go in a row of their own.
                    #if os(macOS)
                        HStack {
                            Button("Copy All", systemImage: "doc.on.doc") { copyToClipboard(text) }
                            ShareLink(item: text, subject: Text(verbatim: name))
                        }
                    #else
                        Button("Copy All", systemImage: "doc.on.doc") { copyToClipboard(text) }
                        ShareLink(item: text, subject: Text(verbatim: name))
                    #endif
                }
                ToolbarItem(placement: .confirmationAction) {
                    DismissButton()
                }
            }
        }
        .sheetLayout(width: 560, height: 520)
    }

    /// Whether the file's name says it's Markdown, which reads formatted.
    private static func isMarkdown(_ name: String) -> Bool {
        ["md", "markdown"].contains((name as NSString).pathExtension.lowercased())
    }
}

/// Reads a photo the app keeps on the device.
public enum PhotoFile {
    public static func image(_ file: URL) -> Image? {
        #if canImport(UIKit)
            UIImage(contentsOfFile: file.path).map { Image(uiImage: $0) }
        #else
            NSImage(contentsOf: file).map { Image(nsImage: $0) }
        #endif
    }

    /// Puts the photo in `file` on the clipboard, to paste elsewhere.
    static func copy(_ file: URL) {
        #if canImport(UIKit)
            guard let image = UIImage(contentsOfFile: file.path) else { return }
            UIPasteboard.general.image = image
        #else
            guard let image = NSImage(contentsOf: file) else { return }
            NSPasteboard.general.clearContents()
            NSPasteboard.general.writeObjects([image])
        #endif
    }
}

/// What a photo in the chat offers when held or right-clicked: Copy, and
/// Share, which can also save it.
public struct PhotoMenu: View {
    let file: URL

    public init(file: URL) {
        self.file = file
    }

    public var body: some View {
        Button("Copy", systemImage: "doc.on.doc") { PhotoFile.copy(file) }
        ShareLink(item: file, preview: SharePreview(Text("Photo")))
    }
}
