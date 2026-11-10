import JSONKit
import XCTest

@testable import InferenceKitLive

final class PartialJSONTests: XCTestCase {
    func testAStringCutOffReadsAsFarAsItGoes() {
        XCTAssertEqual(PartialJSON.value(#"{"message":"I added"#), ["message": "I added"])
        XCTAssertEqual(PartialJSON.value(#"{"message":""#), ["message": ""])
    }

    func testOpenArraysAndObjectsAreClosed() {
        XCTAssertEqual(
            PartialJSON.value(#"{"plan":{"tasks":[{"title":"Run"},{"title":"Re"#),
            ["plan": ["tasks": [["title": "Run"], ["title": "Re"]]]])
    }

    func testAKeyWithoutItsValueIsLeftOut() {
        XCTAssertEqual(PartialJSON.value(#"{"a":"b","mess"#), ["a": "b"])
        XCTAssertEqual(PartialJSON.value(#"{"a":"b","message":"#), ["a": "b"])
        XCTAssertEqual(PartialJSON.value(#"{"a":"b","#), ["a": "b"])
        XCTAssertEqual(PartialJSON.value(#"{"a":1,"b":tr"#), ["a": 1])
    }

    func testACutOffEscapeIsLeftOut() {
        XCTAssertEqual(PartialJSON.value(#"{"a":"one\"#), ["a": "one"])
        XCTAssertEqual(PartialJSON.value(#"{"a":"one\u00"#), ["a": "one"])
        XCTAssertEqual(PartialJSON.value(#"{"a":"say \"hi\" now"#), ["a": #"say "hi" now"#])
    }

    func testWholeJSONReadsAsItIs() {
        XCTAssertEqual(PartialJSON.value(#"{"answer":{"text":"Done"}}"#), ["answer": ["text": "Done"]])
    }

    func testNothingReadableYetIsNil() {
        XCTAssertNil(PartialJSON.value(""))
        XCTAssertNil(PartialJSON.value("Sure"))
    }
}
