import Foundation
import LifrAPI
import LifrAssistantClient
import LifrCore
import LifrStoreClient
import XCTest

final class SkillTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_793_700_000)

    private func request(_ skill: Skill?, writes: Bool = true) -> AssistantRequest {
        AssistantRequest(
            messages: [], workspace: Workspace(), now: now, timeZone: .gmt,
            writes: writes ? AssistantWrites(conversation: UUID()) : nil, skill: skill)
    }

    func testWithoutASkillTheAssistantJustTalks() {
        XCTAssertEqual(request(nil).allowedTools, [])
        XCTAssertFalse(request(nil).draftsPlans)
        let instructions = AssistantPrompt.chatInstructions(request(nil))
        XCTAssertTrue(instructions.contains(Guidance.defaultText))
        XCTAssertTrue(instructions.contains("2026-11-03"))
        XCTAssertFalse(instructions.contains("Avenue"))
    }

    func testWithoutASkillThePromptIsOnlyTheConversation() {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        records.task("Water the plants", in: records.avenue("HOME", "Home"))
        var request = request(nil)
        request.workspace = records.workspace
        request.messages = [ChatMessage(id: UUID(), role: .person, text: "Water the plants?", at: now)]
        request.draft = Plan(tasks: [.init(title: "Buy milk", place: "HOME")])

        let prompt = AssistantPrompt.prompt(request)

        XCTAssertTrue(prompt.contains("Water the plants?"))
        XCTAssertFalse(prompt.contains("HOME-"))
        XCTAssertFalse(prompt.contains("Buy milk"))
    }

    func testASkillAllowsOnlyItsTools() {
        XCTAssertEqual(request(.undo).allowedTools, [.recentActivity, .revert])
        XCTAssertFalse(request(.undo).draftsPlans)
        XCTAssertTrue(request(.add).draftsPlans)
    }

    func testToolsThatSaveNeedWrites() {
        XCTAssertEqual(request(.undo, writes: false).allowedTools, [.recentActivity])
        XCTAssertTrue(request(.tidy, writes: false).allowedTools.allSatisfy { !$0.writes })
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

    func testStoredBuiltInsOnlyKeepWhetherTheyreOnAndPickedAutomatically() {
        var changed = Skill.query
        changed.instructions = "Something else"
        changed.useWhen = "never"
        changed.isOn = false
        changed.autoPicks = false
        let merged = Skill.merged(stored: [changed])
        XCTAssertEqual(merged.map(\.id), Skill.builtIn.map(\.id))
        XCTAssertEqual(merged[0].instructions, Skill.query.instructions)
        XCTAssertEqual(merged[0].useWhen, Skill.query.useWhen)
        XCTAssertFalse(merged[0].isOn)
        XCTAssertFalse(merged[0].autoPicks)
    }

    func testASkillSavedBeforeAutomaticPickingTakesTheBuiltInsDefault() throws {
        func saved(_ id: String) -> Data {
            Data(
                #"{"id":"\#(id)","summary":"","instructions":"Go.","tools":[],"draftsPlans":false,"isOn":true}"#
                    .utf8)
        }
        let breakdown = try JSONDecoder().decode(Skill.self, from: saved("breakdown"))
        let undo = try JSONDecoder().decode(Skill.self, from: saved("undo"))
        let mine = try JSONDecoder().decode(Skill.self, from: saved("user:mine"))

        XCTAssertTrue(breakdown.autoPicks)
        XCTAssertEqual(breakdown.useWhen, Skill.breakdown.useWhen)
        XCTAssertFalse(undo.autoPicks)
        XCTAssertFalse(mine.autoPicks)
        XCTAssertEqual(mine.useWhen, "")
    }

    func testASkillKeepsWhetherItsPickedAutomaticallyWhenSaved() throws {
        let mine = try XCTUnwrap(
            Skill(
                userNamed: "Weekly review", summary: "Fridays.", instructions: "Go.", tools: [], draftsPlans: false,
                autoPicks: true, useWhen: "I ask about my week"))
        let decoded = try JSONDecoder().decode(Skill.self, from: JSONEncoder().encode(mine))
        XCTAssertEqual(decoded, mine)
    }

    // MARK: - Picking

    func testThePickIsOneOfTheSkillsOrNone() {
        let skills = [Skill.query, .breakdown]
        XCTAssertEqual(
            AssistantPrompt.pickedSkill(from: .object(["skill": .string("breakdown")]), in: skills), "breakdown")
        XCTAssertEqual(AssistantPrompt.pickedSkill(from: .object(["skill": .string(" Query ")]), in: skills), "query")
        XCTAssertNil(
            AssistantPrompt.pickedSkill(from: .object(["skill": .string(AssistantPrompt.noSkill)]), in: skills))
        XCTAssertNil(AssistantPrompt.pickedSkill(from: .object(["skill": .string("undo")]), in: skills))
        XCTAssertNil(AssistantPrompt.pickedSkill(from: .object([:]), in: skills))
    }

    func testThePickSeesEachSkillsUseAndTheSkillOfEarlierMessages() throws {
        let mine = try XCTUnwrap(
            Skill(userNamed: "Mine", summary: "My summary.", instructions: "Go.", tools: [], draftsPlans: false))
        let instructions = AssistantPrompt.pickInstructions([.breakdown, mine])
        XCTAssertTrue(instructions.contains(Skill.breakdown.useWhen))
        XCTAssertTrue(instructions.contains("My summary."), "without Use When, the summary")

        let prompt = AssistantPrompt.pickPrompt([
            ChatMessage(id: UUID(), role: .person, text: "Dentist, taxes, gym", at: now, skill: "breakdown"),
            ChatMessage(id: UUID(), role: .assistant, text: "Which is most urgent?", at: now),
            ChatMessage(id: UUID(), role: .person, text: "Taxes", at: now),
        ])
        XCTAssertTrue(prompt.contains("breakdown"))
        XCTAssertTrue(prompt.contains("Taxes"))
    }

    // MARK: - Built in

    func testOnlyAddMustPlanAndOnlyChangeMustChange() {
        XCTAssertTrue(AssistantPrompt.asksForPlan(request(.add)))
        XCTAssertFalse(AssistantPrompt.asksForPlan(request(.breakdown)), "Break down asks first")
        XCTAssertFalse(AssistantPrompt.asksForPlan(request(nil)))
        XCTAssertTrue(request(.breakdown).draftsPlans)
        XCTAssertTrue(request(.change).changesTasks)
        XCTAssertFalse(request(.change).draftsPlans)
    }

    func testBreakDownKeepsTheProjectsItPlans() {
        let plan = Plan(
            projects: [.init(name: "Move house", avenue: "HOME", tasks: [.init(title: "Book a van")])])
        var request = request(.breakdown)
        request.messages = [ChatMessage(id: UUID(), role: .person, text: "Move house: book a van", at: now)]

        XCTAssertEqual(AssistantPrompt.withAskedProjects(plan, request), plan)
        XCTAssertTrue(AssistantPrompt.instructions(request).contains("Make a Project where several tasks"))
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
