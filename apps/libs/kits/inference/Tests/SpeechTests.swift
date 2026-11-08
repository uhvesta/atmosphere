import Foundation
import InferenceKit
import XCTest

final class SpeechRouteTests: XCTestCase {
    private let voices = VoiceCatalog.supertonic3

    private var downloaded: InstalledModel {
        InstalledModel(
            spec: voices.spec, directory: URL(filePath: "/nowhere"), installedAt: Date(timeIntervalSince1970: 0))
    }

    func testTheSystemVoiceSpeaksWhenItsPicked() {
        XCTAssertEqual(SpeechRoute.resolve(.system, language: "en", installed: [downloaded]), .system)
    }

    func testAPickedModelSpeaksALanguageItKnowsOnceDownloaded() {
        let route = SpeechRoute.resolve(.model(voices.id, voice: "M1"), language: "fr-CA", installed: [downloaded])

        XCTAssertEqual(route, .model(downloaded, voices, voice: "M1"))
    }

    func testAModelThatIsntDownloadedFallsBackToTheSystemVoice() {
        XCTAssertEqual(SpeechRoute.resolve(.model(voices.id, voice: "M1"), language: "en", installed: []), .system)
    }

    func testALanguageTheModelDoesntSpeakFallsBackToTheSystemVoice() {
        let route = SpeechRoute.resolve(.model(voices.id, voice: "M1"), language: "zh-Hant", installed: [downloaded])

        XCTAssertEqual(route, .system)
    }

    func testAVoiceTheModelDoesntHaveFallsBackToTheSystemVoice() {
        let route = SpeechRoute.resolve(.model(voices.id, voice: "X9"), language: "en", installed: [downloaded])

        XCTAssertEqual(route, .system)
    }

    func testLanguagesMatchOnTheirBaseSubtag() {
        XCTAssertTrue(voices.speaks("PT_br"))
        XCTAssertTrue(voices.speaks("ja"))
        XCTAssertFalse(voices.speaks("zh"))
    }

    func testVoiceModelsArentModelsToChatWith() {
        let runnable = RunnableModel.list([downloaded])

        XCTAssertEqual(runnable.map(\.id), [.apple])
    }

    func testEveryVoiceModelFileIsPinned() throws {
        for model in VoiceCatalog.all {
            let source = try XCTUnwrap(model.spec.source, model.spec.name)
            XCTAssertEqual(source.commit.count, 40, model.spec.name)
            XCTAssertFalse(source.files.isEmpty)
            for file in source.files {
                switch file.digest {
                case .sha256(let hex): XCTAssertEqual(hex.count, 64, file.path)
                case .gitBlobSHA1(let hex): XCTAssertEqual(hex.count, 40, file.path)
                }
            }
            for voice in model.voices {
                XCTAssertTrue(source.files.contains { $0.path == "voice_styles/\(voice.id).json" }, voice.id)
            }
        }
    }
}

final class VoiceSelectionTests: XCTestCase {
    private let model = VoiceCatalog.supertonic3.id

    func testEveryLanguageStartsWithTheSystemVoice() {
        let selection = VoiceSelection(file: nil)

        XCTAssertEqual(selection.choice(for: "en"), .system)
        XCTAssertEqual(selection.choice(), .system)
    }

    func testALanguageFollowsThePickForEveryLanguageUntilItHasItsOwn() {
        let selection = VoiceSelection(file: nil)
        selection.choose(.model(model, voice: "F1"))
        selection.choose(.system, for: "ja-JP")

        XCTAssertEqual(selection.choice(for: "en"), .model(model, voice: "F1"))
        XCTAssertEqual(selection.choice(for: "ja"), .system)
        XCTAssertTrue(selection.hasOwnChoice(for: "ja"))

        selection.choose(nil, for: "ja")

        XCTAssertEqual(selection.choice(for: "ja"), .model(model, voice: "F1"))
    }

    func testForgettingAModelSendsItsPicksBackToTheSystemVoice() {
        let selection = VoiceSelection(file: nil)
        selection.choose(.model(model, voice: "F1"))
        selection.choose(.model(model, voice: "M3"), for: "de")

        selection.forget(model)

        XCTAssertEqual(selection.choice(for: "de"), .system)
        XCTAssertEqual(selection.choice(), .system)
    }

    func testPicksAreKeptInTheFile() throws {
        let file = FileManager.default.temporaryDirectory.appending(path: "voices-\(UUID()).json")
        defer { try? FileManager.default.removeItem(at: file) }

        VoiceSelection(file: file).choose(.model(model, voice: "M2"), for: "ko")

        XCTAssertEqual(VoiceSelection(file: file).choice(for: "ko"), .model(model, voice: "M2"))
    }
}
