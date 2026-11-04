/// Fractional indexes for `sortOrder`: a key between two neighbours, so a
/// reorder rewrites only the record that moved.
package enum SortKey {
    static let digits = Array("0123456789abcdefghijklmnopqrstuvwxyz")

    /// A key that sorts after `lower` and before `upper`. `nil` means no bound.
    /// Both bounds must be keys this function made, with `lower < upper`.
    package static func between(_ lower: String?, _ upper: String?) -> String {
        let lower = Array(lower ?? "")
        var upper = upper.map(Array.init)
        var key = ""
        var index = 0
        while true {
            let low = index < lower.count ? value(lower[index]) : 0
            let high = upper.map { index < $0.count ? value($0[index]) : 0 } ?? digits.count
            if high - low > 1 {
                key.append(digits[(low + high) / 2])
                return key
            }
            key.append(digits[low])
            if low < high { upper = nil }
            index += 1
        }
    }

    static func value(_ digit: Character) -> Int {
        digits.firstIndex(of: digit) ?? 0
    }
}
