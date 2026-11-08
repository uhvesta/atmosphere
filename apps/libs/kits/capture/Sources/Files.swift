import Foundation
import PDFKit
import UniformTypeIdentifiers

/// A file the person picked to send with a message: a photo for a model
/// that reads them, or the text of a document, which goes in as context.
public enum PickedFile: Hashable, Sendable {
    case photo(Data)
    case text(FileText)

    /// The kinds of file a chat takes: text, Markdown and PDF always, and
    /// photos when its model reads them.
    public static func types(photos: Bool) -> [UTType] {
        var types: [UTType] = [.plainText, .pdf]
        if let markdown = UTType("net.daringfireball.markdown") { types.append(markdown) }
        if photos { types.append(.image) }
        return types
    }

    /// Reads the file at `url`, which may be outside the app's own files,
    /// keeping at most `limit` characters of its text.
    public static func read(_ url: URL, limit: Int) throws(FileError) -> PickedFile {
        let name = url.lastPathComponent
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        let type = UTType(filenameExtension: url.pathExtension)
        if type?.conforms(to: .image) == true {
            do {
                return .photo(try Data(contentsOf: url))
            } catch {
                throw .unreadable(name: name, reason: error.localizedDescription)
            }
        }
        let text: String
        if type?.conforms(to: .pdf) == true {
            guard let document = PDFDocument(url: url) else {
                throw .unreadable(name: name, reason: "it isn't a PDF this device opens")
            }
            text = document.string ?? ""
        } else {
            let data: Data
            do {
                data = try Data(contentsOf: url)
            } catch {
                throw .unreadable(name: name, reason: error.localizedDescription)
            }
            text = try decode(data, name: name)
        }
        return .text(try FileText(name: name, text: text, limit: limit))
    }

    /// Text in UTF-8, or UTF-16 with its byte order mark.
    static func decode(_ data: Data, name: String) throws(FileError) -> String {
        let bytes = [UInt8](data.prefix(3))
        if bytes.starts(with: [0xEF, 0xBB, 0xBF]), let text = String(data: data.dropFirst(3), encoding: .utf8) {
            return text
        }
        if bytes.starts(with: [0xFF, 0xFE]) || bytes.starts(with: [0xFE, 0xFF]),
            let text = String(data: data, encoding: .utf16)
        {
            return text
        }
        guard let text = String(data: data, encoding: .utf8) else { throw .notText(name: name) }
        return text
    }
}

/// A document's text, sent with a message for the model to read.
public struct FileText: Hashable, Sendable {
    /// The file's name, such as Notes.md.
    public var name: String
    public var text: String
    /// Whether the text was cut short to fit.
    public var isCut: Bool

    /// Keeps the first `limit` characters of `text`, trimmed.
    public init(name: String, text: String, limit: Int) throws(FileError) {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { throw .empty(name: name) }
        self.name = name
        self.text = String(trimmed.prefix(limit))
        isCut = trimmed.count > limit
    }
}

/// Why a picked file couldn't be sent.
public enum FileError: Error, Hashable, Sendable, CustomStringConvertible {
    /// It couldn't be opened, with the system's reason.
    case unreadable(name: String, reason: String)
    /// It isn't text in UTF-8 or UTF-16.
    case notText(name: String)
    /// It has no text, such as a PDF of scanned pages.
    case empty(name: String)

    public var description: String {
        switch self {
        case .unreadable(let name, let reason):
            String(
                localized: """
                    Couldn't open \(name) (\(reason)). If it's in iCloud Drive or another app's storage, check it \
                    has downloaded, then add it again.
                    """)
        case .notText(let name):
            String(localized: "\(name) isn't text in UTF-8. Save it as UTF-8 text and add it again, or paste its text.")
        case .empty(let name):
            String(
                localized: """
                    \(name) has no text to read. A PDF of scanned pages has only pictures of its text: attach a \
                    photo of a page instead, or paste the text.
                    """)
        }
    }
}
