import Foundation
import MLX

/// The rotations a transformer turns each pair of a head's values by,
/// for positions from 0, rounded to bfloat16 as the model was trained with.
struct Rotary {
    let cosines: MLXArray
    let sines: MLXArray

    init(length: Int, headSize: Int, base: Float) {
        let frequencies = MLXArray(
            stride(from: 0, to: headSize, by: 2).map { 1 / pow(base, Float($0) / Float(headSize)) })
        let phases = outer(MLXArray(0..<Int32(length)).asType(.float32), frequencies)
        cosines = cos(phases).asType(.bfloat16).asType(.float32)
        sines = sin(phases).asType(.bfloat16).asType(.float32)
    }

    /// `xs`, [batch, length, heads, size], turned for `length` positions
    /// from `offset`, adjacent values making each pair.
    func callAsFunction(_ xs: MLXArray, offset: Int) -> MLXArray {
        let length = xs.dim(1)
        let cosine = cosines[offset..<(offset + length)].expandedDimensions(axes: [0, 2])
        let sine = sines[offset..<(offset + length)].expandedDimensions(axes: [0, 2])
        let pairs = xs.asType(.float32).reshaped(xs.dim(0), length, xs.dim(2), -1, 2)
        let first = pairs[.ellipsis, 0]
        let second = pairs[.ellipsis, 1]
        let turned = stacked([first * cosine - second * sine, second * cosine + first * sine], axis: -1)
        return turned.reshaped(xs.shape).asType(xs.dtype)
    }
}

/// The keys and values a layer has seen, kept in room that grows by
/// `step` positions.
final class LayerCache {
    private var keys: MLXArray?
    private var values: MLXArray?
    private(set) var offset = 0
    private let step = 256

    func update(keys newKeys: MLXArray, values newValues: MLXArray) -> (MLXArray, MLXArray) {
        let previous = offset
        let length = newKeys.dim(2)
        if keys == nil || previous + length > (keys?.dim(2) ?? 0) {
            let room = (length + step - 1) / step * step
            var shape = newKeys.shape
            shape[2] = room
            let moreKeys = MLXArray.zeros(shape, dtype: newKeys.dtype)
            let moreValues = MLXArray.zeros(shape, dtype: newValues.dtype)
            if let keys, let values {
                self.keys = concatenated([keys[.ellipsis, ..<previous, 0...], moreKeys], axis: 2)
                self.values = concatenated([values[.ellipsis, ..<previous, 0...], moreValues], axis: 2)
            } else {
                keys = moreKeys
                values = moreValues
            }
        }
        offset += length
        guard var keys, var values else { return (newKeys, newValues) }
        keys[.ellipsis, previous..<offset, 0...] = newKeys
        values[.ellipsis, previous..<offset, 0...] = newValues
        self.keys = keys
        self.values = values
        return (keys[.ellipsis, ..<offset, 0...], values[.ellipsis, ..<offset, 0...])
    }
}

/// A weight matrix as PyTorch keeps it, [out, in], with its bias.
struct Linear {
    let weight: MLXArray
    let bias: MLXArray?

    func callAsFunction(_ xs: MLXArray) -> MLXArray {
        let product = matmul(xs, weight.T)
        guard let bias else { return product }
        return product + bias
    }
}

/// One layer of the transformers here: attention with grouped keys and
/// values, then a SiLU-gated feed-forward, each after an RMS norm, each
/// added back, scaled when the layer has scales.
struct TransformerLayer {
    struct Shape {
        var heads: Int
        var keyHeads: Int
        var headSize: Int
        var epsilon: Float
    }

    let shape: Shape
    let qkv: Linear
    let output: Linear
    let gate: Linear
    let up: Linear
    let down: Linear
    let attentionNorm: MLXArray
    let feedForwardNorm: MLXArray
    let attentionScale: MLXArray?
    let feedForwardScale: MLXArray?

    /// The layer under `prefix` in `weights`, named as the checkpoints
    /// name them.
    init(_ prefix: String, _ weights: Weights, shape: Shape, scaled: Bool = false) throws(Weights.Missing) {
        self.shape = shape
        qkv = Linear(
            weight: try weights["\(prefix).attention.wqkv.weight"],
            bias: weights.optional("\(prefix).attention.wqkv.bias"))
        output = Linear(weight: try weights["\(prefix).attention.wo.weight"], bias: nil)
        gate = Linear(weight: try weights["\(prefix).feed_forward.w1.weight"], bias: nil)
        down = Linear(weight: try weights["\(prefix).feed_forward.w2.weight"], bias: nil)
        up = Linear(weight: try weights["\(prefix).feed_forward.w3.weight"], bias: nil)
        attentionNorm = try weights["\(prefix).attention_norm.weight"]
        feedForwardNorm = try weights["\(prefix).ffn_norm.weight"]
        attentionScale = scaled ? try weights["\(prefix).attention_layer_scale.gamma"] : nil
        feedForwardScale = scaled ? try weights["\(prefix).ffn_layer_scale.gamma"] : nil
    }

    func callAsFunction(
        _ xs: MLXArray, rotary: Rotary, offset: Int, mask: MLXFast.ScaledDotProductAttentionMaskMode,
        cache: LayerCache?
    ) -> MLXArray {
        var attended = attention(
            rmsNorm(xs, weight: attentionNorm, eps: shape.epsilon), rotary: rotary, offset: offset, mask: mask,
            cache: cache)
        if let attentionScale { attended *= attentionScale }
        let hidden = xs + attended
        let normed = rmsNorm(hidden, weight: feedForwardNorm, eps: shape.epsilon)
        var fed = down(silu(gate(normed)) * up(normed))
        if let feedForwardScale { fed *= feedForwardScale }
        return hidden + fed
    }

    private func attention(
        _ xs: MLXArray, rotary: Rotary, offset: Int, mask: MLXFast.ScaledDotProductAttentionMaskMode,
        cache: LayerCache?
    ) -> MLXArray {
        let (batch, length) = (xs.dim(0), xs.dim(1))
        let size = shape.headSize
        let parts = split(qkv(xs), indices: [shape.heads * size, (shape.heads + shape.keyHeads) * size], axis: -1)
        let queries = rotary(parts[0].reshaped(batch, length, shape.heads, size), offset: offset)
            .transposed(0, 2, 1, 3)
        var keys = rotary(parts[1].reshaped(batch, length, shape.keyHeads, size), offset: offset)
            .transposed(0, 2, 1, 3)
        var values = parts[2].reshaped(batch, length, shape.keyHeads, size).transposed(0, 2, 1, 3)
        if let cache { (keys, values) = cache.update(keys: keys, values: values) }
        let attended = MLXFast.scaledDotProductAttention(
            queries: queries, keys: keys, values: values, scale: 1 / Float(size).squareRoot(), mask: mask)
        return output(attended.transposed(0, 2, 1, 3).reshaped(batch, length, shape.heads * size))
    }
}

private func silu(_ xs: MLXArray) -> MLXArray { xs * sigmoid(xs) }

/// Named weights, and which name was missing when one is.
struct Weights {
    struct Missing: Error {
        var name: String
    }

    var arrays: [String: MLXArray]

    subscript(_ name: String) -> MLXArray {
        get throws(Missing) {
            guard let array = arrays[name] else { throw Missing(name: name) }
            return array
        }
    }

    func optional(_ name: String) -> MLXArray? { arrays[name] }
}
