import Foundation

/// Any JSON value: request params and results on the wire, and JSON schemas.
package enum JSONValue: Hashable, Sendable, Codable {
    case null
    case bool(Bool)
    case number(Double)
    case string(String)
    case array([JSONValue])
    case object([String: JSONValue])

    package init(from decoder: any Decoder) throws {
        let container = try decoder.singleValueContainer()
        if container.decodeNil() {
            self = .null
        } else if let value = try? container.decode(Bool.self) {
            self = .bool(value)
        } else if let value = try? container.decode(Double.self) {
            self = .number(value)
        } else if let value = try? container.decode(String.self) {
            self = .string(value)
        } else if let value = try? container.decode([JSONValue].self) {
            self = .array(value)
        } else {
            self = .object(try container.decode([String: JSONValue].self))
        }
    }

    package func encode(to encoder: any Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case .null: try container.encodeNil()
        case let .bool(value): try container.encode(value)
        case let .number(value):
            if value.rounded() == value, abs(value) < 1e15 {
                try container.encode(Int64(value))
            } else {
                try container.encode(value)
            }
        case let .string(value): try container.encode(value)
        case let .array(value): try container.encode(value)
        case let .object(value): try container.encode(value)
        }
    }

    /// Encodes a value the way the wire does.
    package init(_ value: some Encodable) throws {
        self = try Wire.decoder.decode(JSONValue.self, from: Wire.encoder.encode(value))
    }

    /// Decodes this value as `type`.
    package func decode<T: Decodable>(_ type: T.Type) throws -> T {
        try Wire.decoder.decode(type, from: Wire.encoder.encode(self))
    }
}

extension JSONValue: ExpressibleByStringLiteral, ExpressibleByBooleanLiteral, ExpressibleByIntegerLiteral,
    ExpressibleByArrayLiteral, ExpressibleByDictionaryLiteral
{
    package init(stringLiteral value: String) { self = .string(value) }
    package init(booleanLiteral value: Bool) { self = .bool(value) }
    package init(integerLiteral value: Int) { self = .number(Double(value)) }
    package init(arrayLiteral elements: JSONValue...) { self = .array(elements) }
    package init(dictionaryLiteral elements: (String, JSONValue)...) {
        self = .object(Dictionary(elements, uniquingKeysWith: { $1 }))
    }
}

/// JSON as Lifr writes it: sorted keys, ISO 8601 dates, one message per line.
package enum Wire {
    package static var encoder: JSONEncoder {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
        encoder.dateEncodingStrategy = .iso8601
        return encoder
    }

    package static var decoder: JSONDecoder {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        return decoder
    }

    /// One message as a line of JSON ending in a newline. JSON escapes
    /// newlines inside strings, so a message never spans lines.
    package static func line(_ value: some Encodable) throws -> Data {
        var data = try encoder.encode(value)
        data.append(UInt8(ascii: "\n"))
        return data
    }
}
