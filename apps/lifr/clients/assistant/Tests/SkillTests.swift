import Foundation
import LifrAPI
import LifrAssistantClient
import LifrCore
import XCTest

final class SkillTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_793_700_000)

    private func request(_ skill: Skill?, writes: Bool = true) -> AssistantRequest {
        AssistantRequest(
            messages: [], workspace: Workspace(), now: now, timeZone: .gmt,
            writes: writes ? AssistantWrites(conversation: UUID()) { _ in Workspace() } : nil, skill: skill)
    }

    func testWithoutASkillEveryToolIsAllowedAndPlansAreDrafted() {
        XCTAssertEqual(request(nil).allowedTools, Set(AssistantTool.allCases))
        XCTAssertTrue(request(nil).draftsPlans)
    }

    func testASkillAllowsOnlyItsTools() {
        XCTAssertEqual(request(.undo).allowedTools, [.recentActivity, .revert])
        XCTAssertFalse(request(.undo).draftsPlans)
        XCTAssertTrue(request(.add).draftsPlans)
    }

    func testToolsThatSaveNeedWrites() {
        XCTAssertEqual(request(.undo, writes: false).allowedTools, [.recentActivity])
        XCTAssertTrue(request(nil, writes: false).allowedTools.allSatisfy { !$0.writes })
    }

    func testTheInstructionsFollowThePickedSkill() {
        let text = AssistantPrompt.instructions(request(.tidy))
        XCTAssertTrue(text.contains(Skill.tidy.instructions))
        XCTAssertFalse(text.contains("revert"))
    }

    func testTheHistorySaysWhichSkillEachMessageUsed() {
        var request = request(nil)
        request.messages = [ChatMessage(id: UUID(), role: .person, text: "Clean up", at: now, skill: "tidy")]
        XCTAssertTrue(AssistantPrompt.prompt(request).contains("tidy"))
    }

    func testASkillDraftNeedsInstructionsAndKeepsOnlyKnownTools() {
        let answer = JSONValue.object([
            "name": .string("Weekly review"), "summary": .string("Fridays."),
            "instructions": .string("Go through the week."),
            "tools": .array([.string("overview"), .string("sendEmail")]), "draftsPlans": .bool(false),
        ])
        let draft = AssistantPrompt.skillDraft(from: answer)
        XCTAssertEqual(draft?.tools, [.overview])
        XCTAssertEqual(draft?.name, "Weekly review")
        XCTAssertEqual(draft?.draftsPlans, false)
        XCTAssertNil(AssistantPrompt.skillDraft(from: .object(["name": .string("Empty")])))
    }

    func testAPlanIsDroppedWhenNoSkillDraftsPlans() {
        let answer = JSONValue.object([
            "message": .string("Done."), "plan": .object(["tasks": .array([.object(["title": .string("Run")])])]),
        ])
        XCTAssertNil(AssistantPrompt.reply(from: answer, isComplete: true, draftsPlans: false).plan)
        XCTAssertNotNil(AssistantPrompt.reply(from: answer, isComplete: true).plan)
    }

    func testSearchRanksTitlePrefixThenLettersInOrderThenSummary() throws {
        let weekly = try XCTUnwrap(
            Skill(userNamed: "Weekly tidy", summary: "Friday cleanup", instructions: "", tools: [], draftsPlans: false))
        let skills = Skill.builtIn + [weekly]

        XCTAssertEqual(Skill.search("ti", in: skills).map(\.id).first, "tidy")
        XCTAssertEqual(Skill.search("wee", in: skills).map(\.id), ["user:weekly-tidy"])
        XCTAssertEqual(Skill.search("rvw", in: skills).map(\.id), ["review"])
        XCTAssertEqual(Skill.search("friday", in: skills).map(\.id), ["user:weekly-tidy"])
        XCTAssertEqual(Skill.search("", in: skills), skills)
        XCTAssertEqual(Skill.search("zzz", in: skills), [])
    }

    func testAUserSkillsIDComesFromItsName() {
        let skill = Skill(userNamed: "  Weekly Review! ", summary: "", instructions: "", tools: [], draftsPlans: false)
        XCTAssertEqual(skill?.id, "user:weekly-review")
        XCTAssertEqual(skill?.isUser, true)
        XCTAssertNil(Skill(userNamed: "!!", summary: "", instructions: "", tools: [], draftsPlans: false))
    }

    func testStoredBuiltInsOnlyKeepWhetherTheyreOn() {
        var changed = Skill.query
        changed.instructions = "Something else"
        changed.isOn = false
        let merged = Skill.merged(stored: [changed])
        XCTAssertEqual(merged.map(\.id), Skill.builtIn.map(\.id))
        XCTAssertEqual(merged[0].instructions, Skill.query.instructions)
        XCTAssertFalse(merged[0].isOn)
    }

    func testAMessageSavedBeforeSkillsHasNone() throws {
        let old = Data(#"{"id":"\#(UUID())","role":"person","text":"Hi","at":0}"#.utf8)
        let decoded = try JSONDecoder().decode(ChatMessage.self, from: old)
        XCTAssertNil(decoded.skill)
        XCTAssertEqual(decoded.text, "Hi")
    }

    func testTheInMemoryClientKeepsUserSkillsAndBuiltInSwitches() async throws {
        let client = SkillsClient.inMemory()
        let mine = try XCTUnwrap(
            Skill(userNamed: "Mine", summary: "", instructions: "Be brief.", tools: [.findTasks], draftsPlans: false))
        var undo = Skill.undo
        undo.isOn = false
        try await client.save(mine)
        try await client.save(undo)

        let loaded = try await client.load()
        XCTAssertEqual(loaded.map(\.id), Skill.builtIn.map(\.id) + ["user:mine"])
        XCTAssertEqual(loaded.first { $0.id == "undo" }?.isOn, false)

        try await client.delete(["user:mine"])
        let afterDelete = try await client.load()
        XCTAssertEqual(afterDelete.map(\.id), Skill.builtIn.map(\.id))
    }
}
