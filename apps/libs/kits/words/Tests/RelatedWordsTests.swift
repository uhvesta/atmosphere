import InferenceKit
import WordsKit
import XCTest

/// Related words from a model, or the system's word lists when there's no
/// model or it can't answer.
final class RelatedWordsTests: XCTestCase {
    private let searched = "their coding agents' sessions"

    func testWithNoModelTheWordListsFindRelatedWords() async {
        let related = await RelatedWords.related(to: "groceries", searching: searched, on: nil)

        XCTAssertTrue(related.contains("grocery"), "\(related)")
        XCTAssertFalse(related.contains("groceries"))
        XCTAssertLessThanOrEqual(related.count, BuiltInWords.limit)
    }

    func testAModelsAnswerIsReadAsWordsWithoutListMarksOrTheQuery() async {
        let model = ScriptedRuntime(steps: [ScriptedRuntime.answer("Sure:\n1. Unit tests\n- ci, coverage\nflaky")])

        let related = await RelatedWords.related(to: "Flaky", searching: searched, on: model)

        XCTAssertEqual(related, ["unit tests", "ci", "coverage"])
        XCTAssertEqual(model.turns.count, 1)
    }

    func testAModelThatCantAnswerFallsBackToTheWordLists() async {
        let missing = ScriptedRuntime(status: .notDownloaded, steps: [])
        let builtIn = await RelatedWords.related(to: "groceries", searching: searched, on: nil)

        let fallback = await RelatedWords.related(to: "groceries", searching: searched, on: missing)

        XCTAssertFalse(builtIn.isEmpty)
        XCTAssertEqual(fallback, builtIn)
    }
}
