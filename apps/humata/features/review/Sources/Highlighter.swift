import Foundation

/// Colours a line of code by its tokens: keywords, strings, comments,
/// numbers and type names. It reads one line at a time, so a comment or
/// string that spans lines is coloured on its first line only.
package struct Highlighter: Sendable {
    package enum Kind: Hashable, Sendable {
        case plain
        case keyword
        case string
        case comment
        case number
        case type
    }

    package struct Token: Hashable, Sendable {
        package var kind: Kind
        package var text: String

        package init(_ kind: Kind, _ text: String) {
            self.kind = kind
            self.text = text
        }
    }

    private let keywords: Set<String>
    private let lineComments: [String]
    /// Whether a name starting with a capital names a type.
    private let capitalsAreTypes: Bool

    /// The highlighter for a file's language, by its extension; nil when
    /// it isn't one this knows.
    package init?(path: String) {
        let name = (path as NSString).lastPathComponent.lowercased()
        let ext = (name as NSString).pathExtension
        guard let language = Self.languages.first(where: { $0.extensions.contains(ext) || $0.names.contains(name) })
        else { return nil }
        keywords = language.keywords
        lineComments = language.lineComments
        capitalsAreTypes = language.capitalsAreTypes
    }

    package func tokens(_ line: String) -> [Token] {
        var tokens: [Token] = []
        var rest = Substring(line)
        func add(_ kind: Kind, _ text: Substring) {
            if let last = tokens.last, last.kind == kind {
                tokens[tokens.count - 1].text += text
            } else {
                tokens.append(Token(kind, String(text)))
            }
        }
        while let first = rest.first {
            if lineComments.contains(where: { rest.hasPrefix($0) }) {
                add(.comment, rest)
                break
            }
            if first == "\"" || first == "'" || first == "`" {
                let end = Self.endOfString(rest, quote: first)
                add(.string, rest[..<end])
                rest = rest[end...]
            } else if first.isNumber {
                let end = rest.firstIndex { !($0.isNumber || $0.isLetter || $0 == "." || $0 == "_") } ?? rest.endIndex
                add(.number, rest[..<end])
                rest = rest[end...]
            } else if first.isLetter || first == "_" {
                let end = rest.firstIndex { !($0.isLetter || $0.isNumber || $0 == "_") } ?? rest.endIndex
                let word = rest[..<end]
                let kind: Kind =
                    keywords.contains(String(word))
                    ? .keyword : capitalsAreTypes && (word.first?.isUppercase ?? false) ? .type : .plain
                add(kind, word)
                rest = rest[end...]
            } else {
                add(.plain, rest.prefix(1))
                rest = rest.dropFirst()
            }
        }
        return tokens
    }

    /// Past the closing quote, or the line's end if it has none.
    private static func endOfString(_ text: Substring, quote: Character) -> Substring.Index {
        var index = text.index(after: text.startIndex)
        while index < text.endIndex {
            let character = text[index]
            if character == "\\" {
                index = text.index(after: index)
                if index < text.endIndex { index = text.index(after: index) }
                continue
            }
            index = text.index(after: index)
            if character == quote { return index }
        }
        return text.endIndex
    }

    private struct Language {
        var extensions: Set<String>
        var names: Set<String> = []
        var keywords: Set<String>
        var lineComments: [String]
        var capitalsAreTypes = true
    }

    private static let cKeywords: Set<String> = [
        "if", "else", "for", "while", "do", "switch", "case", "default", "break", "continue", "return", "struct",
        "enum", "const", "static", "void", "int", "char", "float", "double", "long", "unsigned", "typedef", "sizeof",
        "true", "false", "null", "new", "delete", "class", "public", "private", "protected", "this", "try", "catch",
        "throw", "import", "package", "extends", "implements", "interface", "final", "abstract",
    ]

    private static let languages: [Language] = [
        Language(
            extensions: ["swift"],
            keywords: [
                "actor", "as", "associatedtype", "async", "await", "break", "case", "catch", "class", "continue",
                "default", "defer", "do", "else", "enum", "extension", "fallthrough", "false", "fileprivate", "for",
                "func", "guard", "if", "import", "in", "init", "inout", "internal", "is", "let", "nil", "open",
                "operator", "package", "private", "protocol", "public", "repeat", "return", "self", "Self", "some",
                "any", "static", "struct", "subscript", "super", "switch", "throw", "throws", "true", "try",
                "typealias", "var", "where", "while",
            ], lineComments: ["//"]),
        Language(
            extensions: ["rs"],
            keywords: [
                "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
                "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref",
                "return", "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use",
                "where", "while",
            ], lineComments: ["//"]),
        Language(
            extensions: ["py"],
            keywords: [
                "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif", "else",
                "except", "False", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda", "None",
                "nonlocal", "not", "or", "pass", "raise", "return", "True", "try", "while", "with", "yield", "self",
            ], lineComments: ["#"]),
        Language(
            extensions: ["js", "jsx", "ts", "tsx", "mjs", "cjs"],
            keywords: [
                "async", "await", "break", "case", "catch", "class", "const", "continue", "default", "delete", "do",
                "else", "export", "extends", "false", "finally", "for", "from", "function", "if", "import", "in",
                "instanceof", "interface", "let", "new", "null", "return", "super", "switch", "this", "throw", "true",
                "try", "type", "typeof", "undefined", "var", "void", "while", "yield",
            ], lineComments: ["//"]),
        Language(
            extensions: ["go"],
            keywords: [
                "break", "case", "chan", "const", "continue", "default", "defer", "else", "fallthrough", "false",
                "for", "func", "go", "goto", "if", "import", "interface", "map", "nil", "package", "range", "return",
                "select", "struct", "switch", "true", "type", "var",
            ], lineComments: ["//"]),
        Language(
            extensions: ["c", "h", "cc", "cpp", "hpp", "m", "mm", "java", "kt", "cs"], keywords: cKeywords,
            lineComments: ["//"]),
        Language(
            extensions: ["sh", "bash", "zsh"],
            keywords: [
                "if", "then", "else", "elif", "fi", "for", "while", "do", "done", "case", "esac", "in", "function",
                "return", "export", "local", "set",
            ], lineComments: ["#"], capitalsAreTypes: false),
        Language(
            extensions: ["bzl", "bazel"], names: ["build", "workspace", "module.bazel"],
            keywords: [
                "load", "def", "return", "if", "else", "for", "in", "True", "False", "None", "and", "or", "not",
            ],
            lineComments: ["#"], capitalsAreTypes: false),
        Language(
            extensions: ["toml", "yaml", "yml"], keywords: ["true", "false"], lineComments: ["#"],
            capitalsAreTypes: false),
        Language(extensions: ["json"], keywords: ["true", "false", "null"], lineComments: [], capitalsAreTypes: false),
        Language(
            extensions: ["proto"],
            keywords: [
                "syntax", "package", "import", "message", "enum", "service", "rpc", "returns", "repeated", "optional",
                "oneof", "map", "string", "bool", "int32", "int64", "uint32", "uint64", "bytes", "double", "float",
            ], lineComments: ["//"]),
    ]
}
