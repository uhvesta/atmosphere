import Foundation

/// Code put in the message box as a quote: where it's from, the code
/// fenced, and what the person says about it.
package enum CodeQuote {
    /// `code` from lines `start` to `end` of `path`, fenced as `language`,
    /// with `comment` under it. With no path, only the code.
    package static func text(
        path: String?, start: Int?, end: Int?, language: String, code: String, comment: String = ""
    ) -> String {
        var text = ""
        if let path {
            text += "`\(path)`"
            if let start, let end { text += start == end ? " line \(start)" : " lines \(start)–\(end)" }
            text += ":\n"
        }
        // A fence longer than any run of backticks in the code, so it can't
        // close early.
        let longest = code.split(whereSeparator: { $0 != "`" }).map(\.count).max() ?? 0
        let fence = String(repeating: "`", count: max(3, longest + 1))
        text += "\(fence)\(language)\n\(code)\n\(fence)"
        let comment = comment.trimmingCharacters(in: .whitespacesAndNewlines)
        if !comment.isEmpty { text += "\n\n\(comment)" }
        return text
    }

    /// The fence's language for a file, from its extension.
    package static func language(of path: String) -> String {
        (path as NSString).pathExtension.lowercased()
    }
}
