import Foundation

/// Reads GBNF as llama.cpp does and says whether a grammar takes a whole
/// text, so a test can check what a model is held to rather than how the
/// grammar is spelled.
struct GBNF {
    indirect enum Expression {
        case sequence([Expression])
        case choice([Expression])
        case literal([Unicode.Scalar])
        case characters(negated: Bool, [ClosedRange<UInt32>])
        case rule(String)
        case repeated(Expression, min: Int, max: Int?)
    }

    struct ParseError: Error, CustomStringConvertible {
        var description: String
    }

    private(set) var rules: [String: Expression] = [:]

    init(_ text: String) throws(ParseError) {
        for line in text.split(separator: "\n") where !line.trimmingCharacters(in: .whitespaces).isEmpty {
            guard let range = line.range(of: "::=") else { throw ParseError(description: "No ::= in \(line)") }
            let name = line[..<range.lowerBound].trimmingCharacters(in: .whitespaces)
            var parser = Parser(scalars: Array(line[range.upperBound...].unicodeScalars))
            let body = try parser.choice()
            guard parser.atEnd else { throw ParseError(description: "Unread text in \(line)") }
            guard rules[name] == nil else { throw ParseError(description: "\(name) is defined twice") }
            rules[name] = body
        }
        for name in Self.references(in: Array(rules.values)) where rules[name] == nil {
            throw ParseError(description: "\(name) is used but not defined")
        }
    }

    /// Whether `root` takes all of `text`.
    func accepts(_ text: String) -> Bool {
        guard let root = rules["root"] else { return false }
        let scalars = Array(text.unicodeScalars)
        return ends(of: root, in: scalars, from: 0).contains(scalars.count)
    }

    private static func references(in expressions: [Expression]) -> Set<String> {
        var names: Set<String> = []
        for expression in expressions {
            switch expression {
            case .sequence(let items), .choice(let items): names.formUnion(references(in: items))
            case .rule(let name): names.insert(name)
            case .repeated(let item, _, _): names.formUnion(references(in: [item]))
            case .literal, .characters: break
            }
        }
        return names
    }

    /// Every place a match of `expression` starting at `start` can end.
    private func ends(of expression: Expression, in text: [Unicode.Scalar], from start: Int) -> Set<Int> {
        switch expression {
        case .literal(let literal):
            guard start + literal.count <= text.count, Array(text[start..<start + literal.count]) == literal else {
                return []
            }
            return [start + literal.count]
        case .characters(let negated, let ranges):
            guard start < text.count else { return [] }
            let inside = ranges.contains { $0.contains(text[start].value) }
            return inside != negated ? [start + 1] : []
        case .rule(let name):
            guard let body = rules[name] else { return [] }
            return ends(of: body, in: text, from: start)
        case .choice(let options):
            return options.reduce(into: Set<Int>()) { $0.formUnion(ends(of: $1, in: text, from: start)) }
        case .sequence(let items):
            return items.reduce([start]) { positions, item in
                positions.reduce(into: Set<Int>()) { $0.formUnion(ends(of: item, in: text, from: $1)) }
            }
        case .repeated(let item, let min, let max):
            var reached: Set<Int> = min == 0 ? [start] : []
            var frontier: Set<Int> = [start]
            var count = 0
            while !frontier.isEmpty, max.map({ count < $0 }) ?? true {
                count += 1
                let next = frontier.reduce(into: Set<Int>()) { $0.formUnion(ends(of: item, in: text, from: $1)) }
                // An item that matches nothing new would repeat forever.
                frontier = next.subtracting(reached).subtracting(frontier)
                if count >= min { reached.formUnion(next) }
                if count < min { frontier = next }
            }
            return reached
        }
    }

    private struct Parser {
        let scalars: [Unicode.Scalar]
        var offset = 0

        var atEnd: Bool {
            mutating get {
                skipSpace()
                return offset == scalars.count
            }
        }

        mutating func choice() throws(ParseError) -> Expression {
            var options = [try sequence()]
            while peek() == "|" {
                offset += 1
                options.append(try sequence())
            }
            return options.count == 1 ? options[0] : .choice(options)
        }

        private mutating func sequence() throws(ParseError) -> Expression {
            var items: [Expression] = []
            while true {
                skipSpace()
                guard let scalar = peek(), scalar != "|", scalar != ")" else { break }
                var item = try primary()
                while let suffix = peek() {
                    if suffix == "?" {
                        item = .repeated(item, min: 0, max: 1)
                    } else if suffix == "*" {
                        item = .repeated(item, min: 0, max: nil)
                    } else if suffix == "+" {
                        item = .repeated(item, min: 1, max: nil)
                    } else if suffix == "{" {
                        offset += 1
                        let low = number()
                        var high: Int? = low
                        if peek() == "," {
                            offset += 1
                            high = peek() == "}" ? nil : number()
                        }
                        guard peek() == "}" else { throw ParseError(description: "Unclosed {") }
                        item = .repeated(item, min: low, max: high)
                    } else {
                        break
                    }
                    offset += 1
                }
                items.append(item)
            }
            return .sequence(items)
        }

        private mutating func primary() throws(ParseError) -> Expression {
            guard let scalar = peek() else { throw ParseError(description: "Ended early") }
            switch scalar {
            case "(":
                offset += 1
                let inner = try choice()
                skipSpace()
                guard peek() == ")" else { throw ParseError(description: "Unclosed (") }
                offset += 1
                return inner
            case "\"":
                offset += 1
                var literal: [Unicode.Scalar] = []
                while let next = peek(), next != "\"" {
                    literal.append(try character())
                }
                guard peek() == "\"" else { throw ParseError(description: "Unclosed quote") }
                offset += 1
                return .literal(literal)
            case "[":
                offset += 1
                var negated = false
                if peek() == "^" {
                    negated = true
                    offset += 1
                }
                var ranges: [ClosedRange<UInt32>] = []
                while let next = peek(), next != "]" {
                    let low = try character()
                    if peek() == "-", offset + 1 < scalars.count, scalars[offset + 1] != "]" {
                        offset += 1
                        let high = try character()
                        ranges.append(low.value...high.value)
                    } else {
                        ranges.append(low.value...low.value)
                    }
                }
                guard peek() == "]" else { throw ParseError(description: "Unclosed [") }
                offset += 1
                return .characters(negated: negated, ranges)
            default:
                var name = ""
                while let next = peek(), next.properties.isAlphabetic || ("0"..."9").contains(next) || next == "-" {
                    name.unicodeScalars.append(next)
                    offset += 1
                }
                guard !name.isEmpty else { throw ParseError(description: "Unexpected \(scalar)") }
                return .rule(name)
            }
        }

        private mutating func character() throws(ParseError) -> Unicode.Scalar {
            guard let scalar = peek() else { throw ParseError(description: "Ended early") }
            offset += 1
            guard scalar == "\\" else { return scalar }
            guard let escaped = peek() else { throw ParseError(description: "Ended in an escape") }
            offset += 1
            switch escaped {
            case "n": return "\n"
            case "r": return "\r"
            case "t": return "\t"
            case "x":
                guard offset + 2 <= scalars.count,
                    let value = UInt32(String(String.UnicodeScalarView(scalars[offset..<offset + 2])), radix: 16),
                    let result = Unicode.Scalar(value)
                else { throw ParseError(description: "Bad \\x") }
                offset += 2
                return result
            default: return escaped
            }
        }

        private mutating func number() -> Int {
            var value = 0
            while let scalar = peek(), let digit = Int(String(scalar)) {
                value = value * 10 + digit
                offset += 1
            }
            return value
        }

        private mutating func skipSpace() {
            while let scalar = peek(), scalar == " " || scalar == "\t" { offset += 1 }
        }

        private func peek() -> Unicode.Scalar? { offset < scalars.count ? scalars[offset] : nil }
    }
}
