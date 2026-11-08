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
}
