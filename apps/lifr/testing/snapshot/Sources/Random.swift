/// The same numbers on every run, so what's made at random, such as a new
/// task's key, matches its reference. SplitMix64.
package struct SeededRandomNumberGenerator: RandomNumberGenerator {
    private var state: UInt64

    package init(seed: UInt64 = 0) {
        state = seed
    }

    package mutating func next() -> UInt64 {
        state &+= 0x9E37_79B9_7F4A_7C15
        var mixed = state
        mixed = (mixed ^ (mixed >> 30)) &* 0xBF58_476D_1CE4_E5B9
        mixed = (mixed ^ (mixed >> 27)) &* 0x94D0_49BB_1331_11EB
        return mixed ^ (mixed >> 31)
    }
}
