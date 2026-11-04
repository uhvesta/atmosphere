import Foundation

/// An Avenue's short key, such as HEALTH. Task codes show it as a prefix.
///
/// A person picks 2 to 10 capital letters. Reconciliation may add a numeric
/// suffix to resolve a duplicate, and the app then asks for a new key.
package struct AvenueKey: Hashable, Comparable, Codable, Sendable, CustomStringConvertible {
    package let rawValue: String

    /// A key a person typed. Lowercase letters are capitalized.
    package init(_ string: String) throws(LifrError) {
        let key = string.uppercased()
        guard (2...10).contains(key.count), key.allSatisfy(\.isKeyLetter) else {
            throw .invalidAvenueKey(string)
        }
        rawValue = key
    }

    package init(reconciled rawValue: String) {
        self.rawValue = rawValue
    }

    /// The key got a numeric suffix during reconciliation and should be renamed.
    package var needsRename: Bool { rawValue.last?.isKeyDigit ?? false }

    /// The key without a reconciliation suffix.
    var letters: String { String(rawValue.prefix(while: \.isKeyLetter)) }

    package var description: String { rawValue }

    package static func < (lhs: Self, rhs: Self) -> Bool { lhs.rawValue < rhs.rawValue }

    package init(from decoder: any Decoder) throws {
        let string = try decoder.singleValueContainer().decode(String.self)
        let letters = string.prefix(while: \.isKeyLetter)
        guard (2...10).contains(string.count), !letters.isEmpty,
            string.dropFirst(letters.count).allSatisfy(\.isKeyDigit)
        else {
            throw DecodingError.dataCorrupted(
                .init(
                    codingPath: decoder.codingPath,
                    debugDescription: LifrError.invalidAvenueKey(string).description
                ))
        }
        rawValue = string
    }

    package func encode(to encoder: any Encoder) throws {
        var container = encoder.singleValueContainer()
        try container.encode(rawValue)
    }
}

/// A task's short code: 4 random Crockford base32 characters, shown with the
/// Avenue key, such as HEALTH-7K2Q. Lookups ignore the key, so moving a task
/// to another Avenue doesn't break references to it.
package struct TaskCode: Hashable, Comparable, Codable, Sendable, CustomStringConvertible {
    static let alphabet = Array("0123456789ABCDEFGHJKMNPQRSTVWXYZ")
    static let length = 4

    package let rawValue: String

    /// Parses `7K2Q` or `HEALTH-7K2Q`, in any case. As in Crockford base32,
    /// I and L read as 1 and O as 0.
    package init(_ string: String) throws(LifrError) {
        let code = string.split(separator: "-", omittingEmptySubsequences: false).last ?? ""
        var normalized = ""
        for character in code.uppercased() {
            switch character {
            case "I", "L": normalized.append("1")
            case "O": normalized.append("0")
            case _ where Self.alphabet.contains(character): normalized.append(character)
            default: throw .invalidTaskCode(string)
            }
        }
        guard normalized.count == Self.length else { throw .invalidTaskCode(string) }
        rawValue = normalized
    }

    init(digits: [Int]) {
        rawValue = String(digits.map { Self.alphabet[$0] })
    }

    package static func random(using generator: inout some RandomNumberGenerator) -> TaskCode {
        TaskCode(digits: (0..<length).map { _ in Int.random(in: 0..<alphabet.count, using: &generator) })
    }

    /// The same code for the same task and attempt on every device, so two
    /// devices reconciling a duplicate pick the same replacement.
    static func derived(from id: UUID, attempt: Int) -> TaskCode {
        var hash: UInt64 = 0xcbf2_9ce4_8422_2325
        withUnsafeBytes(of: id.uuid) { bytes in
            for byte in bytes { hash = (hash ^ UInt64(byte)) &* 0x100_0000_01b3 }
        }
        for byte in withUnsafeBytes(of: UInt64(attempt).littleEndian, Array.init) {
            hash = (hash ^ UInt64(byte)) &* 0x100_0000_01b3
        }
        return TaskCode(
            digits: (0..<length).map { index in
                Int((hash >> (UInt64(index) * 5)) & 31)
            })
    }

    /// The code as people see it, such as HEALTH-7K2Q.
    func display(in key: AvenueKey) -> String { "\(key)-\(rawValue)" }

    package var description: String { rawValue }

    package static func < (lhs: Self, rhs: Self) -> Bool { lhs.rawValue < rhs.rawValue }

    package init(from decoder: any Decoder) throws {
        let string = try decoder.singleValueContainer().decode(String.self)
        do {
            try self.init(string)
        } catch {
            throw DecodingError.dataCorrupted(
                .init(
                    codingPath: decoder.codingPath,
                    debugDescription: error.description,
                    underlyingError: error
                ))
        }
    }

    package func encode(to encoder: any Encoder) throws {
        var container = encoder.singleValueContainer()
        try container.encode(rawValue)
    }
}

extension Character {
    var isKeyLetter: Bool { ("A"..."Z").contains(self) }
    var isKeyDigit: Bool { ("0"..."9").contains(self) }
}

extension TaskCode {
    /// 0001 for 1: the number's last four decimal digits. For fixed codes in
    /// samples and tests.
    package init(sequence number: Int) {
        self.init(digits: decimalDigits(number, count: Self.length))
    }
}

extension UUID {
    /// 00000000-0000-0000-0000-000000000101 for 101: the number's last 12
    /// decimal digits as the UUID's last 12 hex digits. For fixed IDs in
    /// samples and tests.
    package init(sequence number: Int) {
        let digits = decimalDigits(number, count: 12)
        var bytes = [UInt8](repeating: 0, count: 16)
        for index in 0..<6 {
            bytes[10 + index] = UInt8(digits[2 * index] * 16 + digits[2 * index + 1])
        }
        self.init(
            uuid: (
                bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7], bytes[8], bytes[9],
                bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
            ))
    }
}

/// The last `count` decimal digits of `number`, most significant first.
private func decimalDigits(_ number: Int, count: Int) -> [Int] {
    var rest = abs(number)
    var digits = [Int](repeating: 0, count: count)
    for place in (0..<count).reversed() {
        digits[place] = rest % 10
        rest /= 10
    }
    return digits
}
