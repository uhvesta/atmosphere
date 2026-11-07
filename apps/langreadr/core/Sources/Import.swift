import Foundation

/// Why something couldn't be imported.
public enum ImportError: Error, Hashable, Sendable, CustomStringConvertible {
    /// It had no words to read.
    case empty(BookSource)
    /// A file of a kind the reader can't open.
    case unsupportedFile(name: String)
    /// A file that isn't text in UTF-8 or UTF-16.
    case notText(name: String)

    public var description: String {
        switch self {
        case .empty(.photo):
            "No text was found in the photo. Try a sharper photo, with the text straight and well lit."
        case .empty(.web(let url)):
            "\(url.host() ?? url.absoluteString) has no paragraphs to read. If it needs a sign-in or runs as an app, copy its text and paste it instead."
        case .empty(.file(let name)):
            "\(name) has no text in it. Choose another file, or paste its text."
        case .empty(.text):
            "There's no text to read. Paste or type some first."
        case .unsupportedFile(let name):
            "\(name) isn't a file the reader opens. Choose a .txt, .md or .html file, or copy its text and paste it."
        case .notText(let name):
            "\(name) isn't in a text encoding the reader knows. Save it as UTF-8 and import it again, or paste its text."
        }
    }
}

/// What was read from an import before its language is known.
public struct Draft: Hashable, Sendable {
    public var title: String
    public var chapters: [Chapter]
    public var source: BookSource

    public init(title: String, chapters: [Chapter], source: BookSource) {
        self.title = title
        self.chapters = chapters
        self.source = source
    }

    /// Up to `limit` characters of its text, to tell its language by.
    public func sample(limit: Int = 2_000) -> String {
        String(chapters.flatMap(\.paragraphs).map(\.text).joined(separator: "\n").prefix(limit))
    }

    public func book(in language: Language, at date: Date) -> Book {
        Book(title: title, language: language, source: source, importedAt: date, chapters: chapters)
    }
}

/// Reads text, web pages and files into chapters and paragraphs.
public enum Importer {
    /// Reads plain text. Lines that start with `#` begin chapters. Where
    /// blank lines part the paragraphs, the lines between them are joined;
    /// otherwise each line is one.
    public static func text(_ text: String, title: String? = nil, source: BookSource = .text) throws(ImportError)
        -> Draft
    {
        let lines = text.replacingOccurrences(of: "\r\n", with: "\n").replacingOccurrences(of: "\r", with: "\n")
            .components(separatedBy: "\n")
        let parted = lines.contains { $0.trimmingCharacters(in: .whitespaces).isEmpty }
        var blocks: [Block] = []
        var pending: [String] = []
        func flush() {
            if !pending.isEmpty { blocks.append(.paragraph(join(pending))) }
            pending = []
        }
        for line in lines {
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            if trimmed.hasPrefix("#") {
                flush()
                let heading = trimmed.drop(while: { $0 == "#" }).trimmingCharacters(in: .whitespaces)
                if !heading.isEmpty { blocks.append(.heading(heading)) }
            } else if trimmed.isEmpty {
                flush()
            } else if parted {
                pending.append(trimmed)
            } else {
                blocks.append(.paragraph(trimmed))
            }
        }
        flush()
        return try draft(blocks, title: title, source: source)
    }

    /// Reads a web page's headings and paragraphs, leaving out its scripts,
    /// menus, headers and footers. When it marks its main content, only
    /// that is read.
    public static func html(_ html: String, title: String? = nil, source: BookSource) throws(ImportError) -> Draft {
        let page = HTMLReader(html)
        return try draft(page.blocks, title: title ?? page.title, source: source)
    }

    /// Reads a .txt, .md or .html file.
    public static func file(_ data: Data, name: String) throws(ImportError) -> Draft {
        let kind = (name as NSString).pathExtension.lowercased()
        let source = BookSource.file(name: name)
        let title = (name as NSString).deletingPathExtension
        switch kind {
        case "txt", "text", "md", "markdown":
            return try text(decode(data, name: name), title: title, source: source)
        case "html", "htm", "xhtml":
            return try html(decode(data, name: name), source: source)
        default:
            throw .unsupportedFile(name: name)
        }
    }

    private static func decode(_ data: Data, name: String) throws(ImportError) -> String {
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

    /// Joins a paragraph's lines: with a space, except between characters of
    /// a script written without them.
    private static func join(_ lines: [String]) -> String {
        lines.dropFirst().reduce(lines.first ?? "") { text, line in
            guard let last = text.last, let first = line.first else { return text + line }
            let tight = Script.isWrittenWithoutSpaces(last) || Script.isWrittenWithoutSpaces(first)
            return text + (tight ? "" : " ") + line
        }
    }

    private static func draft(_ blocks: [Block], title: String?, source: BookSource) throws(ImportError) -> Draft {
        var chapters: [Chapter] = []
        for block in blocks {
            switch block {
            case .heading(let heading):
                chapters.append(Chapter(title: heading, paragraphs: []))
            case .paragraph(let text):
                if chapters.isEmpty { chapters.append(Chapter(paragraphs: [])) }
                chapters[chapters.count - 1].paragraphs.append(Paragraph(text: text))
            }
        }
        // A heading with nothing under it, such as a page's last, is dropped,
        // unless it's the only text there is.
        let read = chapters.filter { !$0.paragraphs.isEmpty }
        guard !read.isEmpty else { throw .empty(source) }
        let title = title?.trimmingCharacters(in: .whitespacesAndNewlines)
        let first = read.first?.title ?? read.first?.paragraphs.first?.text ?? ""
        let named = title.flatMap { $0.isEmpty ? nil : $0 } ?? first
        return Draft(title: named.count > 60 ? String(named.prefix(59)) + "…" : named, chapters: read, source: source)
    }
}

enum Block: Hashable {
    case heading(String)
    case paragraph(String)
}
