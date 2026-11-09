/// Persian written the way Persian voice models are trained on it: one
/// form of each letter, no short vowels or joiners, and numbers in words.
enum PersianText {
    static func normalized(_ text: String) -> String {
        var scalars: [Unicode.Scalar] = []
        for scalar in text.precomposedStringWithCanonicalMapping.unicodeScalars {
            if scalar == "\u{066A}" {
                scalars.append("%")  // the Arabic percent sign, which sits among the marks below
            } else if let replacement = letters[scalar] {
                scalars.append(replacement)
            } else if let digit = digit(scalar) {
                scalars.append(digit)
            } else if !isDropped(scalar) {
                scalars.append(scalar)
            }
        }
        scalars = withoutThousandsCommas(scalars)
        var spoken: [Unicode.Scalar] = []
        var index = 0
        while index < scalars.count {
            let scalar = scalars[index]
            if scalar == "%" {
                spoken += " درصد ".unicodeScalars
                index += 1
            } else if isDigit(scalar) {
                let end = numberEnd(in: scalars, from: index)
                spoken += " \(spokenNumber(String(String.UnicodeScalarView(scalars[index..<end])))) ".unicodeScalars
                index = end
            } else {
                spoken.append(scalar)
                index += 1
            }
        }
        return SpokenText.collapsedSpaces(String(scalars: withoutSpacesBeforeMarks(spoken)))
    }

    /// `number`, digits with at most one "." or "," in them, in words: a
    /// "," before three digits groups thousands, another one is a list's
    /// comma, and a "." is a decimal point.
    static func spokenNumber(_ number: String) -> String {
        if let comma = number.firstIndex(of: ",") {
            let whole = String(number[..<comma])
            let rest = String(number[number.index(after: comma)...])
            if rest.count == 3 { return words(whole + rest) }
            return "\(words(whole)) \u{060C} \(words(rest))"
        }
        if let point = number.firstIndex(of: ".") {
            return "\(words(String(number[..<point]))) ممیز \(words(String(number[number.index(after: point)...])))"
        }
        return words(number)
    }

    /// `digits` as a whole number in words, "هزار" rather than "یک هزار"
    /// for a thousand; digits too long to name are left as they are.
    static func words(_ digits: String) -> String {
        let trimmed = digits.drop { $0 == "0" }
        guard !trimmed.isEmpty else { return "صفر" }
        guard trimmed.count <= scales.count * 3 else { return digits }
        let values = trimmed.compactMap(\.wholeNumberValue)
        var groups: [Int] = []
        var end = values.count
        while end > 0 {
            let start = max(end - 3, 0)
            groups.insert(values[start..<end].reduce(0) { $0 * 10 + $1 }, at: 0)
            end = start
        }
        let named = groups.enumerated().compactMap { offset, group -> String? in
            guard group > 0 else { return nil }
            let scale = scales[groups.count - 1 - offset]
            return scale.isEmpty ? belowThousand(group) : "\(belowThousand(group)) \(scale)"
        }
        let text = named.joined(separator: " و ")
        return text.hasPrefix("یک هزار") ? String(text.dropFirst("یک ".count)) : text
    }

    private static func belowThousand(_ number: Int) -> String {
        let hundred = number / 100
        let ten = number % 100 / 10
        let one = number % 10
        var parts = hundred > 0 ? [hundreds[hundred]] : []
        if ten == 1 {
            parts.append(teens[one])
        } else {
            if ten > 1 { parts.append(tens[ten]) }
            if one > 0 { parts.append(ones[one]) }
        }
        return parts.joined(separator: " و ")
    }

    /// Where the number starting at `start` ends: its digits, then a "."
    /// or "," and more digits, if they follow.
    private static func numberEnd(in text: [Unicode.Scalar], from start: Int) -> Int {
        var end = start
        while end < text.count, isDigit(text[end]) { end += 1 }
        if end + 1 < text.count, text[end] == "." || text[end] == ",", isDigit(text[end + 1]) {
            end += 1
            while end < text.count, isDigit(text[end]) { end += 1 }
        }
        return end
    }

    /// `text` without the "," between a digit and a last group of three,
    /// as in 1,234, so the number is read whole.
    private static func withoutThousandsCommas(_ text: [Unicode.Scalar]) -> [Unicode.Scalar] {
        text.indices.compactMap { index in
            let scalar = text[index]
            guard scalar == ",", index > 0, isDigit(text[index - 1]), index + 3 < text.count + 0,
                text[(index + 1)...(index + 3)].allSatisfy(isDigit),
                index + 4 == text.count || !isWordCharacter(text[index + 4])
            else { return scalar }
            return nil
        }
    }

    private static func withoutSpacesBeforeMarks(_ text: [Unicode.Scalar]) -> [Unicode.Scalar] {
        var kept: [Unicode.Scalar] = []
        for scalar in text {
            if marks.contains(scalar) {
                while let last = kept.last, SpokenText.isSpace(last) { kept.removeLast() }
            }
            kept.append(scalar)
        }
        return kept
    }

    private static func isDigit(_ scalar: Unicode.Scalar) -> Bool {
        ("0"..."9").contains(scalar)
    }

    private static func isWordCharacter(_ scalar: Unicode.Scalar) -> Bool {
        scalar == "_" || scalar.properties.isAlphabetic || scalar.properties.numericType != nil
    }

    private static func digit(_ scalar: Unicode.Scalar) -> Unicode.Scalar? {
        let value = scalar.value
        let zero: UInt32
        switch value {
        case 0x06F0...0x06F9: zero = 0x06F0
        case 0x0660...0x0669: zero = 0x0660
        default: return nil
        }
        return Unicode.Scalar(0x30 + value - zero)
    }

    private static func isDropped(_ scalar: Unicode.Scalar) -> Bool {
        switch scalar.value {
        case 0x064B...0x0670, 0x06D6...0x06ED: true  // short vowels and Quranic marks
        case 0x200B...0x200F, 0x202A...0x202E, 0x2066...0x2069, 0xFEFF: true  // joiners and direction marks
        case 0x0640: true  // tatweel
        default: false
        }
    }

    private static let letters: [Unicode.Scalar: Unicode.Scalar] = [
        "\u{064A}": "\u{06CC}", "\u{0649}": "\u{06CC}", "\u{0643}": "\u{06A9}", "\u{0629}": "\u{0647}",
        "\u{0623}": "\u{0627}", "\u{0625}": "\u{0627}", "\u{0671}": "\u{0627}",
        "\u{00AB}": "\"", "\u{00BB}": "\"", "\u{066C}": ",", "\u{066B}": ".", "\u{2013}": "-", "\u{2014}": "-",
    ]
    private static let marks: Set<Unicode.Scalar> = ["\u{060C}", ",", ".", "!", "?", "\u{061F}", ":", ";", "\u{061B}"]
    private static let ones = ["", "یک", "دو", "سه", "چهار", "پنج", "شش", "هفت", "هشت", "نه"]
    private static let teens = [
        "ده", "یازده", "دوازده", "سیزده", "چهارده", "پانزده", "شانزده", "هفده", "هجده", "نوزده",
    ]
    private static let tens = ["", "", "بیست", "سی", "چهل", "پنجاه", "شصت", "هفتاد", "هشتاد", "نود"]
    private static let hundreds = ["", "یکصد", "دویست", "سیصد", "چهارصد", "پانصد", "ششصد", "هفتصد", "هشتصد", "نهصد"]
    private static let scales = ["", "هزار", "میلیون", "میلیارد", "بیلیون", "بیلیارد", "تریلیون", "ترلیارد"]
}
