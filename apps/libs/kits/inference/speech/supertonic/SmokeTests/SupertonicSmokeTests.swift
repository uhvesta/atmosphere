import Foundation
import InferenceKit
import InferenceKitSupertonic
import XCTest

/// Speaks with a Supertonic 3 downloaded by hand into the folder in
/// SUPERTONIC_DIR, the repository's files as they are, and checks the audio
/// is about as long as the sentence takes to say.
final class SupertonicSmokeTests: XCTestCase {
    func testASentenceBecomesSpeech() async throws {
        guard let folder = ProcessInfo.processInfo.environment["SUPERTONIC_DIR"] else {
            throw XCTSkip("Set SUPERTONIC_DIR to a folder with Supertonic 3's files")
        }
        let model = InstalledModel(
            spec: VoiceCatalog.supertonic3.spec, directory: URL(filePath: folder, directoryHint: .isDirectory),
            installedAt: Date())
        let engine = SupertonicEngine(model: model)
        let pieces = Pieces()

        try await engine.synthesize("The weather is lovely today.", language: "en", voice: "F1") { pieces.add($0) }

        let seconds = pieces.all.reduce(0) { $0 + $1.duration }
        XCTAssertGreaterThan(seconds, 0.8)
        XCTAssertLessThan(seconds, 6)
        XCTAssertTrue(pieces.all.allSatisfy { $0.sampleRate == 44_100 })
        let loudest = pieces.all.flatMap(\.samples).map(abs).max() ?? 0
        XCTAssertGreaterThan(loudest, 0.01, "It's silent")
    }
}

private final class Pieces: @unchecked Sendable {
    private let lock = NSLock()
    private var pieces: [SpeechAudio] = []

    func add(_ piece: SpeechAudio) { lock.withLock { pieces.append(piece) } }
    var all: [SpeechAudio] { lock.withLock { pieces } }
}
