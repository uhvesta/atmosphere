import SwiftUI

/// What a part of code is, to color it by.
enum CodeToken: Hashable, Sendable {
    case keyword
    case string
    case comment
    case number
}

/// How a language writes comments, strings and keywords: enough to color
/// the code a model writes, not to check it.
struct CodeSyntax: Sendable {
    var lineComments: [String]
    var blockComments: Bool
    var quotes: Set<Character>
    var keywords: Set<String>
    /// Whether keywords match in any case, as in SQL.
    var anyCase = false

    /// The syntax for a fence's language, or nil for text that isn't code,
    /// such as a fence with no language, which is often a diagram or output.
    static func of(_ language: String?) -> CodeSyntax? {
        guard let language = language?.lowercased(), !plain.contains(language) else { return nil }
        switch language {
        case "py", "python", "rb", "ruby", "r", "perl", "pl":
            return CodeSyntax(lineComments: ["#"], blockComments: false, quotes: ["\"", "'"], keywords: common)
        case "sh", "bash", "zsh", "shell", "console", "fish", "powershell", "ps1":
            return CodeSyntax(lineComments: ["#"], blockComments: false, quotes: ["\"", "'"], keywords: shell)
        case "yaml", "yml", "toml", "ini", "dockerfile", "makefile", "make":
            return CodeSyntax(lineComments: ["#"], blockComments: false, quotes: ["\"", "'"], keywords: literals)
        case "json", "jsonc":
            return CodeSyntax(lineComments: ["//"], blockComments: true, quotes: ["\""], keywords: literals)
        case "sql", "sqlite", "postgres", "postgresql", "mysql":
            return CodeSyntax(
                lineComments: ["--"], blockComments: true, quotes: ["'", "\""], keywords: sql, anyCase: true)
        case "lua", "haskell", "hs":
            return CodeSyntax(lineComments: ["--"], blockComments: false, quotes: ["\"", "'"], keywords: common)
        case "swift", "rust", "rs":
            return CodeSyntax(lineComments: ["//"], blockComments: true, quotes: ["\""], keywords: common)
        case "js", "javascript", "jsx", "ts", "typescript", "tsx", "go":
            return CodeSyntax(lineComments: ["//"], blockComments: true, quotes: ["\"", "'", "`"], keywords: common)
        default:
            return CodeSyntax(lineComments: ["//"], blockComments: true, quotes: ["\"", "'"], keywords: common)
        }
    }

    /// `code` cut into runs, each with what it is, or nil for the rest.
    func runs(_ code: String) -> [(text: String, token: CodeToken?)] {
        let chars = Array(code)
        var runs: [(text: String, token: CodeToken?)] = []
        var index = 0
        func add(_ end: Int, _ token: CodeToken?) {
            let text = String(chars[index..<end])
            if token == nil, let last = runs.last, last.token == nil {
                runs[runs.count - 1].text += text
            } else {
                runs.append((text, token))
            }
            index = end
        }
        while index < chars.count {
            let char = chars[index]
            if lineComments.contains(where: { Self.starts(chars, at: index, with: $0) }) {
                add(chars[index...].firstIndex(of: "\n") ?? chars.count, .comment)
            } else if blockComments, Self.starts(chars, at: index, with: "/*") {
                var end = index + 2
                while end < chars.count, !Self.starts(chars, at: end, with: "*/") { end += 1 }
                add(min(end + 2, chars.count), .comment)
            } else if quotes.contains(char) {
                add(stringEnd(chars, from: index), .string)
            } else if char.isNumber, index == 0 || !Self.isWord(chars[index - 1]) {
                var end = index + 1
                while end < chars.count, chars[end].isHexDigit || "xXoObB._".contains(chars[end]) { end += 1 }
                add(end, .number)
            } else if Self.isWord(char) {
                var end = index + 1
                while end < chars.count, Self.isWord(chars[end]) { end += 1 }
                let word = String(chars[index..<end])
                add(end, keywords.contains(anyCase ? word.lowercased() : word) ? .keyword : nil)
            } else {
                add(index + 1, nil)
            }
        }
        return runs
    }

    /// Past the quote that closes the string opening at `start`, which
    /// ends at the line's end unless it's backticked.
    private func stringEnd(_ chars: [Character], from start: Int) -> Int {
        let quote = chars[start]
        var end = start + 1
        while end < chars.count {
            if chars[end] == "\\" {
                end += 2
                continue
            }
            if chars[end] == quote { return end + 1 }
            if chars[end] == "\n", quote != "`" { return end }
            end += 1
        }
        return chars.count
    }

    private static func starts(_ chars: [Character], at index: Int, with prefix: String) -> Bool {
        var position = index
        for char in prefix {
            guard position < chars.count, chars[position] == char else { return false }
            position += 1
        }
        return true
    }

    private static func isWord(_ char: Character) -> Bool { char.isLetter || char.isNumber || char == "_" }

    private static let plain: Set<String> = ["text", "txt", "plaintext", "plain", "markdown", "md", "output", "log"]

    private static let literals: Set<String> = ["true", "false", "null", "nil", "none", "yes", "no"]

    private static let common: Set<String> = [
        "and", "as", "async", "await", "break", "case", "catch", "class", "const", "continue", "def", "default",
        "defer", "del", "do", "elif", "else", "end", "enum", "except", "export", "extends", "extension", "false",
        "final", "finally", "fn", "for", "from", "func", "function", "go", "guard", "if", "impl", "implements",
        "import", "in", "init", "interface", "is", "lambda", "let", "local", "match", "mod", "mut", "new", "nil",
        "None", "not", "null", "or", "package", "pass", "private", "protocol", "pub", "public", "raise", "return",
        "self", "Self", "static", "struct", "super", "switch", "then", "this", "throw", "throws", "trait", "True",
        "False", "true", "try", "type", "typeof", "use", "val", "var", "void", "where", "while", "with", "yield",
    ]

    private static let shell: Set<String> = [
        "case", "do", "done", "elif", "else", "esac", "export", "fi", "for", "function", "if", "in", "local",
        "return", "then", "until", "while",
    ]

    private static let sql: Set<String> = [
        "alter", "and", "as", "asc", "by", "create", "delete", "desc", "distinct", "drop", "from", "group",
        "having", "index", "insert", "into", "is", "join", "left", "limit", "not", "null", "on", "or", "order",
        "primary", "key", "select", "set", "table", "update", "values", "where", "with",
    ]
}

extension CodeSyntax {
    /// Code longer than this shows in one color, so a long block doesn't
    /// slow the chat while it's written.
    static let longest = 20_000

    /// `code` colored for `language`, or as it is when it isn't code.
    static func colored(_ code: String, language: String?) -> AttributedString {
        guard code.count <= longest, let syntax = of(language) else { return AttributedString(code) }
        var string = AttributedString()
        for run in syntax.runs(code) {
            var part = AttributedString(run.text)
            switch run.token {
            case .keyword?: part.foregroundColor = .pink
            case .string?: part.foregroundColor = .orange
            case .comment?: part.foregroundColor = .secondary
            case .number?: part.foregroundColor = .blue
            case nil: break
            }
            string += part
        }
        return string
    }
}
