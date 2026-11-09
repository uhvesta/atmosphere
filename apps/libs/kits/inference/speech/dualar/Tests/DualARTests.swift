import Foundation
import XCTest

@testable import InferenceKitDualAR

final class TorchCheckpointTests: XCTestCase {
    func testReadsEachTensorsShapeTypeAndBytes() throws {
        let checkpoint = try TorchCheckpoint(data: TinyCheckpoint.data)
        XCTAssertEqual(Set(checkpoint.tensors.keys), ["layer.weight", "layer.bias", "half"])

        let weight = try XCTUnwrap(checkpoint.tensors["layer.weight"])
        XCTAssertEqual(weight.shape, [2, 3])
        XCTAssertEqual(weight.type, .float32)
        XCTAssertEqual(floats(checkpoint.bytes(of: weight)), [0, 1, 2, 3, 4, 5])

        let bias = try XCTUnwrap(checkpoint.tensors["layer.bias"])
        XCTAssertEqual(floats(checkpoint.bytes(of: bias)), [0.5, -1.5])

        let half = try XCTUnwrap(checkpoint.tensors["half"])
        XCTAssertEqual(half.shape, [4])
        XCTAssertEqual(half.type, .bfloat16)
        let widened = stride(from: 0, to: half.byteCount, by: 2).map { offset in
            let bytes = checkpoint.bytes(of: half)
            let bits = UInt32(bytes[bytes.startIndex + offset]) | UInt32(bytes[bytes.startIndex + offset + 1]) << 8
            return Float(bitPattern: bits << 16)
        }
        XCTAssertEqual(widened, [1, 2, 3, 4])
    }

    func testRefusesWhatIsntAZip() {
        XCTAssertThrowsError(try TorchCheckpoint(data: Data("not a checkpoint".utf8))) { error in
            guard case .notAZip = error as? TorchCheckpoint.Failure else {
                return XCTFail("expected notAZip, got \(error)")
            }
        }
    }

    func testRefusesACutOffFile() {
        let cut = TinyCheckpoint.data.prefix(TinyCheckpoint.data.count / 2)
        XCTAssertThrowsError(try TorchCheckpoint(data: Data(cut)))
    }

    private func floats(_ data: Data) -> [Float] {
        stride(from: 0, to: data.count, by: 4).map { offset in
            let start = data.startIndex + offset
            let bits = (0..<4).reduce(UInt32(0)) { $0 | UInt32(data[start + $1]) << (8 * $1) }
            return Float(bitPattern: bits)
        }
    }
}

final class SamplerTests: XCTestCase {
    func testKeepsTheLikeliestUpToTopK() {
        let scores: [Float] = [1, 5, 3, 4, 2]
        XCTAssertEqual(Sampler<SplitMix64>.candidates(scores, topK: 3, topP: 1), [1, 3, 2])
    }

    func testCutsAtTopPButKeepsTheLikeliest() {
        // Probabilities about 0.87, 0.12 and 0.02.
        let scores: [Float] = [0, 2, 4]
        XCTAssertEqual(Sampler<SplitMix64>.candidates(scores, topK: 50, topP: 0.99), [2, 1])
        XCTAssertEqual(Sampler<SplitMix64>.candidates(scores, topK: 50, topP: 0.5), [2])
    }

    func testGreedyTakesTheLikeliest() {
        var sampler = Sampler(random: SplitMix64(seed: 1))
        var sampling = Sampling()
        sampling.greedy = true
        for _ in 0..<20 {
            XCTAssertEqual(sampler.pick([0.1, 3, 2.9, -1], sampling), 1)
        }
    }

    func testDrawsOnlyCandidatesInProportion() {
        var sampler = Sampler(random: SplitMix64(seed: 7))
        let sampling = Sampling(temperature: 1, topP: 1, topK: 2)
        let scores: [Float] = [log(3), log(1), log(0.5)]
        var counts = [0, 0, 0]
        for _ in 0..<4000 { counts[sampler.pick(scores, sampling)] += 1 }
        XCTAssertEqual(counts[2], 0)
        XCTAssertEqual(Double(counts[0]) / 4000, 0.75, accuracy: 0.03)
    }

    func testTheSameSeedPicksTheSame() {
        let scores = (0..<100).map { Float($0 % 7) }
        var first = Sampler(random: SplitMix64(seed: 42))
        var second = Sampler(random: SplitMix64(seed: 42))
        let sampling = Sampling(temperature: 1.5, topP: 0.99, topK: 50)
        XCTAssertEqual(
            (0..<50).map { _ in first.pick(scores, sampling) }, (0..<50).map { _ in second.pick(scores, sampling) })
    }
}
