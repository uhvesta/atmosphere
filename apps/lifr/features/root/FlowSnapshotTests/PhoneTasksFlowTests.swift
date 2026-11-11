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

    /// Picking several tasks in Today: pick two, give them a priority, then
    /// pick all, trash them and Undo that.
    func testBulkEdit() async throws {
        let (store, flow) = app("bulk-edit")
        await flow.step("launch", image: false)
        store.send(.today(.list(.selectTapped)))
        await flow.step("picking")
        let rows = store.today.list.rows.map(\.id)
        let first = try XCTUnwrap(rows.first)
        let last = try XCTUnwrap(rows.last)
        store.send(.today(.list(.rowTapped(first))))
        store.send(.today(.list(.toggled(last))))
        await flow.step("two-picked")
        store.send(.today(.list(.bulk(.priority(.high)))))
        await flow.step("prioritized")
        XCTAssertEqual(store.workspace?.tasks[first]?.priority, .high)
        XCTAssertEqual(store.workspace?.tasks[last]?.priority, .high)
        XCTAssertFalse(store.today.list.isSelecting)
        store.send(.today(.list(.selectTapped)))
        store.send(.today(.list(.selectAllTapped)))
        await flow.step("all-picked")
        store.send(.today(.list(.bulk(.trash))))
        await flow.step("trashed")
        let trashed = try XCTUnwrap(store.toast?.id)
        store.send(.undoTapped(trashed))
        await flow.step("restored")
        XCTAssertEqual(store.today.list.rows.map(\.id), rows)
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
        let closed = try XCTUnwrap(store.logbook.rows.first)
        let word = try XCTUnwrap(closed.title.split(separator: " ").last.map(String.init))
        store.send(.logbook(.searchChanged(word.lowercased())))
        await flow.step("logbook-search")
        XCTAssertEqual(store.logbook.rows.first?.id, closed.id)
        store.send(.logbook(.searchChanged("zzz")))
        await flow.step("logbook-no-results")
        XCTAssertTrue(store.logbook.isEmpty)
        flow.finish()
    }

    /// A search saved as a filter: it's listed on the Avenues tab, opens
    /// with its tasks, can be updated, and is deleted only after asking.
    func testSavedFilters() async throws {
        let (store, flow) = app("saved-filters")
        store.tab = .search
        store.send(.search(.binding(.set(\.filter.priorities, [.high]))))
        await flow.step("search")
        store.send(.search(.saveFilterTapped))
        store.send(.search(.editor(.presented(.binding(.set(\.name, "Urgent"))))))
        store.send(.search(.editor(.presented(.binding(.set(\.symbol, "bolt"))))))
        await flow.step("save")
        store.send(.search(.editor(.presented(.saveTapped))))
        await flow.step("saved", image: false)
        let urgent = try XCTUnwrap(store.workspace?.sortedSavedFilters.first?.id)
        store.tab = .avenues
        await flow.step("avenues")
        store.send(.savedFilterTapped(urgent))
        await flow.step("opened")
        store.send(.savedFilter(.presented(.binding(.set(\.filter.due, .nextSevenDays)))))
        await flow.step("changed")
        store.send(.savedFilter(.presented(.updateSavedFilterTapped)))
        await flow.step("updated", image: false)
        XCTAssertEqual(store.workspace?.savedFilters[urgent]?.filter.due, .nextSevenDays)
        store.send(.savedFilter(.presented(.editSavedFilterTapped)))
        await flow.step("edit")
        store.send(.savedFilter(.presented(.editor(.presented(.deleteTapped)))))
        await flow.step("delete-asks")
        store.send(.savedFilter(.presented(.editor(.presented(.confirm(.presented(.delete)))))))
        await flow.step("deleted")
        XCTAssertNil(store.savedFilter)
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
