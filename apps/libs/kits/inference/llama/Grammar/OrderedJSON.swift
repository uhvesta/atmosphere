/// JSON whose objects keep their keys in the order they're written, as a
/// schema's properties are the order the model writes them in.
enum OrderedJSON: Sendable {
    case null
    case bool(Bool)
    /// A number as it was written, so a constant reads back the same.
    case number(String)
    case string(String)
    case array([OrderedJSON])
    case object([(name: String, value: OrderedJSON)])

    /// The value written as compact JSON.
    var text: String {
        switch self {
        case .null: "null"
        case .bool(let value): value ? "true" : "false"
        case .number(let value): value
        case .string(let value): Self.quoted(value)
        case .array(let items): "[" + items.map(\.text).joined(separator: ",") + "]"
        case .object(let fields):
            "{" + fields.map { Self.quoted($0.name) + ":" + $0.value.text }.joined(separator: ",") + "}"
        }
    }

    private static func quoted(_ text: String) -> String {
        var quoted = "\""
        for scalar in text.unicodeScalars {
            switch scalar {
            case "\"": quoted += #"\""#
            case "\\": quoted += #"\\"#
            case "\n": quoted += #"\n"#
            case "\r": quoted += #"\r"#
            case "\t": quoted += #"\t"#
            case _ where scalar.value < 0x20:
                let hex = String(scalar.value, radix: 16)
                quoted += #"\u"# + String(repeating: "0", count: 4 - hex.count) + hex
            default: quoted.unicodeScalars.append(scalar)
            }
        }
        return quoted + "\""
    }

    /// Reads one JSON document from UTF-8 bytes.
    struct Reader {
        private let bytes: [UInt8]
        private var offset = 0
        /// How deep arrays and objects may nest, so a hostile schema can't
        /// overflow the stack.
        private static let depthLimit = 64

        init(_ text: String) { bytes = Array(text.utf8) }

        mutating func document() throws(SchemaGrammarError) -> OrderedJSON {
            let value = try self.value(depth: 0)
            skipSpace()
            guard offset == bytes.count else { throw .notJSON(offset: offset) }
            return value
        }

        private mutating func value(depth: Int) throws(SchemaGrammarError) -> OrderedJSON {
            guard depth < Self.depthLimit else { throw .notJSON(offset: offset) }
            skipSpace()
            guard let byte = peek() else { throw .notJSON(offset: offset) }
            switch byte {
            case UInt8(ascii: "{"):
                offset += 1
                var fields: [(name: String, value: OrderedJSON)] = []
                skipSpace()
                if peek() == UInt8(ascii: "}") {
                    offset += 1
                    return .object(fields)
                }
                while true {
                    skipSpace()
                    let name = try string()
                    try expect(":")
                    fields.append((name, try value(depth: depth + 1)))
                    skipSpace()
                    if peek() == UInt8(ascii: ",") {
                        offset += 1
                        continue
                    }
                    try expect("}")
                    return .object(fields)
                }
            case UInt8(ascii: "["):
                offset += 1
                var items: [OrderedJSON] = []
                skipSpace()
                if peek() == UInt8(ascii: "]") {
                    offset += 1
                    return .array(items)
                }
                while true {
                    items.append(try value(depth: depth + 1))
                    skipSpace()
                    if peek() == UInt8(ascii: ",") {
                        offset += 1
                        continue
                    }
                    try expect("]")
                    return .array(items)
                }
            case UInt8(ascii: "\""): return .string(try string())
            case UInt8(ascii: "t"):
                try word("true")
                return .bool(true)
            case UInt8(ascii: "f"):
                try word("false")
                return .bool(false)
            case UInt8(ascii: "n"):
                try word("null")
                return .null
            default: return .number(try number())
            }
        }

        private mutating func string() throws(SchemaGrammarError) -> String {
            guard peek() == UInt8(ascii: "\"") else { throw .notJSON(offset: offset) }
            offset += 1
            var scalars = String.UnicodeScalarView()
            var start = offset
            func flush(_ end: Int) throws(SchemaGrammarError) {
                guard let run = String(validating: bytes[start..<end], as: UTF8.self) else {
                    throw .notJSON(offset: start)
                }
                scalars.append(contentsOf: run.unicodeScalars)
            }
            while let byte = peek() {
                switch byte {
                case UInt8(ascii: "\""):
                    try flush(offset)
                    offset += 1
                    return String(scalars)
                case UInt8(ascii: "\\"):
                    try flush(offset)
                    offset += 1
                    scalars.append(try escape())
                    start = offset
                case 0..<0x20: throw .notJSON(offset: offset)
                default: offset += 1
                }
            }
            throw .notJSON(offset: offset)
        }

        private mutating func escape() throws(SchemaGrammarError) -> Unicode.Scalar {
            guard let byte = peek() else { throw .notJSON(offset: offset) }
            offset += 1
            switch byte {
            case UInt8(ascii: "\""): return "\""
            case UInt8(ascii: "\\"): return "\\"
            case UInt8(ascii: "/"): return "/"
            case UInt8(ascii: "b"): return "\u{8}"
            case UInt8(ascii: "f"): return "\u{c}"
            case UInt8(ascii: "n"): return "\n"
            case UInt8(ascii: "r"): return "\r"
            case UInt8(ascii: "t"): return "\t"
            case UInt8(ascii: "u"):
                let first = try hex()
                if (0xD800..<0xDC00).contains(first) {
                    // A surrogate pair, as JSON writes what's past the BMP.
                    try word(#"\u"#)
                    let second = try hex()
                    guard (0xDC00..<0xE000).contains(second),
                        let scalar = Unicode.Scalar(0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00))
                    else { throw .notJSON(offset: offset) }
                    return scalar
                }
                guard let scalar = Unicode.Scalar(first) else { throw .notJSON(offset: offset) }
                return scalar
            default: throw .notJSON(offset: offset - 1)
            }
        }

        private mutating func hex() throws(SchemaGrammarError) -> UInt32 {
            guard offset + 4 <= bytes.count,
                let text = String(validating: bytes[offset..<offset + 4], as: UTF8.self),
                let value = UInt32(text, radix: 16)
            else { throw .notJSON(offset: offset) }
            offset += 4
            return value
        }

        private mutating func number() throws(SchemaGrammarError) -> String {
            let start = offset
            func digits() -> Int {
                let first = offset
                while let byte = peek(), (UInt8(ascii: "0")...UInt8(ascii: "9")).contains(byte) { offset += 1 }
                return offset - first
            }
            if peek() == UInt8(ascii: "-") { offset += 1 }
            guard digits() > 0 else { throw .notJSON(offset: start) }
            if peek() == UInt8(ascii: ".") {
                offset += 1
                guard digits() > 0 else { throw .notJSON(offset: offset) }
            }
            if peek() == UInt8(ascii: "e") || peek() == UInt8(ascii: "E") {
                offset += 1
                if peek() == UInt8(ascii: "+") || peek() == UInt8(ascii: "-") { offset += 1 }
                guard digits() > 0 else { throw .notJSON(offset: offset) }
            }
            guard let number = String(validating: bytes[start..<offset], as: UTF8.self) else {
                throw .notJSON(offset: start)
            }
            return number
        }

        private mutating func word(_ word: String) throws(SchemaGrammarError) {
            for byte in word.utf8 {
                guard peek() == byte else { throw .notJSON(offset: offset) }
                offset += 1
            }
        }

        private mutating func expect(_ character: Unicode.Scalar) throws(SchemaGrammarError) {
            skipSpace()
            guard peek() == UInt8(ascii: character) else { throw .notJSON(offset: offset) }
            offset += 1
        }

        private mutating func skipSpace() {
            while let byte = peek(), byte == 0x20 || byte == 0x0A || byte == 0x0D || byte == 0x09 { offset += 1 }
        }

        private func peek() -> UInt8? { offset < bytes.count ? bytes[offset] : nil }
    }
}
