import Foundation
import InferenceKit
import XCTest

@testable import InferenceKitDualAR

/// Audio8's Persian voice from the folder in DUALAR_DIR, checked against
/// what the model's own code makes. Skips without it; it downloads
/// nothing.
final class DualARSmokeTests: XCTestCase {
    private func engine(greedy: Bool = false) throws -> DualAREngine {
        guard let path = ProcessInfo.processInfo.environment["DUALAR_DIR"], !path.isEmpty else {
            throw XCTSkip("Set DUALAR_DIR to a downloaded Audio8-TTS-Persian")
        }
        let model = InstalledModel(
            spec: VoiceCatalog.audio8Persian.spec, directory: URL(filePath: path, directoryHint: .isDirectory),
            installedAt: .now)
        var sampling = Sampling()
        sampling.greedy = greedy
        return DualAREngine(model: model, sampling: sampling, maxFrames: 160, seed: 1)
    }

    func testThePromptIsTheProcessors() async throws {
        let prompt = try await engine().prompt(Reference.text)
        XCTAssertEqual(prompt, Reference.prompt)
    }

    func testTheCodecMakesTheReferenceAudio() async throws {
        let audio = try await engine().decode(Reference.codes)
        XCTAssertEqual(audio.count, Reference.samples)
        let rms = (audio.reduce(0) { $0 + $1 * $1 } / Float(max(audio.count, 1))).squareRoot()
        XCTAssertEqual(rms, Reference.rms, accuracy: Reference.rms * 0.02)
        let head = Array(audio.dropFirst(2048).prefix(Reference.head.count))
        let error = zip(head, Reference.head).map { abs($0 - $1) }.max() ?? .infinity
        XCTAssertLessThan(error, 1e-3)
    }

    func testGreedyCodesStartAsTheReferences() async throws {
        let engine = try engine(greedy: true)
        _ = try await engine.prompt(Reference.text)  // loads it, so only writing is timed
        let started = Date.now
        let codes = try await engine.codes(Reference.text)
        let seconds = Date.now.timeIntervalSince(started)
        let frames = codes.first?.count ?? 0
        print("greedy: \(frames) frames in \(seconds) s, \(Double(frames) / seconds) frames a second")
        XCTAssertEqual(codes.count, Reference.codes.count)
        // bfloat16 here, float32 there: the first frames agree, later ones
        // may drift apart.
        XCTAssertEqual(codes.first?.first, Reference.codes[0][0])
        let same = zip(codes[0], Reference.codes[0]).prefix(while: { $0 == $1 }).count
        print("greedy: the first \(same) semantic codes match the reference's")
        XCTAssertEqual(Double(frames), Double(Reference.codes[0].count), accuracy: 24)
    }

    func testSpeaksASentence() async throws {
        let pieces = Pieces()
        let started = Date.now
        try await engine().synthesize("سلام، حال شما چطور است؟ امروز هوا خیلی خوب است.", language: "fa", voice: "") {
            pieces.add($0)
        }
        let audio = pieces.all
        let seconds = audio.reduce(0) { $0 + $1.duration }
        print("spoke \(seconds) s of audio in \(Date.now.timeIntervalSince(started)) s")
        XCTAssertEqual(audio.first?.sampleRate, 44_100)
        XCTAssertGreaterThan(seconds, 1.5)
        XCTAssertLessThan(seconds, 12)
        if let out = ProcessInfo.processInfo.environment["DUALAR_WAV"], !out.isEmpty {
            try Wave.write(audio.flatMap(\.samples), rate: 44_100, to: URL(filePath: out))
        }
    }
}

private final class Pieces: @unchecked Sendable {
    private let lock = NSLock()
    private var pieces: [SpeechAudio] = []

    func add(_ piece: SpeechAudio) { lock.withLock { pieces.append(piece) } }
    var all: [SpeechAudio] { lock.withLock { pieces } }
}

/// 16-bit mono WAV, to listen to what the test made.
private enum Wave {
    static func write(_ samples: [Float], rate: Int, to url: URL) throws {
        var data = Data()
        func append<T: FixedWidthInteger>(_ value: T) {
            withUnsafeBytes(of: value.littleEndian) { data.append(contentsOf: $0) }
        }
        data.append(contentsOf: Array("RIFF".utf8))
        append(UInt32(36 + samples.count * 2))
        data.append(contentsOf: Array("WAVEfmt ".utf8))
        append(UInt32(16))
        append(UInt16(1))
        append(UInt16(1))
        append(UInt32(rate))
        append(UInt32(rate * 2))
        append(UInt16(2))
        append(UInt16(16))
        data.append(contentsOf: Array("data".utf8))
        append(UInt32(samples.count * 2))
        for sample in samples { append(Int16(max(-1, min(1, sample)) * 32767)) }
        try data.write(to: url)
    }
}
