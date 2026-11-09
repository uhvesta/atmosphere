import XCTest

@testable import InferenceKitSpeechText

final class SpokenTextTests: XCTestCase {
    private func persian(_ text: String) -> String {
        SpokenText.normalized(text, language: "fa")
    }

    func testArabicLettersBecomeTheirPersianForms() {
        XCTAssertEqual(persian("كتاب علي"), "کتاب علی")
        XCTAssertEqual(persian("أحمد و إيران"), "احمد و ایران")
    }

    func testShortVowelsJoinersAndTatweelAreLeftOut() {
        XCTAssertEqual(persian("سَلامٌ"), "سلام")
        XCTAssertEqual(persian("می\u{200C}خواهم"), "میخواهم")
        XCTAssertEqual(persian("خیـــلی"), "خیلی")
    }

    func testNumbersAreReadInWordsInEitherScript() {
        XCTAssertEqual(persian("۱۲۳ تومان"), "یکصد و بیست و سه تومان")
        XCTAssertEqual(persian("105 و 120 و 19 و 0"), "یکصد و پنج و یکصد و بیست و نوزده و صفر")
    }

    func testAThousandIsNotOneThousand() {
        XCTAssertEqual(persian("1000"), "هزار")
        XCTAssertEqual(persian("1001"), "هزار و یک")
        XCTAssertEqual(persian("2000"), "دو هزار")
        XCTAssertEqual(persian("1000000"), "یک میلیون")
    }

    func testGroupedThousandsAreReadAsOneNumber() {
        XCTAssertEqual(
            persian("1,234,567 ریال"),
            "یک میلیون و دویست و سی و چهار هزار و پانصد و شصت و هفت ریال")
        XCTAssertEqual(persian("۱٬۲۳۴"), "هزار و دویست و سی و چهار")
    }

    func testDecimalsPercentsAndListsAreSpoken() {
        XCTAssertEqual(persian("3.05 متر"), "سه ممیز پنج متر")
        XCTAssertEqual(persian("۷٫۵"), "هفت ممیز پنج")
        XCTAssertEqual(persian("۲۵٪ تخفیف"), "بیست و پنج درصد تخفیف")
        XCTAssertEqual(persian("1,2 و 3"), "یک، دو و سه")
    }

    func testMarksSitAgainstTheirWordAndSpacesCollapse() {
        XCTAssertEqual(persian("«سلام» ، خوبی ؟"), "\"سلام\"، خوبی؟")
        XCTAssertEqual(persian("  چند   فاصله  "), "چند فاصله")
    }

    func testOtherLanguagesOnlyHaveTheirSpacingTidied() {
        XCTAssertEqual(SpokenText.normalized("  Hello,   1000 times ", language: "en"), "Hello, 1000 times")
    }

    func testShortTextIsOnePieceAndEmptyTextNone() {
        XCTAssertEqual(SpokenText.pieces(of: "سلام. خوبی؟", language: "fa"), ["سلام. خوبی؟"])
        XCTAssertEqual(SpokenText.pieces(of: " \u{200C} ", language: "fa"), [])
    }

    func testLongTextIsCutBetweenSentencesAndKeepsEveryWord() {
        let sentence = "این یک جمله نسبتا طولانی است که برای آزمایش نوشته شده."
        let text = Array(repeating: sentence, count: 6).joined(separator: " ")
        let pieces = SpokenText.pieces(of: text, language: "fa")
        XCTAssertGreaterThan(pieces.count, 1)
        XCTAssertTrue(pieces.allSatisfy { $0.unicodeScalars.count <= SpokenText.pieceLength })
        XCTAssertTrue(pieces.allSatisfy { $0.hasSuffix(".") })
        XCTAssertEqual(pieces.joined(separator: " "), text)
    }

    func testASentenceTooLongIsCutAtACommaOrSpace() {
        let text = "و این یکی، که ویرگول دارد، باید " + Array(repeating: "کلمه", count: 60).joined(separator: " ")
        let pieces = SpokenText.pieces(of: text, language: "fa", length: 50)
        XCTAssertTrue(pieces.allSatisfy { $0.unicodeScalars.count <= 51 })
        XCTAssertTrue(pieces.allSatisfy { !$0.hasPrefix(" ") && !$0.hasSuffix(" ") })
        XCTAssertEqual(pieces.joined(separator: " ").split(separator: " "), text.split(separator: " "))
    }
}
