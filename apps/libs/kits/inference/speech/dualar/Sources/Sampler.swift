import Foundation

/// How a token is picked from a model's scores.
struct Sampling: Hashable, Sendable {
    /// How flat the distribution is: higher is more varied.
    var temperature: Float = 0.8
    /// The share of the probability the candidates keep, from the likeliest.
    var topP: Float = 0.95
    /// The most candidates kept.
    var topK: Int = 50
    /// Always take the likeliest, ignoring the rest.
    var greedy = false
}

/// Picks tokens from scores the way the model was tuned to: the likeliest
/// `topK`, cut to the first `topP` of the probability, scaled by the
/// temperature, then drawn from at random.
struct Sampler<Random: RandomNumberGenerator> {
    var random: Random

    /// The index of the picked score.
    mutating func pick(_ scores: [Float], _ sampling: Sampling) -> Int {
        let kept = Self.candidates(scores, topK: sampling.topK, topP: sampling.topP)
        guard !sampling.greedy, kept.count > 1 else { return kept.first ?? 0 }
        let temperature = max(sampling.temperature, 1e-5)
        let top = scores[kept[0]]
        let weights = kept.map { exp((scores[$0] - top) / temperature) }
        var target = Float.random(in: 0..<1, using: &random) * weights.reduce(0, +)
        for (index, weight) in zip(kept, weights) {
            target -= weight
            if target < 0 { return index }
        }
        return kept[kept.count - 1]
    }

    /// The indices that stay candidates, likeliest first: never more than
    /// `topK`, and only as many as reach `topP` of the probability between
    /// them, but always the likeliest.
    static func candidates(_ scores: [Float], topK: Int, topP: Float) -> [Int] {
        let order = scores.indices.sorted { scores[$0] > scores[$1] || (scores[$0] == scores[$1] && $0 < $1) }
        guard let first = order.first else { return [] }
        let top = scores[first]
        let total = scores.reduce(Float(0)) { $0 + exp($1 - top) }
        var kept: [Int] = []
        var cumulative: Float = 0
        for (position, index) in order.enumerated() {
            cumulative += exp(scores[index] - top) / total
            if position > 0, position >= topK || cumulative > topP { break }
            kept.append(index)
        }
        return kept
    }
}

/// A fast generator that repeats from a seed, for tests and repeatable runs.
struct SplitMix64: RandomNumberGenerator {
    private var state: UInt64

    init(seed: UInt64) {
        state = seed
    }

    mutating func next() -> UInt64 {
        state &+= 0x9E37_79B9_7F4A_7C15
        var value = state
        value = (value ^ (value >> 30)) &* 0xBF58_476D_1CE4_E5B9
        value = (value ^ (value >> 27)) &* 0x94D0_49BB_1331_11EB
        return value ^ (value >> 31)
    }
}
