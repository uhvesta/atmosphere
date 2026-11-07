import LifrCore
import XCTest

/// The requests people make most, on the real model, each checked by what
/// got saved rather than by the model's words. The draft is added, and
/// changes to existing tasks applied, as the model left them: anything the
/// person would have to fix is a failure.
///
/// The sample records are as of Tuesday 2026-11-03, so Friday is
/// 2026-11-06 and next Monday 2026-11-09.
@MainActor
final class RequestsOnRealModelTests: XCTestCase {
    private let friday = "2026-11-06"

    // MARK: - Adding

    func testOneTask() async throws {
        let model = try RealModel()
        try await model.chat()
        try await model.send("Add a task to call my mum, in HOME")
        let added = try await model.addDraft()
        XCTAssertEqual(added.count, 1)
        let task = try XCTUnwrap(added.first)
        XCTAssertTrue(task.title.localizedCaseInsensitiveContains("mum"), task.title)
        XCTAssertEqual(try model.avenue(of: task), "HOME")
        XCTAssertNil(task.dueDate, "A date the person didn't ask for")
    }

    func testSeveralTasks() async throws {
        let model = try RealModel()
        try await model.chat()
        try await model.send("Add three tasks to Home: vacuum the living room, take out the bins, clean the windows")
        let added = try await model.addDraft()
        XCTAssertEqual(added.count, 3)
        for words in ["vacuum", "bins", "windows"] {
            let task = try XCTUnwrap(
                added.first { $0.title.localizedCaseInsensitiveContains(words) }, "No task with “\(words)”")
            XCTAssertEqual(try model.avenue(of: task), "HOME")
        }
    }

    func testATaskDueFriday() async throws {
        let model = try RealModel()
        try await model.chat()
        try await model.send("Add a task to pay the rent, due Friday, in HOME")
        let added = try await model.addDraft()
        XCTAssertEqual(added.count, 1)
        XCTAssertEqual(added.first?.dueDate?.description, friday)
    }

    func testAnUrgentTask() async throws {
        let model = try RealModel()
        try await model.chat()
        try await model.send("Add an urgent task to file my taxes, in WORK")
        let added = try await model.addDraft()
        XCTAssertEqual(added.count, 1)
        XCTAssertEqual(added.first?.priority, .urgent)
        XCTAssertEqual(try added.first.flatMap(model.avenue), "WORK")
    }

    func testATaskWithSubTasks() async throws {
        let model = try RealModel()
        try await model.chat()
        try await model.send(
            "Add a task to plan the team offsite in WORK, with the sub tasks book a venue and send the invites")
        let added = try await model.addDraft()
        let parent = try XCTUnwrap(added.first { $0.parentID == nil }, "No top-level task")
        XCTAssertTrue(parent.title.localizedCaseInsensitiveContains("offsite"), parent.title)
        let subtasks = added.filter { $0.parentID == parent.id }
        XCTAssertEqual(subtasks.count, 2, "Sub tasks: \(subtasks.map(\.title))")
        XCTAssertEqual(added.count, 3)
    }

    func testATaskInAProject() async throws {
        let model = try RealModel()
        try await model.chat()
        try await model.send("Add a task to stretch after every run to my Half marathon project")
        let added = try await model.addDraft()
        XCTAssertEqual(added.count, 1)
        let task = try XCTUnwrap(added.first)
        XCTAssertEqual(try model.avenue(of: task), "HEALTH")
        XCTAssertEqual(try task.projectID.flatMap { try model.workspace.projects[$0]?.name }, "Half marathon")
    }

    func testANewAvenueWithTasks() async throws {
        let model = try RealModel()
        try await model.chat()
        try await model.send("Make a new Avenue called Garden, with the tasks plant tulips and prune the roses")
        let added = try await model.addDraft()
        XCTAssertEqual(added.count, 2, "Added: \(added.map(\.title))")
        let garden = try XCTUnwrap(
            try model.workspace.avenues.values.first { $0.name.localizedCaseInsensitiveContains("garden") },
            "No Garden Avenue")
        XCTAssertTrue(added.allSatisfy { $0.avenueID == garden.id }, "Tasks outside Garden")
    }

    // MARK: - Follow-ups

    /// "Also add…", "change that to Friday" and "remove the second one",
    /// each answered with the whole draft.
    func testFollowUps() async throws {
        let model = try RealModel()
        try await model.chat()
        try await model.send("Add a task to buy milk, in HOME")
        XCTAssertEqual(model.draft?.taskCount, 1)

        try await model.send("Also add buy eggs")
        let both = try XCTUnwrap(model.draft)
        XCTAssertEqual(both.taskCount, 2, "Draft: \(both.tasks.map(\.title))")

        try await model.send("Change that to Friday")
        let eggs = try XCTUnwrap(model.draft?.tasks.first { $0.title.localizedCaseInsensitiveContains("egg") })
        XCTAssertEqual(eggs.due.flatMap { CalendarDate.resolving($0, today: today) }?.description, friday)
        XCTAssertEqual(model.draft?.taskCount, 2)

        try await model.send("Remove the second one")
        let added = try await model.addDraft()
        XCTAssertEqual(added.map(\.title).count, 1, "Added: \(added.map(\.title))")
        XCTAssertTrue(added.first?.title.localizedCaseInsensitiveContains("milk") ?? false)
    }

    // MARK: - Changing existing tasks

    func testRenamingATask() async throws {
        let model = try RealModel()
        try await model.chat()
        let plumber = try model.task(titled: "Call the plumber")
        try await model.send("Rename “Call the plumber” to “Call the electrician”")
        try await model.applyProposal()
        await model.settled { try model.workspace.tasks[plumber.id]?.title == "Call the electrician" }
        XCTAssertEqual(try model.workspace.tasks[plumber.id]?.title, "Call the electrician")
        XCTAssertNil(model.draft, "A draft for a rename")
    }

    func testMovingATask() async throws {
        let model = try RealModel()
        try await model.chat()
        let passport = try model.task(titled: "Renew passport")
        try await model.send("Move “Renew passport” to WORK")
        try await model.applyProposal()
        await model.settled { try model.workspace.tasks[passport.id]?.avenueID != passport.avenueID }
        let moved = try XCTUnwrap(try model.workspace.tasks[passport.id])
        XCTAssertEqual(try model.avenue(of: moved), "WORK")
        XCTAssertNil(model.draft, "A draft for a move")
    }

    func testFinishingATask() async throws {
        let model = try RealModel()
        try await model.chat()
        let plants = try model.task(titled: "Water the plants")
        try await model.send("I watered the plants, mark it done")
        try await model.applyProposal()
        await model.settled { try model.workspace.tasks[plants.id]?.status == .done }
        XCTAssertEqual(try model.workspace.tasks[plants.id]?.status, .done)
    }

    func testReschedulingATask() async throws {
        let model = try RealModel()
        try await model.chat()
        let physio = try model.task(titled: "Book physio appointment")
        try await model.send("Push “Book physio appointment” to next Monday")
        try await model.applyProposal()
        await model.settled { try model.workspace.tasks[physio.id]?.dueDate != physio.dueDate }
        XCTAssertEqual(try model.workspace.tasks[physio.id]?.dueDate?.description, "2026-11-09")
    }

    private var today: CalendarDate { CalendarDate(RealModel.now, in: .gmt) }
}
