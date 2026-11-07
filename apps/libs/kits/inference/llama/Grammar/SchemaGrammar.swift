/// A grammar in llama.cpp's GBNF that holds a model to JSON matching a
/// JSON Schema, so a constrained turn always decodes.
///
/// It covers the schemas the app sends: `type` (one or a list), `enum`,
/// `const`, `anyOf` and `oneOf`, an object's `properties` in the order
/// they're listed, `required` and `additionalProperties`, an array's
/// `items`, and `description`, which only the prompt reads. An object
/// takes no properties beyond those it lists, so the model can't wander
/// off into fields nothing reads.
public enum SchemaGrammar {
    /// The grammar for `schema`, a JSON Schema as text. Its rule `root` is
    /// the whole answer.
    public static func gbnf(for schema: String) throws(SchemaGrammarError) -> String {
        var reader = OrderedJSON.Reader(schema)
        let node = try reader.document()
        var builder = Builder()
        let answer = try builder.rule(for: node, named: "answer")
        builder.define("root", "ws " + answer + " ws")
        return builder.text
    }
}

/// Why a schema has no grammar.
public enum SchemaGrammarError: Error, Equatable, Sendable {
    /// The schema isn't JSON: `offset` is the byte where reading stopped.
    case notJSON(offset: Int)
    /// The schema uses `keyword`, which the grammar can't hold a model to.
    case unsupported(keyword: String)
    /// `keyword` holds something a schema doesn't allow there.
    case invalid(keyword: String)
}

extension SchemaGrammarError: CustomStringConvertible {
    public var description: String {
        switch self {
        case .notJSON(let offset):
            "The answer's JSON Schema isn't valid JSON (at byte \(offset)), so the model can't be held to it. "
                + "Fix the schema the app builds for this turn."
        case .unsupported(let keyword):
            "The answer's JSON Schema uses \"\(keyword)\", which llama.cpp's grammar here doesn't support. "
                + "Describe the answer without it, or run the turn on a model that reads JSON Schema itself."
        case .invalid(let keyword):
            "The answer's JSON Schema has a \"\(keyword)\" that isn't valid JSON Schema. "
                + "Fix the schema the app builds for this turn."
        }
    }
}

/// Builds the rules, naming each after where it is in the schema.
struct Builder {
    private(set) var rules: [(name: String, body: String)] = []
    private var bodies: [String: String] = [:]
    private var names: Set<String> = []

    /// Keywords that describe a value without changing what it may be.
    private static let notes: Set<String> = ["description", "title", "$schema", "examples", "default"]

    var text: String {
        rules.map { "\($0.name) ::= \($0.body)" }.joined(separator: "\n") + "\n"
    }

    /// Adds `name ::= body`, and the space between tokens that every
    /// rule may use.
    mutating func define(_ name: String, _ body: String) {
        rules.append((name, body))
        names.insert(name)
        _ = primitive(.ws)
    }

    /// The name of a rule for `node`, written as `name` or a name like it.
    /// A primitive's rule is shared.
    mutating func rule(for node: OrderedJSON, named name: String) throws(SchemaGrammarError) -> String {
        let body = try expression(for: node, named: name)
        // A rule that's only another rule is that rule.
        if names.contains(body) { return body }
        if let shared = bodies[body] { return shared }
        let unique = unique(name)
        define(unique, body)
        bodies[body] = unique
        return unique
    }

    private mutating func expression(for node: OrderedJSON, named name: String) throws(SchemaGrammarError)
        -> String
    {
        // `true`, or a schema with no constraints, is any JSON.
        if case .bool(true) = node { return primitive(.value) }
        guard case .object(let keywords) = node else { throw .invalid(keyword: name) }
        let schema = Dictionary(keywords, uniquingKeysWith: { first, _ in first })
        for key in schema.keys where !Self.notes.contains(key) && !Self.known.contains(key) {
            throw .unsupported(keyword: key)
        }
        if let constant = schema["const"] { return Self.literal(constant.text) }
        if let choices = schema["enum"] {
            guard case .array(let values) = choices, !values.isEmpty else { throw .invalid(keyword: "enum") }
            return values.map { Self.literal($0.text) }.joined(separator: " | ")
        }
        if let options = schema["anyOf"] ?? schema["oneOf"] {
            guard case .array(let schemas) = options, !schemas.isEmpty else { throw .invalid(keyword: "anyOf") }
            var alternatives: [String] = []
            for (index, option) in schemas.enumerated() {
                alternatives.append(try rule(for: option, named: "\(name)-\(index)"))
            }
            return alternatives.joined(separator: " | ")
        }
        let types: [String]
        switch schema["type"] {
        case .none:
            types = schema["properties"] != nil ? ["object"] : schema["items"] != nil ? ["array"] : []
        case .string(let type)?: types = [type]
        case .array(let list)?:
            types = try list.map { (type: OrderedJSON) throws(SchemaGrammarError) -> String in
                guard case .string(let name) = type else { throw .invalid(keyword: "type") }
                return name
            }
        case .some: throw .invalid(keyword: "type")
        }
        if types.isEmpty { return primitive(.value) }
        var alternatives: [String] = []
        for type in types {
            switch type {
            case "object": alternatives.append(try object(schema, named: name))
            case "array": alternatives.append(try array(schema, named: name))
            case "string": alternatives.append(primitive(.string))
            case "boolean": alternatives.append(primitive(.boolean))
            case "number": alternatives.append(primitive(.number))
            case "integer": alternatives.append(primitive(.integer))
            case "null": alternatives.append(primitive(.null))
            default: throw .invalid(keyword: "type")
            }
        }
        return alternatives.count == 1 ? alternatives[0] : alternatives.map { "(\($0))" }.joined(separator: " | ")
    }

    private static let known: Set<String> = [
        "type", "properties", "required", "additionalProperties", "items", "enum", "const", "anyOf", "oneOf",
    ]

    /// An object with its properties in order. A property that's left out
    /// takes its comma with it: `start(i)` is what follows "{" from the
    /// i-th property on, and `rest(i)` what follows a property already
    /// written.
    private mutating func object(_ schema: [String: OrderedJSON], named name: String) throws(SchemaGrammarError)
        -> String
    {
        var properties: [(name: String, value: OrderedJSON)] = []
        switch schema["properties"] {
        case .none: break
        case .object(let fields)?: properties = fields
        case .some: throw .invalid(keyword: "properties")
        }
        var required: Set<String> = []
        switch schema["required"] {
        case .none: break
        case .array(let names)?:
            for item in names {
                guard case .string(let name) = item else { throw .invalid(keyword: "required") }
                required.insert(name)
            }
        case .some: throw .invalid(keyword: "required")
        }
        switch schema["additionalProperties"] {
        case .none, .bool(false)?: break
        // An object that lists nothing may hold anything.
        case .bool(true)? where properties.isEmpty: return primitive(.object)
        case .bool(true)?: throw .unsupported(keyword: "additionalProperties")
        case .object? where properties.isEmpty: return primitive(.object)
        case .some: throw .unsupported(keyword: "additionalProperties")
        }
        if properties.isEmpty, schema["properties"] == nil { return primitive(.object) }
        if properties.isEmpty { return #""{" ws "}""# }

        var pairs: [String] = []
        for (index, property) in properties.enumerated() {
            let value = try rule(for: property.value, named: "\(name)-\(Self.ruleName(property.name, index))")
            pairs.append(Self.literal(OrderedJSON.string(property.name).text) + #" ws ":" ws "# + value)
        }
        let isRequired = properties.map { required.contains($0.name) }
        let count = properties.count
        // The object starts at its first property, or at the next each
        // time an optional one is left out. Whichever it starts at, the
        // rest follow it.
        var starts = [0]
        while let last = starts.last, last + 1 < count, !isRequired[last] { starts.append(last + 1) }
        var rest: [Int: String] = [:]
        for index in (1..<max(count, 1)).reversed() {
            let pair = #"ws "," ws "# + pairs[index]
            let next = rest[index + 1].map { " " + $0 } ?? ""
            rest[index] = rule(unique: "\(name)-rest-\(index)", (isRequired[index] ? pair : "(\(pair))?") + next)
        }
        var start: [Int: String] = [:]
        for index in starts.reversed() {
            let first = pairs[index] + (rest[index + 1].map { " " + $0 } ?? "")
            // Leaving this one out starts with the next, if any is left.
            let skip = isRequired[index] ? nil : start[index + 1]
            start[index] = rule(unique: "\(name)-start-\(index)", skip.map { "\(first) | \($0)" } ?? first)
        }
        let body = start[0] ?? ""
        let optional = isRequired.allSatisfy { !$0 }
        return optional ? #""{" ws ("# + body + #" ws)? "}""# : #""{" ws "# + body + #" ws "}""#
    }

    private mutating func array(_ schema: [String: OrderedJSON], named name: String) throws(SchemaGrammarError)
        -> String
    {
        let item =
            try schema["items"].map { (items: OrderedJSON) throws(SchemaGrammarError) -> String in
                try rule(for: items, named: "\(name)-item")
            } ?? primitive(.value)
        return #""[" ws ("# + item + #" (ws "," ws "# + item + #")* ws)? "]""#
    }

    /// A rule that isn't shared, such as one step through an object.
    private mutating func rule(unique name: String, _ body: String) -> String {
        let unique = unique(name)
        define(unique, body)
        return unique
    }

    private func unique(_ name: String) -> String {
        var candidate = name
        var number = 1
        while names.contains(candidate) {
            number += 1
            candidate = "\(name)\(number)"
        }
        return candidate
    }

    /// A property's name as it can appear in a rule's: letters, digits
    /// and dashes.
    private static func ruleName(_ name: String, _ index: Int) -> String {
        let kept = name.unicodeScalars.map { scalar in
            ("a"..."z").contains(scalar) || ("A"..."Z").contains(scalar) || ("0"..."9").contains(scalar)
                ? Character(scalar) : "-"
        }
        let cleaned = String(kept)
        return cleaned.allSatisfy({ $0 == "-" }) ? "p\(index)" : cleaned
    }

    /// `text`, written out as GBNF's quoted literal.
    static func literal(_ text: String) -> String {
        var quoted = "\""
        for scalar in text.unicodeScalars {
            switch scalar {
            case "\"": quoted += #"\""#
            case "\\": quoted += #"\\"#
            case "\n": quoted += #"\n"#
            case "\r": quoted += #"\r"#
            case "\t": quoted += #"\t"#
            case _ where scalar.value < 0x20 || scalar.value == 0x7F:
                let hex = String(scalar.value, radix: 16)
                quoted += #"\x"# + (hex.count == 1 ? "0" : "") + hex
            default: quoted.unicodeScalars.append(scalar)
            }
        }
        return quoted + "\""
    }

    // MARK: Primitives

    private enum Primitive: String, CaseIterable {
        case ws, string, char, number, integer, boolean, null, value, object, array
    }

    /// The rule for `primitive`, added with those it uses the first time.
    private mutating func primitive(_ primitive: Primitive) -> String {
        let name = primitive.rawValue
        guard !names.contains(name) else { return name }
        names.insert(name)
        rules.append((name, Self.body(of: primitive)))
        switch primitive {
        case .string: _ = self.primitive(.char)
        case .value:
            for each in [Primitive.object, .array, .string, .number, .boolean, .null] { _ = self.primitive(each) }
        case .object, .array: _ = self.primitive(.value)
        case .ws, .char, .number, .integer, .boolean, .null: break
        }
        _ = self.primitive(.ws)
        return name
    }

    private static func body(of primitive: Primitive) -> String {
        switch primitive {
        // A little room for a newline and an indent, and no more, so a
        // model can't fill its window with spaces.
        case .ws: #"| " " | "\n" [ \t]{0,20}"#
        case .string: #""\"" char* "\"""#
        case .char: #"[^"\\\x7F\x00-\x1F] | "\\" (["\\/bfnrt] | "u" [0-9a-fA-F]{4})"#
        case .number: #""-"? ("0" | [1-9] [0-9]{0,15}) ("." [0-9]+)? ([eE] [-+]? [0-9]+)?"#
        case .integer: #""-"? ("0" | [1-9] [0-9]{0,15})"#
        case .boolean: #""true" | "false""#
        case .null: #""null""#
        case .value: "object | array | string | number | boolean | null"
        case .object: #""{" ws (string ws ":" ws value (ws "," ws string ws ":" ws value)* ws)? "}""#
        case .array: #""[" ws (value (ws "," ws value)* ws)? "]""#
        }
    }
}
