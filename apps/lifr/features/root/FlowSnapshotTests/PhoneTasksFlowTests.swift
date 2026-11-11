import ComposableArchitecture
import FlowKit
import LifrAPI
import LifrAssistantClient
import LifrAssistantLive
import LifrBugReport
import LifrConversations
import LifrCore
import LifrGlance
import LifrGlanceClient
import LifrLockClient
import LifrModelsClient
import LifrRemindersClient
import LifrRoot
import LifrSearchClient
import LifrSessionClient
import LifrShortcuts
import LifrStoreClient
import LifrSyncClient
import LifrWidgetViews
import SwiftUI
import XCTest

/// Tasks and projects on the iPhone: a task's details, Quick Add, avenues and a project.
@MainActor
final class PhoneTasksFlowTests: PhoneFlowTestCase {
    /// A task's detail: its fields, adding a Sub task, a comment.
    func testTaskDetail() async throws {
        let (store, flow) = app("detail")
        await flow.step("launch", image: false)
        let review = try task("Review Q4 roadmap", in: store)
        store.send(.today(.list(.rowTapped(review))))
        await flow.step("opened")
        store.send(.today(.list(.detail(.presented(.binding(.set(\.newSubtaskTitle, "Book a room")))))))
        store.send(.today(.list(.detail(.presented(.addSubtaskSubmitted)))))
        await flow.step("subtask-added")
        store.send(.today(.list(.detail(.presented(.statusPicked(.done))))))
        await flow.step("finish-asks")
        flow.finish()
    }

    /// Quick add: a task with an Avenue, a date and a priority.
    func testQuickAdd() async throws {
        let (store, flow) = app("quick-add")
        await flow.step("launch", image: false)
        store.send(.newTaskTapped)
        await flow.step("empty")
        let home = try XCTUnwrap(store.workspace?.avenues.values.first { $0.key.rawValue == "HOME" }?.id)
        store.send(.quickAdd(.presented(.binding(.set(\.title, "Paint the fence")))))
        store.send(.quickAdd(.presented(.binding(.set(\.avenueID, home)))))
        store.send(.quickAdd(.presented(.binding(.set(\.due, .tomorrow)))))
        store.send(.quickAdd(.presented(.binding(.set(\.priority, .high)))))
        await flow.step("filled")
        store.send(.quickAdd(.presented(.addTapped)))
        await flow.step("added")
        flow.finish()
    }

    /// Avenues: the list, one Avenue, a new Project, a new Avenue.
    func testAvenues() async throws {
        let (store, flow) = app("avenues")
        store.tab = .avenues
        await flow.step("list")
        let health = try XCTUnwrap(store.workspace?.avenues.values.first { $0.key.rawValue == "HEALTH" }?.id)
        store.send(.avenues(.opened(.avenue(health))))
        await flow.step("avenue")
        store.send(.avenues(.avenue(.presented(.newProjectTapped))))
        await flow.step("new-project")
        store.send(.avenues(.avenue(.presented(.newProjectCancelled))))
        store.send(.avenues(.newAvenueTapped))
        await flow.step("new-avenue")
        flow.finish()
    }

    /// The trash: a task moved there from its detail, the Trash under the
    /// Avenues, Empty Trash asking first, then restoring the task.
    func testTrash() async throws {
        let (store, flow) = app("trash")
        await flow.step("launch", image: false)
        let review = try task("Review Q4 roadmap", in: store)
        store.send(.today(.list(.rowTapped(review))))
        store.send(.today(.list(.detail(.presented(.trashTapped)))))
        await flow.step("trashed", image: false)
        store.tab = .avenues
        await flow.step("avenues")
        store.isTrashShown = true
        await flow.step("trash")
        store.send(.trash(.emptyTapped))
        await flow.step("empty-asks")
        store.send(.trash(.confirmation(.dismiss)))
        store.send(.trash(.restoreTapped(review)))
        await flow.step("restored")
        flow.finish()
    }

    /// Anytime, Later and the Logbook from the Avenues tab: a task closed
    /// from Anytime moves to the Logbook.
    func testShelves() async throws {
        let (store, flow) = app("shelves")
        store.tab = .avenues
        await flow.step("avenues")
        store.shownShelf = .anytime
        await flow.step("anytime")
        let card = try task("Pick up a birthday card", in: store)
        store.send(.anytime(.rowAction(card, .status(.done))))
        await flow.step("closed", image: false)
        store.shownShelf = .later
        await flow.step("later")
        store.shownShelf = .logbook
        await flow.step("logbook")
        flow.finish()
    }

    /// A Project: rename it, give it a due date, archive it with its open
    /// task moved out and Undo that, archive it again, find it under
    /// Archived, then restore it. And edit its Avenue.
    func testProject() async throws {
        let sample = Workspace.sample(now: now, timeZone: .gmt)
        let marathon = try XCTUnwrap(sample.projects.values.first { $0.name == "Half marathon" }?.id)
        let health = try XCTUnwrap(sample.avenues.values.first { $0.key.rawValue == "HEALTH" }?.id)
        let (store, flow) = app("project", workspace: sample)
        store.tab = .avenues
        await flow.step("avenues")
        store.send(.avenues(.opened(.project(marathon))))
        await flow.step("opened")
        store.send(.avenues(.avenue(.presented(.project(.binding(.set(\.name, "Spring half marathon")))))))
        store.send(.avenues(.avenue(.presented(.project(.nameSubmitted)))))
        store.send(.avenues(.avenue(.presented(.project(.dueDatePicked(try CalendarDate("2027-04-11")))))))
        await flow.step("renamed-due")
        store.send(.avenues(.avenue(.presented(.project(.archiveTapped)))))
        await flow.step("archive-asks")
        store.send(.avenues(.avenue(.presented(.project(.archive(.presented(.moveToAvenue)))))))
        await flow.step("archived")
        let archived = try XCTUnwrap(store.toast?.id)
        store.send(.undoTapped(archived))
        await flow.step("archive-undone")
        XCTAssertEqual(store.workspace?.projects[marathon]?.status, .active)
        store.send(.avenues(.avenue(.presented(.project(.archiveTapped)))))
        store.send(.avenues(.avenue(.presented(.project(.archive(.presented(.moveToAvenue)))))))
        await flow.step("archived-again", image: false)
        store.send(.avenues(.avenue(.dismiss)))
        await flow.step("archived-list")
        store.send(.avenues(.opened(.project(marathon))))
        store.send(.avenues(.avenue(.presented(.project(.restoreTapped)))))
        await flow.step("restored")
        store.send(.avenues(.opened(.avenue(health))))
        store.send(.avenues(.avenue(.presented(.editAvenueTapped))))
        await flow.step("edit-avenue")
        flow.finish()
    }

    /// Moving: a Project to another Avenue takes its task along, and a
    /// task picked into a Project of another Avenue takes its Sub tasks.
    /// Undo puts each back whole.
    func testMoves() async throws {
        let sample = Workspace.sample(now: now, timeZone: .gmt)
        let marathon = try XCTUnwrap(sample.projects.values.first { $0.name == "Half marathon" }?.id)
        let home = try XCTUnwrap(sample.avenues.values.first { $0.key.rawValue == "HOME" }?.id)
        let (store, flow) = app("moves", workspace: sample)
        store.tab = .avenues
        await flow.step("avenues", image: false)
        store.send(.avenues(.opened(.project(marathon))))
        await flow.step("project-opened")
        store.send(.avenues(.avenue(.presented(.project(.avenuePicked(home))))))
        await flow.step("project-moved")
        XCTAssertEqual(
            store.workspace?.tasks.values.filter { $0.projectID == marathon }.map(\.avenueID), [home])
        store.send(.undoTapped(try XCTUnwrap(store.toast?.id)))
        await flow.step("project-move-undone")
        store.send(.avenues(.avenue(.dismiss)))
        store.tab = .today
        let review = try task("Review Q4 roadmap", in: store)
        store.send(.today(.list(.rowTapped(review))))
        await flow.step("task-opened")
        store.send(.today(.list(.detail(.presented(.projectPicked(marathon))))))
        await flow.step("task-moved")
        XCTAssertEqual(
            store.workspace?.tasks.values.filter { $0.parentID == review }.map(\.projectID),
            [marathon, marathon, marathon])
        store.send(.undoTapped(try XCTUnwrap(store.toast?.id)))
        await flow.step("task-move-undone")
        flow.finish()
    }
}
