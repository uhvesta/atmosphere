import AssistantKit

// A small notes assistant to test skills on: two tools that read and one
// that writes, and two built-in skills.

enum NoteTool: String, SkillTool {
    case find, read, edit

    var writes: Bool { self == .edit }

    static var builtInSkills: [Skill<NoteTool>] {
        [
            Skill(
                id: "ask", summary: "Answer questions about your notes.", instructions: "Answer from the notes.",
                tools: [.find, .read], draftsPlans: false, autoPicks: true, useWhen: "a question"),
            Skill(
                id: "tidy", summary: "Clean up titles.", instructions: "Suggest clearer titles.",
                tools: [.find, .edit], draftsPlans: false),
        ]
    }
}

typealias NoteSkill = Skill<NoteTool>
