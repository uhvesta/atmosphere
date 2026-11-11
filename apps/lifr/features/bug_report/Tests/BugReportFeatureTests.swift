import ComposableArchitecture
import Foundation
import LifrModelsClient
import LifrPasteboardClient
import XCTest

@testable import LifrBugReport

@MainActor
final class BugReportFeatureTests: XCTestCase {
    func testANewReportSaysWhichDeviceAndModelItCameFrom() {
        let state = withDependencies {
            $0.uuid = .incrementing
            $0.date.now = Date(timeIntervalSince1970: 5)
            $0.models.selection = { _ in "gemma-4-e2b" }
        } operation: {
            BugReportFeature.State.new(recording: nil)
        }

        XCTAssertEqual(state.report.device.assistantModel, "gemma-4-e2b")
        XCTAssertEqual(state.report.device.model, "iPhone18,1")
        XCTAssertEqual(state.report.createdAt, Date(timeIntervalSince1970: 5))
        XCTAssertFalse(state.report.includesPersonal)
    }

    func testSavingKeepsTheReportWithoutTheAppsTextAndCloses() async throws {
        let reports = BugReportsClient.inMemory()
        let dismissed = LockIsolated(false)
        var report = BugReport.sample(includesPersonal: false)
        report.happened = ""
        let store = TestStore(initialState: BugReportFeature.State(report: report)) {
            BugReportFeature()
        } withDependencies: {
            $0.bugReports = reports
            $0.dismiss = DismissEffect { dismissed.setValue(true) }
        }

        await store.send(.saveTapped)
        await store.send(.binding(.set(\.report.happened, "The task vanished"))) {
            $0.report.happened = "The task vanished"
        }
        await store.send(.saveTapped) { $0.isSaving = true }
        await store.receive(\.saved) { $0.isSaving = false }

        let saved = try await reports.load().reports
        XCTAssertEqual(saved, [Redactor.redact(BugReport.sample(includesPersonal: false))])
        XCTAssertEqual(saved.first?.recording?.screenshots, [])
        XCTAssertTrue(dismissed.value)
    }

    func testAFailedSaveSaysWhyAndStaysOpen() async {
        let failure = BugReportError.fileSystem(operation: .write, path: "BugReports", reason: "disk full")
        let store = TestStore(initialState: BugReportFeature.State(report: .sample(includesPersonal: true))) {
            BugReportFeature()
        } withDependencies: {
            $0.bugReports.save = { _ in throw failure }
        }

        await store.send(.saveTapped) { $0.isSaving = true }
        await store.receive(\.saved) {
            $0.isSaving = false
            $0.error = failure
        }
    }

    func testCopyingPutsTheKeptReportOnTheClipboard() async {
        let copied = LockIsolated<String?>(nil)
        let store = TestStore(initialState: BugReportFeature.State(report: .sample(includesPersonal: false))) {
            BugReportFeature()
        } withDependencies: {
            $0.pasteboard = PasteboardClient { copied.setValue($0) }
        }

        await store.send(.copyTapped) { $0.copied = true }

        XCTAssertEqual(copied.value, Redactor.redact(.sample(includesPersonal: false)).text)
        XCTAssertFalse(copied.value?.contains("Buy milk") ?? true)
        XCTAssertTrue(copied.value?.contains("The task vanished") ?? false)
    }

    func testCancellingClosesWithoutKeepingTheReport() async throws {
        let reports = BugReportsClient.inMemory()
        let dismissed = LockIsolated(false)
        let store = TestStore(initialState: BugReportFeature.State(report: .sample(includesPersonal: false))) {
            BugReportFeature()
        } withDependencies: {
            $0.bugReports = reports
            $0.dismiss = DismissEffect { dismissed.setValue(true) }
        }

        await store.send(.cancelTapped)

        XCTAssertTrue(dismissed.value)
        let saved = try await reports.load().reports
        XCTAssertEqual(saved, [])
    }
}

@MainActor
final class BugReportsFeatureTests: XCTestCase {
    func testTheListOpensCopiesAndDeletesSavedReports() async throws {
        let report = BugReport.sample(includesPersonal: true, id: UUID(1))
        let reports = BugReportsClient.inMemory([report])
        let copied = LockIsolated<String?>(nil)
        let store = TestStore(initialState: BugReportsFeature.State()) {
            BugReportsFeature()
        } withDependencies: {
            $0.bugReports = reports
            $0.pasteboard = PasteboardClient { copied.setValue($0) }
        }

        await store.send(.task)
        await store.receive(\.loaded.success) {
            $0.saved = SavedReports(reports: [report])
            $0.isLoaded = true
        }
        await store.send(.openTapped(UUID(1))) { $0.opened = report }
        await store.send(.copyTapped(UUID(1))) { $0.copied = UUID(1) }
        XCTAssertEqual(copied.value, report.text)
        await store.send(.deleteTapped(UUID(1)))
        await store.receive(\.deleted) {
            $0.saved = SavedReports()
            $0.opened = nil
        }

        let left = try await reports.load()
        XCTAssertEqual(left, SavedReports())
    }
}
