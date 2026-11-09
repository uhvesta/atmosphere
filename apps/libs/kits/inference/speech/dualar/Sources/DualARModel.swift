import Foundation
import MLX

/// The model's sizes, from its config.json.
struct DualARConfig: Decodable, Hashable, Sendable {
    var dim: Int
    var nLayer: Int
    var nHead: Int
    var nLocalHeads: Int
    var headDim: Int
    var nFastLayer: Int
    var fastNHead: Int
    var fastNLocalHeads: Int
    var fastHeadDim: Int
    var normEps: Float
    var ropeBase: Float
    var maxSeqLen: Int
    var numCodebooks: Int
    var codebookSize: Int
    var semanticBeginId: Int
    var semanticEndId: Int
    var eosTokenId: Int
    var rasTopP: Float
    var rasTemperature: Float
    var rasWindowSize: Int
    var codecSampleRate: Int
    var codecFrameSize: Int
    var codecPostNLayer: Int
    var codecPostNHead: Int
    var codecPostNLocalHeads: Int

    var semanticCount: Int { semanticEndId - semanticBeginId + 1 }
}

/// A dual autoregressive speech model: a slow transformer reads the text
/// and, frame by frame, picks each frame's semantic token, and a fast one
/// picks the frame's other codebooks from the slow one's state. The frames
/// are the codec's codes.
final class DualARModel {
    let config: DualARConfig
    private let embeddings: MLXArray
    private let codebookEmbeddings: MLXArray
    private let layers: [TransformerLayer]
    private let norm: MLXArray
    /// The rows of the output the filter lets through: the semantic tokens,
    /// then the end.
    private let speechRows: MLXArray
    private let fastEmbeddings: MLXArray
    private let fastLayers: [TransformerLayer]
    private let fastNorm: MLXArray
    private let fastOutput: MLXArray
    private let rotary: Rotary
    private let fastRotary: Rotary

    init(config: DualARConfig, weights: Weights) throws(Weights.Missing) {
        self.config = config
        embeddings = try weights["embeddings.weight"]
        codebookEmbeddings = try weights["codebook_embeddings.weight"]
        let shape = TransformerLayer.Shape(
            heads: config.nHead, keyHeads: config.nLocalHeads, headSize: config.headDim, epsilon: config.normEps)
        layers = try (0..<config.nLayer).map { index throws(Weights.Missing) in
            try TransformerLayer("layers.\(index)", weights, shape: shape)
        }
        norm = try weights["norm.weight"]
        speechRows = concatenated(
            [
                embeddings[config.semanticBeginId...config.semanticEndId],
                embeddings[config.eosTokenId...config.eosTokenId],
            ], axis: 0)
        fastEmbeddings = try weights["fast_embeddings.weight"]
        let fastShape = TransformerLayer.Shape(
            heads: config.fastNHead, keyHeads: config.fastNLocalHeads, headSize: config.fastHeadDim,
            epsilon: config.normEps)
        fastLayers = try (0..<config.nFastLayer).map { index throws(Weights.Missing) in
            try TransformerLayer("fast_layers.\(index)", weights, shape: fastShape)
        }
        fastNorm = try weights["fast_norm.weight"]
        fastOutput = try weights["fast_output.weight"]
        rotary = Rotary(length: config.maxSeqLen, headSize: config.headDim, base: config.ropeBase)
        fastRotary = Rotary(length: config.numCodebooks, headSize: config.fastHeadDim, base: config.ropeBase)
    }

    /// The codes for the prompt `tokens`: `numCodebooks` rows, a column a
    /// frame, until the model ends the speech or `maxFrames` frames.
    /// `stop` is asked before each frame.
    func generate(
        _ tokens: [Int], sampling: Sampling, maxFrames: Int, seed: UInt64? = nil, stop: () -> Bool = { false }
    ) -> [[Int32]] {
        var sampler = Sampler(random: seed.map(SplitMix64.init(seed:)) ?? SplitMix64(seed: .random(in: .min ... .max)))
        let caches = layers.map { _ in LayerCache() }
        let frames = min(maxFrames, config.maxSeqLen - tokens.count)
        var (scores, hidden) = slowStep(
            embeddings[MLXArray(tokens.map(Int32.init))].expandedDimensions(axis: 0), offset: 0, caches: caches)
        var recent: [Int]?
        var codes = [[Int32]](repeating: [], count: config.numCodebooks)
        for _ in 0..<max(frames, 0) {
            guard !stop() else { break }
            let semantic = pickSemantic(scores, sampling: sampling, recent: recent, sampler: &sampler)
            if recent == nil {
                recent = []
            } else {
                recent?.append(semantic)
                if let count = recent?.count, count > config.rasWindowSize { recent?.removeFirst() }
            }
            guard semantic != config.eosTokenId else { break }
            let frame = codebooks(hidden, semantic: semantic, sampling: sampling, sampler: &sampler)
            for (row, code) in frame.enumerated() { codes[row].append(Int32(code)) }
            (scores, hidden) = slowStep(
                embed(semantic: semantic, codebooks: frame), offset: caches[0].offset, caches: caches)
        }
        return codes
    }

    /// The semantic token, or the end, from the scores of the speech rows,
    /// picked again with the wider settings when the pick repeats one of
    /// the last few.
    private func pickSemantic(
        _ scores: [Float], sampling: Sampling, recent: [Int]?, sampler: inout Sampler<SplitMix64>
    ) -> Int {
        let token = { (index: Int) in
            index < self.config.semanticCount ? self.config.semanticBeginId + index : self.config.eosTokenId
        }
        let normal = token(sampler.pick(scores, sampling))
        guard !sampling.greedy else { return normal }
        var wide = sampling
        wide.topP = config.rasTopP
        wide.temperature = config.rasTemperature
        let high = token(sampler.pick(scores, wide))
        guard let recent, recent.contains(normal), normal != config.eosTokenId else { return normal }
        return high
    }

    /// Runs the slow transformer on `input`, [1, length, dim], and gives
    /// the speech rows' scores at the last position, and its normed state.
    private func slowStep(_ input: MLXArray, offset: Int, caches: [LayerCache]) -> ([Float], MLXArray) {
        var hidden = input
        let mask: MLXFast.ScaledDotProductAttentionMaskMode = input.dim(1) > 1 ? .causal : .none
        for (layer, cache) in zip(layers, caches) {
            hidden = layer(hidden, rotary: rotary, offset: offset, mask: mask, cache: cache)
        }
        let last = rmsNorm(hidden[0..., (hidden.dim(1) - 1)...], weight: norm, eps: config.normEps)
        let scores = matmul(last, speechRows.T).reshaped(-1).asType(.float32)
        eval(scores, last)
        return (scores.asArray(Float.self), last)
    }

    /// The frame's codebooks: the first is the semantic token's, the rest
    /// the fast transformer picks one by one, starting from `hidden`.
    private func codebooks(
        _ hidden: MLXArray, semantic: Int, sampling: Sampling, sampler: inout Sampler<SplitMix64>
    ) -> [Int] {
        let caches = fastLayers.map { _ in LayerCache() }
        _ = fastStep(hidden, position: 0, caches: caches)
        var codes = [min(max(semantic - config.semanticBeginId, 0), config.codebookSize - 1)]
        for position in 1..<config.numCodebooks {
            let previous = codes[codes.count - 1]
            let input = fastEmbeddings[previous..<(previous + 1)].expandedDimensions(axis: 0)
            let scores = fastStep(input, position: position, caches: caches)
            eval(scores)
            codes.append(sampler.pick(scores.asArray(Float.self), sampling))
        }
        return codes
    }

    private func fastStep(_ input: MLXArray, position: Int, caches: [LayerCache]) -> MLXArray {
        var hidden = input
        for (layer, cache) in zip(fastLayers, caches) {
            hidden = layer(hidden, rotary: fastRotary, offset: position, mask: .none, cache: cache)
        }
        return matmul(rmsNorm(hidden, weight: fastNorm, eps: config.normEps), fastOutput.T).reshaped(-1)
            .asType(.float32)
    }

    /// A generated frame as the slow transformer reads it back: the
    /// semantic token's embedding plus each codebook's.
    private func embed(semantic: Int, codebooks: [Int]) -> MLXArray {
        let rows = codebooks.enumerated().map { Int32($0.element + $0.offset * config.codebookSize) }
        let sum = codebookEmbeddings[MLXArray(rows)].sum(axis: 0)
        return (embeddings[semantic..<(semantic + 1)] + sum).expandedDimensions(axis: 0)
    }
}
