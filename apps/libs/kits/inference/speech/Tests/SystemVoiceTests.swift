import InferenceKit
import XCTest

@testable import InferenceKitSpeech

final class SystemVoiceTests: XCTestCase {
    func testALanguageWithNoSystemVoiceFailsRatherThanSpeakingInAnother() async {
        // Neither iOS nor macOS ships a voice for Persian.
        do throws(SpeechError) {
            try await SystemVoice().synthesize("سلام، حالت چطوره؟", language: "fa-IR", voice: "") { _ in }
            XCTFail("Persian was spoken in another language's voice.")
        } catch {
            XCTAssertEqual(error, .systemVoiceFailed(language: "fa-IR"))
        }
    }

    func testARegionWithNoVoiceOfItsOwnUsesItsLanguagesVoice() {
        XCTAssertEqual(SystemVoice.voice(for: "en-NZ").map { VoiceModel.base(of: $0.language) }, "en")
        XCTAssertEqual(SystemVoice.voice(for: "en").map { VoiceModel.base(of: $0.language) }, "en")
    }
}
