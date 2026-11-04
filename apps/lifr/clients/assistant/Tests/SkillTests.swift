import Foundation
import LifrAPI
import LifrAssistantClient
import LifrCore
import XCTest

final class SkillTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_793_700_000)

    private func request(_ skills: [Skill], writes: Bool = true) -> AssistantRequest {
        AssistantRequest(
            messages: [], workspace: Workspace(), now: now, timeZone: .gmt,
            writes: writes ? AssistantWrites(conversation: UUID()) { _ in Workspace() } : nil,
            skills: skills.map { SkillUse($0) })
    }

    func testWithoutSkillsEveryToolIsAllowedAndPlansAreDrafted() {
        XCTAssertEqual(request([]).allowedTools, Set(AssistantTool.allCases))
        XCTAssertTrue(request([]).draftsPlans)
    }

    func testAttachedSkillsAllowTheirToolsTogether() {
        let request = request([.query, .undo])
        XCTAssertEqual(request.allowedTools, [.findTasks, .getTask, .overview, .recentActivity, .revert])
        XCTAssertFalse(request.draftsPlans)
        XCTAssertTrue(self.request([.query, .add]).draftsPlans)
    }

    func testToolsThatSaveNeedWrites() {
        XCTAssertEqual(request([.undo], writes: false).allowedTools, [.recentActivity])
        XCTAssertTrue(request([], writes: false).allowedTools.allSatisfy { !$0.writes })
    }

    func testTheInstructionsIncludeEachSkillAndItsNote() {
        var request = request([.tidy])
        request.skills[0].note = "only the HOME Avenue"
        let text = AssistantPrompt.instructions(request)
        XCTAssertTrue(text.contains(Skill.tidy.instructions))
        XCTAssertTrue(text.contains("only the HOME Avenue"))
        XCTAssertFalse(text.contains("revert"))
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

    func testAConversationUsesOnlySkillsThatExistAndAreOn() {
        var conversation = Conversation(id: UUID(), createdAt: now)
        conversation.skills = [.init(id: "tidy", note: "HOME"), .init(id: "query"), .init(id: "user:gone")]
        var query = Skill.query
        query.isOn = false
        let uses = conversation.uses(of: [query, .tidy])
        XCTAssertEqual(uses, [SkillUse(.tidy, note: "HOME")])
    }

    func testAConversationSavedBeforeSkillsHasNone() throws {
        var conversation = Conversation(id: UUID(), createdAt: now)
        conversation.messages = [ChatMessage(id: UUID(), role: .person, text: "Hi", at: now)]
        var object = try XCTUnwrap(
            JSONSerialization.jsonObject(with: JSONEncoder().encode(conversation)) as? [String: Any])
        object["skills"] = nil
        let old = try JSONSerialization.data(withJSONObject: object)

        let decoded = try JSONDecoder().decode(Conversation.self, from: old)
        XCTAssertEqual(decoded, conversation)
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
