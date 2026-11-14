import ComposableArchitecture
import HumataDaemonClient
import HumataProjects
import XCTest

@MainActor
final class RepositorySearchTests: XCTestCase {
    private let saved = [
        SavedRepository(id: 1, name: "web-app", location: "/code/web-app", lastUsed: Date(timeIntervalSince1970: 30)),
        SavedRepository(
            id: 2, name: "humata", location: "https://github.com/acme/humata", source: .github, owner: "acme",
            lastUsed: Date(timeIntervalSince1970: 20)),
        SavedRepository(id: 3, name: "app-server", location: "/code/app-server"),
        SavedRepository(id: 4, name: "mapper", location: "/code/mapper"),
        SavedRepository(id: 5, name: "a-p-p", location: "/code/a-p-p"),
    ]

    func testRankingPutsAStartFirstThenAWordThenAnywhereThenTheLettersInOrder() {
        let ranked = RepositorySearch.ranked("app", in: saved).map(\.id)
        // app-server starts with it, web-app has a word that does, mapper
        // holds it, a-p-p has its letters in order; humata has none.
        XCTAssertEqual(ranked, [3, 1, 4, 5])
    }

    func testTiesKeepTheMostRecentlyUsedFirst() {
        let repositories = [
            SavedRepository(id: 1, name: "api-new", location: "/a", lastUsed: Date(timeIntervalSince1970: 9)),
            SavedRepository(id: 2, name: "api-old", location: "/b", lastUsed: Date(timeIntervalSince1970: 1)),
        ]
        XCTAssertEqual(RepositorySearch.ranked("api", in: repositories).map(\.id), [1, 2])
        XCTAssertEqual(RepositorySearch.ranked("", in: repositories).map(\.id), [1, 2])
    }

    func testTheLocationMatchesToo() {
        XCTAssertEqual(RepositorySearch.ranked("acme", in: saved).map(\.id), [2])
    }

    func testAURLIsTakenAsTyped() {
        XCTAssertEqual(RepositorySearch.suggestions(for: "https://github.com/acme/app", in: saved, home: "/"), [])
        XCTAssertEqual(RepositorySearch.suggestions(for: "git@github.com:acme/app.git", in: saved, home: "/"), [])
    }

    func testAPathCompletesFoldersAndTildeIsTheHomeFolder() throws {
        let root = try folders(["alpha", "Alps", "beta", ".hidden"], files: ["alpine.txt"])
        XCTAssertEqual(
            RepositorySearch.folders(completing: root + "/al", home: "/nowhere"), [root + "/alpha", root + "/Alps"])
        XCTAssertEqual(
            RepositorySearch.folders(completing: root + "/", home: "/nowhere"),
            [root + "/alpha", root + "/Alps", root + "/beta"])
        XCTAssertEqual(RepositorySearch.folders(completing: root + "/.", home: "/nowhere"), [root + "/.hidden"])
        XCTAssertEqual(RepositorySearch.folders(completing: "~/be", home: root), [root + "/beta"])
        XCTAssertEqual(RepositorySearch.folders(completing: root + "/missing/", home: "/nowhere"), [])
    }

    func testTypingInNewProjectOffersTheSavedListRecentFirstAndPickingFillsTheRow() async throws {
        let daemon = InMemoryDaemon()
        daemon.saveRepositories(saved)
        let store = newProject(daemon)

        await store.send(\.search.typed, (row: 0, ""))
        await settle(store)
        XCTAssertEqual(store.state.search.row, 0)
        XCTAssertEqual(
            store.state.search.suggestions.map(\.location).prefix(2),
            ["/code/web-app", "https://github.com/acme/humata"])

        await store.send(\.search.typed, (row: 0, "hum"))
        await settle(store)
        XCTAssertEqual(store.state.search.suggestions.map(\.name), ["humata"])

        await store.send(\.search.submitted)
        await settle(store)
        XCTAssertEqual(store.state.repositories[id: 0]?.path, "https://github.com/acme/humata")
        XCTAssertEqual(store.state.name, "humata")
        XCTAssertNil(store.state.search.row)
        XCTAssertTrue(store.state.canCreate)

        // The row's text changing to what was picked leaves the list shut.
        await store.send(\.search.typed, (row: 0, "https://github.com/acme/humata"))
        await settle(store)
        XCTAssertNil(store.state.search.row)
    }

    func testArrowsMoveAndReturnPicksTheHighlightedOne() async throws {
        let daemon = InMemoryDaemon()
        daemon.saveRepositories(saved)
        let store = newProject(daemon)

        await store.send(\.search.typed, (row: 0, "app"))
        await settle(store)
        await store.send(\.search.moved, 1)
        await store.send(\.search.moved, 10)
        XCTAssertEqual(store.state.search.highlighted?.location, "/code/a-p-p")
        await store.send(\.search.moved, -1)
        await store.send(\.search.submitted)
        await settle(store)
        XCTAssertEqual(store.state.repositories[id: 0]?.path, "/code/mapper")
    }

    func testEscapeClosesTheListUntilTheTextChanges() async throws {
        let daemon = InMemoryDaemon()
        daemon.saveRepositories(saved)
        let store = newProject(daemon)

        await store.send(\.search.typed, (row: 0, "app"))
        await settle(store)
        await store.send(\.search.closed)
        await store.send(\.search.typed, (row: 0, "app"))
        await settle(store)
        XCTAssertNil(store.state.search.row)

        await store.send(\.search.typed, (row: 0, "apps"))
        await settle(store)
        XCTAssertEqual(store.state.search.row, 0)
    }

    func testAFolderTypedCompletesAndPickingItFillsTheRow() async throws {
        let root = try folders(["humata", "lifr"])
        let daemon = InMemoryDaemon()
        let store = newProject(daemon)

        await store.send(\.search.typed, (row: 0, root + "/hu"))
        await settle(store)
        XCTAssertEqual(store.state.search.suggestions, [.folder(root + "/humata")])
        await store.send(\.search.submitted)
        await settle(store)
        XCTAssertEqual(store.state.repositories[id: 0]?.path, root + "/humata")
        XCTAssertEqual(store.state.name, "humata")
    }

    func testProjectSettingsPickingFillsOnlyThatRow() async throws {
        let daemon = InMemoryDaemon()
        daemon.saveRepositories(saved)
        let project = try await daemon.client.createProject("code", [Repository(path: "/code/one")], "")
        let store = TestStore(initialState: ProjectSettingsFeature.State(project: project)) {
            ProjectSettingsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.addRepositoryTapped)
        let rows = store.state.repositories.ids
        await store.send(\.search.typed, (row: rows[1], "mapp"))
        await settle(store)
        await store.send(\.search.submitted)
        await settle(store)
        XCTAssertEqual(store.state.repositories.map(\.path), ["/code/one", "/code/mapper"])
        XCTAssertEqual(store.state.name, "code")
    }

    func testWhenTheSavedListCantBeReadTypingInFullStillWorks() async throws {
        let store = TestStore(initialState: NewProjectFeature.State()) {
            NewProjectFeature()
        } withDependencies: {
            $0.daemon.savedRepositories = { throw DaemonError.disconnected }
        }
        store.exhaustivity = .off

        await store.send(\.binding.repositories, [RepositoryRow(id: 0, path: "/code/x")])
        await store.send(\.search.typed, (row: 0, "/code/x"))
        await settle(store)
        XCTAssertNotNil(store.state.search.failure)
        XCTAssertFalse(store.state.search.loading)
        await store.send(\.binding.name, "x")
        XCTAssertTrue(store.state.canCreate)
    }

    private func newProject(_ daemon: InMemoryDaemon) -> TestStoreOf<NewProjectFeature> {
        let store = TestStore(initialState: NewProjectFeature.State()) {
            NewProjectFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off
        return store
    }

    /// A new folder holding these folders and files; its path.
    private func folders(_ names: [String], files: [String] = []) throws -> String {
        let root = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString)
        for name in names {
            try FileManager.default.createDirectory(at: root.appending(path: name), withIntermediateDirectories: true)
        }
        for file in files {
            try Data().write(to: root.appending(path: file))
        }
        addTeardownBlock { try? FileManager.default.removeItem(at: root) }
        return root.standardizedFileURL.path(percentEncoded: false).trimmingSuffix("/")
    }
}

extension String {
    fileprivate func trimmingSuffix(_ suffix: String) -> String {
        hasSuffix(suffix) ? String(dropLast(suffix.count)) : self
    }
}
