import XCTest

@testable import InferenceKitSupertonic

final class SupertonicTextTests: XCTestCase {
    /// A table that knows every character up to U+FFFF, by its own value.
    private let indexer = (0..<65_536).map { Int32($0) }

    func testTextIsTaggedWithItsLanguageAndEndsInAStop() {
        XCTAssertEqual(SupertonicText.prepared("Hello   world", language: "en"), "<en>Hello world.</en>")
        XCTAssertEqual(SupertonicText.prepared("Ready?", language: "en"), "<en>Ready?</en>")
    }

    func testCurlyQuotesAndDashesAreMadePlain() {
        XCTAssertEqual(
            SupertonicText.prepared("\u{201C}Yes\u{201D} \u{2014} it\u{2019}s", language: "en"),
            "<en>\"Yes\" - it's.</en>")
    }

    func testEmojiAreLeftOut() {
        XCTAssertEqual(SupertonicText.prepared("Hi 😀 there", language: "en"), "<en>Hi there.</en>")
    }

    func testShortTextIsOnePiece() {
        XCTAssertEqual(SupertonicText.pieces(of: "One. Two."), ["One. Two."])
    }

    func testLongTextIsCutBetweenSentencesToFit() {
        let sentence = String(repeating: "word ", count: 15).trimmingCharacters(in: .whitespaces) + "."
        let text = Array(repeating: sentence, count: 4).joined(separator: " ")

        let pieces = SupertonicText.pieces(of: text)

        XCTAssertGreaterThan(pieces.count, 1)
        XCTAssertTrue(pieces.allSatisfy { $0.count <= SupertonicText.pieceLength }, "\(pieces)")
        XCTAssertEqual(pieces.joined(separator: " "), text)
    }

    func testAWordLongerThanAPieceIsCut() {
        let word = String(repeating: "a", count: 300)

        let pieces = SupertonicText.pieces(of: word)

        XCTAssertEqual(pieces.joined(), word)
        XCTAssertTrue(pieces.allSatisfy { $0.count <= SupertonicText.pieceLength })
    }

    func testEveryPieceFitsTheModelOnceTagged() {
        // Hangul syllables come apart into two or three letters each.
        let korean = String(repeating: "안녕하세요 ", count: 30)

        let pieces = SupertonicText.encoded(korean, language: "ko", indexer: indexer)

        XCTAssertGreaterThan(pieces.count, 1)
        XCTAssertTrue(pieces.allSatisfy { $0.count <= SupertonicText.width }, "\(pieces.map(\.count))")
    }

    func testCharactersTheModelDoesntKnowAreLeftOut() {
        var indexer = indexer
        indexer[98] = -1  // "b"

        XCTAssertEqual(SupertonicText.ids(of: "abc", indexer: indexer), [97, 99])
    }
}
