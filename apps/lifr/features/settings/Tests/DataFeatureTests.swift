import ComposableArchitecture
import Foundation
import LifrBackupClient
import LifrCore
import LifrSettings
import LifrStoreClient
import XCTest

@MainActor
final class DataFeatureTests: XCTestCase {
    /// 2026-11-03 10:00 UTC, a Tuesday.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)

    /// A Home Avenue with "Post the parcel" and "Buy stamps", and an export
    /// of it made before "Buy stamps" was added.
    private func records() -> (before: Workspace, after: Workspace, home: Avenue) {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        let home = records.avenue("HOME", "Home")
        _ = records.task("Post the parcel", in: home)
        let before = records.workspace
        _ = records.task("Buy stamps", in: home)
        return (before, records.workspace, home)
    }

    private func makeStore(_ workspace: Workspace) -> TestStoreOf<DataFeature> {
        let store = TestStore(initialState: DataFeature.State(workspace: workspace)) {
            DataFeature()
        } withDependencies: { [now] in
            $0.store = .inMemory(workspace)
            $0.backups = BackupClient.inMemory().client
            $0.date.now = now
            $0.timeZone = .gmt
            $0.uuid = UUIDGenerator { UUID() }
            $0.withRandomNumberGenerator = WithRandomNumberGenerator(SystemRandomNumberGenerator())
        }
        store.exhaustivity = .off
        return store
    }

    func testImportingAnExportShowsWhatItAddsAndAddsOnlyThatOnceConfirmed() async throws {
        let (before, after, _) = records()
        let store = makeStore(before)
        let data = try after.exported(at: now).encoded()
        await store.send(.fileRead(source: "Lifr.json", data, isCSV: false))
        XCTAssertEqual(store.state.preview?.plan.count(of: .task), 1)
        XCTAssertEqual(store.state.workspace.tasks.count, 1)

        await store.send(.importConfirmed)
        await store.receive(\.saved)
        await store.receive(\.delegate.saved)
        XCTAssertEqual(Set(store.state.workspace.tasks.values.map(\.title)), ["Post the parcel", "Buy stamps"])
        XCTAssertEqual(store.state.imported?.count, 1)
    }

    func testUndoImportPutsWhatTheImportAddedInTheTrash() async throws {
        let (before, after, _) = records()
        let store = makeStore(before)
        await store.send(.fileRead(source: "Lifr.json", try after.exported(at: now).encoded(), isCSV: false))
        await store.send(.importConfirmed)
        await store.receive(\.saved)

        await store.send(.undoTapped)
        await store.receive(\.undone)
        // Undo puts imported tasks in the Trash, as undoing any change does.
        let live = store.state.workspace.tasks.values.filter { $0.deletedAt == nil }
        XCTAssertEqual(live.map(\.title), ["Post the parcel"])
        XCTAssertNil(store.state.imported)
    }

    func testCancellingAnImportChangesNothing() async throws {
        let (before, after, _) = records()
        let store = makeStore(before)
        await store.send(.fileRead(source: "Lifr.json", try after.exported(at: now).encoded(), isCSV: false))
        await store.send(.importCancelled)
        XCTAssertNil(store.state.preview)
        XCTAssertEqual(store.state.workspace, before)
    }

    func testAFileThatIsntLifrsShowsWhyAndImportsNothing() async {
        let (before, _, _) = records()
        let store = makeStore(before)
        await store.send(.fileRead(source: "notes.json", Data("[1, 2]".utf8), isCSV: false))
        XCTAssertNotNil(store.state.alert)
        XCTAssertNil(store.state.preview)
    }

    func testACSVFileMakesATaskPerRowInThePickedAvenue() async {
        let (before, _, home) = records()
        let store = makeStore(before)
        let csv = Data("Task,Notes\nCall the bank,About the card\nWater plants,\n".utf8)
        await store.send(.fileRead(source: "todo.csv", csv, isCSV: true))
        guard var mapping = store.state.csv?.mapping else { return XCTFail("no CSV sheet") }
        XCTAssertEqual(mapping.title, 0)
        XCTAssertEqual(mapping.details, 1)
        mapping.avenueID = home.id
        await store.send(.csvMapped(mapping))

        await store.send(.csvConfirmed)
        await store.receive(\.saved)
        let added = store.state.workspace.tasks.values.filter { $0.title != "Post the parcel" }
        XCTAssertEqual(Set(added.map(\.title)), ["Call the bank", "Water plants"])
        XCTAssertEqual(Set(added.map(\.avenueID)), [home.id])
        XCTAssertEqual(added.first { $0.title == "Call the bank" }?.details, "About the card")
    }

    func testABadCSVRowKeepsTheSheetSoAnotherColumnCanBePicked() async {
        let (before, _, _) = records()
        let store = makeStore(before)
        await store.send(.fileRead(source: "todo.csv", Data("Title,Due\nRun,soon\n".utf8), isCSV: true))
        await store.send(.csvConfirmed)
        XCTAssertNotNil(store.state.alert)
        XCTAssertNotNil(store.state.csv)
        XCTAssertEqual(store.state.workspace, before)
    }

    func testExportingEverythingMakesAFileThatReadsBackAsTheRecords() async throws {
        let (_, after, _) = records()
        let store = makeStore(after)
        await store.send(.exportTapped)
        guard case .everything(_, let data) = store.state.export else { return XCTFail("nothing to export") }
        let export = try LifrExport.decoded(from: data)
        XCTAssertEqual(export.records.count { if case .task = $0 { true } else { false } }, 2)
    }

    func testExportingAsMarkdownMakesAFilePerAvenue() async {
        let (_, after, _) = records()
        let store = makeStore(after)
        await store.send(.markdownTapped)
        guard case .markdown(let files) = store.state.export else { return XCTFail("nothing to export") }
        XCTAssertEqual(files.count, after.avenues.count)
        XCTAssertNotNil(files["Home.md"])
    }
}
