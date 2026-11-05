import Foundation
import LifrAssistantClient
import LifrCore
import LifrStoreClient
import XCTest

final class GuidanceTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_793_700_000)

    func testStartsWithLifrsDefault() {
        let guidance = Guidance()
        XCTAssertTrue(guidance.isDefault)
        XCTAssertEqual(guidance.text, Guidance.defaultText)
    }

    func testSavingUsesTheNewVersionAndKeepsTheOldOnes() throws {
        var guidance = Guidance()
        let first = UUID()
        let second = UUID()
        try guidance.save("Be brief.", id: first, at: now)
        try guidance.save("  Be cheerful.\n", id: second, at: now.addingTimeInterval(60))

        XCTAssertEqual(guidance.text, "Be cheerful.")
        XCTAssertEqual(guidance.history.map(\.id), [second, first])
    }

    func testAnyVersionOrTheDefaultCanBeUsedAgain() throws {
        var guidance = Guidance()
        let first = UUID()
        try guidance.save("Be brief.", id: first, at: now)
        try guidance.save("Be cheerful.", id: UUID(), at: now)

        guidance.use(first)
        XCTAssertEqual(guidance.text, "Be brief.")
        guidance.use(nil)
        XCTAssertEqual(guidance.text, Guidance.defaultText)
        XCTAssertEqual(guidance.history.count, 2)
    }

    func testUsingAnUnknownVersionChangesNothing() throws {
        var guidance = Guidance()
        try guidance.save("Be brief.", id: UUID(), at: now)
        let before = guidance
        guidance.use(UUID())
        XCTAssertEqual(guidance, before)
    }

    func testSavingTheSameTextOrTheDefaultAddsNoVersion() throws {
        var guidance = Guidance()
        try guidance.save("Be brief.", id: UUID(), at: now)
        try guidance.save("Be brief.", id: UUID(), at: now)
        XCTAssertEqual(guidance.history.count, 1)

        try guidance.save(Guidance.defaultText, id: UUID(), at: now)
        XCTAssertTrue(guidance.isDefault)
        XCTAssertEqual(guidance.history.count, 1)
    }

    func testEmptyOrTooLongTextIsRefused() {
        var guidance = Guidance()
        XCTAssertThrowsError(try guidance.save(" \n", id: UUID(), at: now)) { error in
            XCTAssertEqual(error as? GuidanceError, .empty)
        }
        let long = String(repeating: "a", count: Guidance.limit + 1)
        XCTAssertThrowsError(try guidance.save(long, id: UUID(), at: now)) { error in
            XCTAssertEqual(error as? GuidanceError, .tooLong(Guidance.limit + 1))
        }
        XCTAssertEqual(guidance, Guidance())
    }

    func testHistoryDropsTheOldestVersions() throws {
        var guidance = Guidance()
        for index in 0...Guidance.historyLimit {
            try guidance.save("Version \(index)", id: UUID(), at: now)
        }
        XCTAssertEqual(guidance.history.count, Guidance.historyLimit)
        XCTAssertEqual(guidance.text, "Version \(Guidance.historyLimit)")
        XCTAssertFalse(guidance.history.contains { $0.text == "Version 0" })
    }

    func testTheAssistantIsToldThePersonsInstructionsAndLifrsOwn() {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        let home = records.avenue("HOME", "Home")
        let request = AssistantRequest(
            messages: [ChatMessage(id: UUID(), role: .person, text: "Hi", at: now)],
            workspace: records.workspace, now: now, timeZone: .gmt, guidance: "Answer like a pirate.")

        for instructions in [AssistantPrompt.instructions(request), AssistantPrompt.plainInstructions(request)] {
            XCTAssertTrue(instructions.contains("Answer like a pirate."))
            XCTAssertFalse(instructions.contains(Guidance.defaultText))
            XCTAssertTrue(instructions.contains(home.key.rawValue))
        }
    }
}
