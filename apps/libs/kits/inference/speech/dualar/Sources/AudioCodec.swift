import Foundation
import MLX
import MLXNN

/// The codec's decoder: a frame's codes become `frameSize` samples of
/// audio. The codes are looked up and summed, a windowed causal
/// transformer reads them, two transposed convolutions bring them to four
/// times the frame rate, and a causal convolutional decoder with snake
/// activations makes the audio from that. Everything runs in float32.
final class AudioCodec {
    private let semanticCodebook: MLXArray
    private let semanticProjection: Linear
    private let residualCodebooks: [MLXArray]
    private let residualProjections: [Linear]
    private let layers: [TransformerLayer]
    private let norm: MLXArray
    private let window: Int
    private let rotary: Rotary
    private let upsample: [(Convolution, ConvNeXt)]
    private let decoder: Decoder

    /// The decoder's weights from the codec's checkpoint, its encoder's
    /// left unread.
    init(checkpoint: TorchCheckpoint, config: DualARConfig) throws(Weights.Missing) {
        let weights = Weights(
            arrays: Dictionary(
                uniqueKeysWithValues: checkpoint.tensors.compactMap { name, tensor -> (String, MLXArray)? in
                    let name = name.hasPrefix("codec.") ? String(name.dropFirst("codec.".count)) : name
                    guard name.hasPrefix("quantizer.") || name.hasPrefix("decoder.") else { return nil }
                    let type: DType =
                        switch tensor.type {
                        case .float32: .float32
                        case .float16: .float16
                        case .bfloat16: .bfloat16
                        }
                    return (name, MLXArray(checkpoint.bytes(of: tensor), tensor.shape, dtype: type).asType(.float32))
                }))
        let semantic = "quantizer.semantic_quantizer.quantizers.0"
        semanticCodebook = try weights["\(semantic).codebook.weight"]
        semanticProjection = try Self.projection(semantic, weights)
        residualCodebooks = try (0..<(config.numCodebooks - 1)).map { index throws(Weights.Missing) in
            try weights["quantizer.quantizer.quantizers.\(index).codebook.weight"]
        }
        residualProjections = try (0..<(config.numCodebooks - 1)).map { index throws(Weights.Missing) in
            try Self.projection("quantizer.quantizer.quantizers.\(index)", weights)
        }
        let heads = config.codecPostNHead
        let size = 64
        let shape = TransformerLayer.Shape(
            heads: heads, keyHeads: config.codecPostNLocalHeads, headSize: size, epsilon: 1e-5)
        layers = try (0..<config.codecPostNLayer).map { index throws(Weights.Missing) in
            try TransformerLayer("quantizer.post_module.layers.\(index)", weights, shape: shape, scaled: true)
        }
        norm = try weights["quantizer.post_module.norm.weight"]
        window = 128
        rotary = Rotary(length: config.maxSeqLen, headSize: size, base: 10000)
        upsample = try (0..<2).map { index throws(Weights.Missing) in
            let prefix = "quantizer.upsample.\(index)"
            return (
                try Convolution(transposedFrom: "\(prefix).0.conv", weights, stride: 2),
                try ConvNeXt("\(prefix).1", weights)
            )
        }
        decoder = try Decoder(weights)
    }

    /// The audio for `codes`, a row a codebook and a column a frame.
    func decode(_ codes: [[Int32]]) -> [Float] {
        guard let frames = codes.first?.count, frames > 0 else { return [] }
        let semantic = MLXArray(codes[0].map { min(max($0, 0), Int32(semanticCodebook.dim(0) - 1)) })
        var xs = semanticProjection(semanticCodebook[semantic])
        for (index, row) in codes.dropFirst().enumerated() where index < residualCodebooks.count {
            let book = residualCodebooks[index]
            let indices = MLXArray(row.map { min(max($0, 0), Int32(book.dim(0) - 1)) })
            xs += residualProjections[index](book[indices])
        }
        xs = xs.expandedDimensions(axis: 0)
        let mask = MLXFast.ScaledDotProductAttentionMaskMode.array(Self.windowMask(frames, window: window))
        for layer in layers {
            xs = layer(xs, rotary: rotary.extended(to: frames), offset: 0, mask: mask, cache: nil)
        }
        xs = rmsNorm(xs, weight: norm, eps: 1e-5)
        for (convolution, block) in upsample {
            xs = block(convolution(xs))
        }
        let audio = decoder(xs).reshaped(-1)
        eval(audio)
        return audio.asArray(Float.self)
    }

    /// Each frame sees itself and the `window - 1` before it.
    private static func windowMask(_ length: Int, window: Int) -> MLXArray {
        let rows = MLXArray(0..<Int32(length)).expandedDimensions(axis: 1)
        let columns = MLXArray(0..<Int32(length)).expandedDimensions(axis: 0)
        return (columns .<= rows) .&& (columns .>= rows - Int32(window - 1))
    }

    /// A quantizer's 1×1 output convolution, weight-normed, as a matrix.
    private static func projection(_ prefix: String, _ weights: Weights) throws(Weights.Missing) -> Linear {
        let weight = normed(
            direction: try weights["\(prefix).out_proj.weight_v"], magnitude: try weights["\(prefix).out_proj.weight_g"]
        )
        return Linear(weight: weight.squeezed(axis: 2), bias: try weights["\(prefix).out_proj.bias"])
    }
}

extension Rotary {
    /// These rotations, or new ones as long as `length` when they're
    /// shorter.
    fileprivate func extended(to length: Int) -> Rotary {
        cosines.dim(0) >= length ? self : Rotary(length: length, headSize: cosines.dim(1) * 2, base: 10000)
    }
}

/// A weight from weight norm's parts: `direction` scaled, along its first
/// axis, to `magnitude`.
private func normed(direction: MLXArray, magnitude: MLXArray) -> MLXArray {
    let length = sqrt((direction * direction).sum(axes: Array(1..<direction.ndim), keepDims: true))
    return magnitude * direction / length
}

/// A causal 1-D convolution, or a transposed one, on [batch, length,
/// channels].
private struct Convolution {
    let weight: MLXArray
    let bias: MLXArray
    let stride: Int
    let dilation: Int
    let groups: Int
    let transposed: Bool

    /// A convolution PyTorch stores as [out, in, kernel], weight-normed
    /// when its parts are there.
    init(_ prefix: String, _ weights: Weights, dilation: Int = 1, groups: Int = 1) throws(Weights.Missing) {
        let weight = try Self.weight(prefix, weights)
        self.weight = weight.transposed(0, 2, 1)
        bias = try weights["\(prefix).bias"]
        stride = 1
        self.dilation = dilation
        self.groups = groups
        transposed = false
    }

    /// A transposed convolution PyTorch stores as [in, out, kernel].
    init(transposedFrom prefix: String, _ weights: Weights, stride: Int) throws(Weights.Missing) {
        let weight = try Self.weight(prefix, weights)
        self.weight = weight.transposed(1, 2, 0)
        bias = try weights["\(prefix).bias"]
        self.stride = stride
        dilation = 1
        groups = 1
        transposed = true
    }

    private static func weight(_ prefix: String, _ weights: Weights) throws(Weights.Missing) -> MLXArray {
        if let weight = weights.optional("\(prefix).weight") { return weight }
        return normed(
            direction: try weights["\(prefix).parametrizations.weight.original1"],
            magnitude: try weights["\(prefix).parametrizations.weight.original0"])
    }

    func callAsFunction(_ xs: MLXArray) -> MLXArray {
        let kernel = weight.dim(1)
        if transposed {
            let output = convTransposed1d(xs, weight, stride: stride) + bias
            let crop = kernel - stride
            return crop > 0 ? output[0..., ..<(output.dim(1) - crop), 0...] : output
        }
        let reach = (kernel - 1) * dilation + 1
        let padded = MLX.padded(xs, widths: [.init(0), .init((reach - stride, 0)), .init(0)])
        return conv1d(padded, weight, stride: stride, dilation: dilation, groups: groups) + bias
    }
}

/// x + sin²(αx) / α, a channel at a time.
private struct Snake {
    let alpha: MLXArray

    init(_ name: String, _ weights: Weights) throws(Weights.Missing) {
        alpha = try weights[name].reshaped(1, 1, -1)
    }

    func callAsFunction(_ xs: MLXArray) -> MLXArray {
        let wave = sin(alpha * xs)
        return xs + wave * wave / (alpha + 1e-9)
    }
}

/// A depthwise causal convolution, a layer norm, a GELU-gated widening
/// and narrowing, scaled and added back.
private struct ConvNeXt {
    let depthwise: Convolution
    let normWeight: MLXArray
    let normBias: MLXArray
    let widen: Linear
    let narrow: Linear
    let gamma: MLXArray

    init(_ prefix: String, _ weights: Weights) throws(Weights.Missing) {
        let channels = try weights["\(prefix).gamma"].dim(0)
        depthwise = try Convolution("\(prefix).dwconv.conv", weights, groups: channels)
        normWeight = try weights["\(prefix).norm.weight"]
        normBias = try weights["\(prefix).norm.bias"]
        widen = Linear(weight: try weights["\(prefix).pwconv1.weight"], bias: try weights["\(prefix).pwconv1.bias"])
        narrow = Linear(weight: try weights["\(prefix).pwconv2.weight"], bias: try weights["\(prefix).pwconv2.bias"])
        gamma = try weights["\(prefix).gamma"]
    }

    func callAsFunction(_ xs: MLXArray) -> MLXArray {
        let normed = layerNorm(depthwise(xs), weight: normWeight, bias: normBias, eps: 1e-6)
        return xs + gamma * narrow(gelu(widen(normed)))
    }
}

/// Two snakes around a dilated convolution and a 1×1 one, added back.
private struct ResidualUnit {
    let first: Snake
    let wide: Convolution
    let second: Snake
    let narrow: Convolution

    init(_ prefix: String, _ weights: Weights, dilation: Int) throws(Weights.Missing) {
        first = try Snake("\(prefix).block.0.alpha", weights)
        wide = try Convolution("\(prefix).block.1.conv", weights, dilation: dilation)
        second = try Snake("\(prefix).block.2.alpha", weights)
        narrow = try Convolution("\(prefix).block.3.conv", weights)
    }

    func callAsFunction(_ xs: MLXArray) -> MLXArray {
        let output = narrow(second(wide(first(xs))))
        let extra = xs.dim(1) - output.dim(1)
        return (extra > 0 ? xs[0..., ..<output.dim(1), 0...] : xs) + output
    }
}

/// From the codes' features, at four times the frame rate, to audio.
private struct Decoder {
    struct Block {
        let snake: Snake
        let upsample: Convolution
        let units: [ResidualUnit]
    }

    let input: Convolution
    let blocks: [Block]
    let snake: Snake
    let output: Convolution

    init(_ weights: Weights) throws(Weights.Missing) {
        input = try Convolution("decoder.model.0.conv", weights)
        blocks = try zip(1...4, [8, 8, 4, 2]).map { index, stride throws(Weights.Missing) in
            let prefix = "decoder.model.\(index).block"
            return Block(
                snake: try Snake("\(prefix).0.alpha", weights),
                upsample: try Convolution(transposedFrom: "\(prefix).1.conv", weights, stride: stride),
                units: try zip(2...4, [1, 3, 9]).map { unit, dilation throws(Weights.Missing) in
                    try ResidualUnit("\(prefix).\(unit)", weights, dilation: dilation)
                })
        }
        snake = try Snake("decoder.model.5.alpha", weights)
        output = try Convolution("decoder.model.6.conv", weights)
    }

    func callAsFunction(_ xs: MLXArray) -> MLXArray {
        var xs = input(xs)
        for block in blocks {
            xs = block.upsample(block.snake(xs))
            for unit in block.units { xs = unit(xs) }
        }
        return tanh(output(snake(xs)))
    }
}
