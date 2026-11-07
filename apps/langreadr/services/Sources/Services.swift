import Foundation
import LangReadrCore

/// A word of a paragraph, where it is in the paragraph's text.
public struct WordSpan: Hashable, Sendable {
    public var text: String
    /// Where it starts and ends, in characters.
    public var start: Int
    public var end: Int

    public init(text: String, start: Int, end: Int) {
        self.text = text
        self.start = start
        self.end = end
    }
}

/// What the reader asks of the system, and of the web, besides its model.
/// The live ones are `.live`; tests and previews give their own.
public struct ReaderServices: Sendable {
    /// The language of `text`, if it can tell.
    public var detectLanguage: @Sendable (_ text: String) -> Language?
    /// The words of `text`, in order.
    public var words: @Sendable (_ text: String, _ language: Language) -> [WordSpan]
    /// `text` in Latin letters by ICU's transform; nil when it has no rule
    /// for the script or the text is Latin already.
    public var transliterate: @Sendable (_ text: String) -> String?
    /// Says `text` aloud in `language`.
    public var speak: @MainActor @Sendable (_ text: String, _ language: Language) -> Void
    /// The lines of text in a photo, in reading order.
    public var recognizeText: @Sendable (_ image: Data) async throws(RecognitionError) -> [String]
    /// A web page's HTML.
    public var fetchPage: @Sendable (_ url: URL) async throws(PageError) -> String

    public init(
        detectLanguage: @escaping @Sendable (String) -> Language?,
        words: @escaping @Sendable (String, Language) -> [WordSpan],
        transliterate: @escaping @Sendable (String) -> String?,
        speak: @escaping @MainActor @Sendable (String, Language) -> Void,
        recognizeText: @escaping @Sendable (Data) async throws(RecognitionError) -> [String],
        fetchPage: @escaping @Sendable (URL) async throws(PageError) -> String
    ) {
        self.detectLanguage = detectLanguage
        self.words = words
        self.transliterate = transliterate
        self.speak = speak
        self.recognizeText = recognizeText
        self.fetchPage = fetchPage
    }
}

/// Why a photo's text couldn't be read.
public enum RecognitionError: Error, Hashable, Sendable, CustomStringConvertible {
    /// The data isn't an image the system can open.
    case notAnImage
    case failed(String)

    public var description: String {
        switch self {
        case .notAnImage:
            "That file isn't a photo the reader can open. Choose a JPEG, PNG or HEIC photo or screenshot."
        case .failed(let reason):
            "The photo's text couldn't be read: \(reason). Try again, or paste the text instead."
        }
    }
}

/// Why a web page couldn't be fetched.
public enum PageError: Error, Hashable, Sendable, CustomStringConvertible {
    /// Not a web link.
    case notALink(String)
    /// No connection, or the server didn't answer.
    case unreachable(host: String, reason: String)
    /// The server answered with an error status.
    case status(host: String, code: Int)
    /// The link is to something other than a web page, such as a PDF.
    case notAPage(host: String, type: String)

    public var description: String {
        switch self {
        case .notALink(let text):
            "“\(text)” isn't a web link. Paste the page's whole address, starting with https://."
        case .unreachable(let host, let reason):
            "\(host) couldn't be reached: \(reason). Check the connection and try again."
        case .status(let host, let code) where code == 401 || code == 403:
            "\(host) wouldn't show the page (\(code)): it may need a sign-in. Open it in the browser, copy its "
                + "text and paste it instead."
        case .status(let host, let code) where code == 429:
            "\(host) is limiting requests (429). Wait a few minutes, then try again."
        case .status(let host, let code):
            "\(host) answered with an error (\(code)). Check the link, or try again later."
        case .notAPage(let host, let type):
            "The link on \(host) is to a \(type) file, not a web page. Download it and import the file, if it's text."
        }
    }
}

extension ReaderServices {
    /// A link from what the person typed: with or without https://.
    public static func link(_ text: String) throws(PageError) -> URL {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        let written = trimmed.contains("://") ? trimmed : "https://" + trimmed
        guard let url = URL(string: written), let scheme = url.scheme?.lowercased(),
            scheme == "https" || scheme == "http", let host = url.host(), host.contains(".")
        else { throw .notALink(trimmed) }
        return url
    }
}
