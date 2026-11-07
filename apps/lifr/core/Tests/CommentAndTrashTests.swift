import Foundation
import LifrCore
import TestingKit

func commentAndTrashTests() -> [TestCase] {
    [
        TestCase("given a comment by the user, then an agent can't edit or delete it") {
            let fixture = try Fixture()
            let task = try fixture.task("Task")
            let comment = try fixture.workspace.addComment("Mine", to: task.id, in: fixture.context)
            fixture.actor = .agent("Claude Code", device: "Mac", surface: .mcp)
            try expectError(.notCommentAuthor(comment: comment.id)) {
                try fixture.workspace.editComment(comment.id, body: "Changed", in: fixture.context)
            }
            try expectError(.notCommentAuthor(comment: comment.id)) {
                try fixture.workspace.deleteComment(comment.id, in: fixture.context)
            }
        },
        TestCase("given a comment by an agent, then only that agent can edit it") {
            let fixture = try Fixture()
            let task = try fixture.task("Task")
            fixture.actor = .agent("Claude Code", device: "Mac", surface: .mcp)
            let comment = try fixture.workspace.addComment("Agent's", to: task.id, in: fixture.context)
            try fixture.workspace.editComment(comment.id, body: "Edited", in: fixture.context)
            try expectEqual(fixture.workspace.comments[comment.id]?.body, "Edited")
            fixture.actor = .agent("Other agent", device: "Mac", surface: .mcp)
            try expectError(.notCommentAuthor(comment: comment.id)) {
                try fixture.workspace.deleteComment(comment.id, in: fixture.context)
            }
        },
        TestCase("given a deleted comment, then it stays as a marker and history records it") {
            let fixture = try Fixture()
            let task = try fixture.task("Task")
            let comment = try fixture.workspace.addComment("Oops", to: task.id, in: fixture.context)
            try fixture.workspace.deleteComment(comment.id, in: fixture.context)
            try expectEqual(fixture.workspace.comments[comment.id]?.deletedAt, fixture.now)
            let kinds = fixture.workspace.history.filter { $0.taskID == task.id }.map(\.kind)
            try expectEqual(kinds, [.created, .commentAdded, .commentDeleted])
        },
        TestCase("given an empty comment, then it's refused") {
            let fixture = try Fixture()
            let task = try fixture.task("Task")
            try expectError(.emptyComment) { try fixture.workspace.addComment(" \n", to: task.id, in: fixture.context) }
        },
        TestCase("given a parent in the trash, then its Sub tasks are too, and restore brings both back") {
            let fixture = try Fixture()
            let parent = try fixture.task("Parent")
            let subtask = try fixture.task("Sub task", parent: parent)
            _ = try fixture.workspace.moveToTrash(parent.id, in: fixture.context)
            try expectEqual(fixture.get(subtask).deletedAt, fixture.now)
            try expectError(.inTrash(parent.id)) {
                try fixture.workspace.setStatus(.done, of: parent.id, in: fixture.context)
            }
            _ = try fixture.workspace.restore(parent.id, in: fixture.context)
            try expectEqual(fixture.get(subtask).deletedAt, nil)
        },
        TestCase("given a task trashed 31 days ago, then purging deletes it with its comments and history") {
            let fixture = try Fixture()
            let kept = try fixture.task("Kept")
            let trashed = try fixture.task("Trashed")
            _ = try fixture.workspace.addComment("Bye", to: trashed.id, in: fixture.context)
            _ = try fixture.workspace.moveToTrash(trashed.id, in: fixture.context)
            fixture.advance(days: 29)
            fixture.workspace.purgeTrash(now: fixture.now)
            try expect(fixture.workspace.tasks[trashed.id] != nil, "purged before 30 days")
            fixture.advance(days: 2)
            fixture.workspace.purgeTrash(now: fixture.now)
            try expect(fixture.workspace.tasks[trashed.id] == nil, "not purged after 30 days")
            try expect(fixture.workspace.comments.isEmpty)
            try expect(fixture.workspace.history.allSatisfy { $0.taskID == kept.id })
        },
    ]
}
