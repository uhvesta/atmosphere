import LifrAPI
import LifrModelsClient
import LifrSearchClient
import LifrSearchLive
import XCTest

/// Words related to a search: from the system's word lists with no model,
/// or from the model picked for searching, falling back to the lists.
final class LiveRelatedWordsTests: XCTestCase {
    private func client(_ selected: ModelID, _ model: any ModelRuntime = ScriptedRuntime(steps: []))
        -> RelatedWordsClient
    {
        .live(selected: { selected }, runtime: { model })
    }

    func testWithNoModelTheSystemsWordListsFindRelatedWords() async {
        let built = client(.builtIn)

        let related = await built.related("groceries")

        XCTAssertEqual(built.source(), .builtIn)
        XCTAssertTrue(related.contains("grocery"), "\(related)")
        XCTAssertTrue(related.contains("shopping"), "\(related)")
        XCTAssertFalse(related.contains("groceries"))
        XCTAssertLessThanOrEqual(related.count, 12)
    }

    func testEachWordSearchedForGetsSomeRelatedWords() async {
        let related = await client(.builtIn).related("the plumber in the kitchen")

        XCTAssertTrue(related.contains("electrician"), "\(related)")
        XCTAssertTrue(related.contains("pantry") || related.contains("cupboard"), "\(related)")
    }

    func testWordsTheSystemDoesntKnowHaveNoRelatedWords() async {
        let unknown = await client(.builtIn).related("xyzzy qwrtp")
        let none = await client(.builtIn).related("the and of")

        XCTAssertEqual(unknown, [])
        XCTAssertEqual(none, [])
    }

    func testAModelPickedForSearchingFindsTheWords() async {
        let model = ScriptedRuntime(steps: [ScriptedRuntime.answer("Here you go:\n1. Milk\n- eggs, bread\ngroceries")])
        let picked = client("gemma", model)

        let related = await picked.related("Groceries")

        XCTAssertEqual(picked.source(), .model)
        XCTAssertEqual(related, ["milk", "eggs", "bread"])
        XCTAssertEqual(model.turns.count, 1)
    }

    func testAModelThatCantAnswerFallsBackToTheWordLists() async {
        let failing = ScriptedRuntime(steps: [ScriptedRuntime.fail(.failed("out of memory"))])
        let missing = ScriptedRuntime(status: .notDownloaded, steps: [])
        let builtIn = await client(.builtIn).related("groceries")

        let afterFailing = await client("gemma", failing).related("groceries")
        let afterMissing = await client("gemma", missing).related("groceries")

        XCTAssertFalse(builtIn.isEmpty)
        XCTAssertEqual(afterFailing, builtIn)
        XCTAssertEqual(afterMissing, builtIn)
    }
}
