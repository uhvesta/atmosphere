import AssistantKit
import Foundation
import TestingKit

func skillTests() -> [TestCase] {
    [
        TestCase("given a name, then the person's skill gets an ID from its words, or none without any") {
            let skill = try NoteSkill(
                userNamed: "Weekly Review!", summary: "", instructions: "Go", tools: [], draftsPlans: false
            ).unwrap()
            try expectEqual(skill.id, "user:weekly-review")
            try expect(skill.isUser)
            try expectEqual(
                NoteSkill(userNamed: " … ", summary: "", instructions: "Go", tools: [], draftsPlans: false), nil)
        },
        TestCase("given stored skills, then built-ins keep the app's instructions with the stored on and off") {
            var tidy = NoteTool.builtInSkills[1]
            tidy.instructions = "Old instructions"
            tidy.isOn = false
            let mine = try NoteSkill(
                userNamed: "Mine", summary: "", instructions: "Go", tools: [.read], draftsPlans: false
            ).unwrap()
            let merged = NoteSkill.merged(stored: [mine, tidy])
            try expectEqual(merged.map(\.id), ["ask", "tidy", "user:mine"])
            try expectEqual(merged[1].instructions, "Suggest clearer titles.")
            try expectEqual(merged[1].isOn, false)
        },
        TestCase("given a built-in saved before automatic picking, then it reads with the app's default") {
            let json = """
                {"id":"ask","summary":"s","instructions":"i","tools":["find"],"draftsPlans":false,"isOn":true}
                """
            let read = try JSONDecoder().decode(NoteSkill.self, from: Data(json.utf8))
            try expect(read.autoPicks)
            try expectEqual(read.useWhen, "a question")
        },
        TestCase("given a query, then titles starting with it come first, then letters in order, then summaries") {
            let skills = NoteTool.builtInSkills
            try expectEqual(NoteSkill.search("t", in: skills).map(\.id), ["tidy", "ask"])
            try expectEqual(NoteSkill.search("ak", in: skills).map(\.id), ["ask"])
            try expectEqual(NoteSkill.search("clean titles", in: skills).map(\.id), ["tidy"])
            try expectEqual(NoteSkill.search("", in: skills).map(\.id), ["ask", "tidy"])
        },
        TestCase("given skills, then only those on and marked can be picked, by a name in any case") {
            var skills = NoteTool.builtInSkills
            try expectEqual(NoteSkill.pickable(skills).map(\.id), ["ask"])
            skills[0].isOn = false
            try expectEqual(NoteSkill.pickable(skills), [])
            try expectEqual(NoteSkill.named(" TIDY", in: skills)?.id, "tidy")
            try expectEqual(NoteSkill.named("other", in: skills), nil)
        },
        TestCase("given a skill with a tool that writes, then it writes") {
            try expect(NoteTool.builtInSkills[1].writes)
            try expect(!NoteTool.builtInSkills[0].writes)
        },
    ]
}
