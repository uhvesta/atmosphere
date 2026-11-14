import ComposableArchitecture
import HumataDaemonClient
import HumataProjects
import XCTest

@MainActor
final class SavedRepositoriesFeatureTests: XCTestCase {
    func testAddingAnOwnerListsItsRepositories() async throws {
        let daemon = InMemoryDaemon()
        daemon.listOnGitHub("octo-org", .repositories(["api", "web"]))
        let store = settings(daemon)

        await store.send(.task)
        await settle(store)
        XCTAssertEqual(store.state.owners, [])
        XCTAssertEqual(store.state.repositories, [])

        await store.send(\.binding.newOwner, "@octo-org")
        await store.send(.addOwnerTapped)
        await settle(store)
        XCTAssertFalse(store.state.addingOwner)
        XCTAssertEqual(store.state.newOwner, "")
        XCTAssertEqual(store.state.owners?.map(\.login), ["octo-org"])
        XCTAssertEqual(store.state.owners?.first?.repositories, 2)
        XCTAssertNotNil(store.state.owners?.first?.refreshed)
        XCTAssertEqual(store.state.repositories?.map(\.name).sorted(), ["api", "web"])
        XCTAssertNil(store.state.failure)
    }

    func testAnOwnerThatCantBeAddedSaysWhyAndKeepsWhatWasTyped() async throws {
        let daemon = InMemoryDaemon()
        let store = settings(daemon)

        await store.send(.task)
        await settle(store)
        await store.send(\.binding.newOwner, "not a login!")
        await store.send(.addOwnerTapped)
        await settle(store)
        XCTAssertNotNil(store.state.failure)
        XCTAssertFalse(store.state.addingOwner)
        XCTAssertEqual(store.state.newOwner, "not a login!")
        XCTAssertEqual(store.state.owners, [])
    }

    func testARefreshThatFailsShowsWhyOnTheOwnerAndKeepsWhatWasListed() async throws {
        let daemon = InMemoryDaemon()
        daemon.listOnGitHub("octo-org", .repositories(["api"]))
        _ = try await daemon.client.addRepositoryOwner("octo-org")
        let store = settings(daemon)
        await store.send(.task)
        await settle(store)

        daemon.listOnGitHub("octo-org", .fails("GitHub refused this Mac's token; sign in again with gh auth login."))
        await store.send(.refreshTapped("octo-org"))
        await settle(store)
        XCTAssertEqual(store.state.refreshing, [])
        XCTAssertNotNil(store.state.owners?.first?.error)
        XCTAssertEqual(store.state.owners?.first?.repositories, 1)
        XCTAssertEqual(store.state.repositories?.map(\.name), ["api"])

        daemon.listOnGitHub("octo-org", .repositories(["api", "docs"]))
        await store.send(.refreshTapped(nil))
        await settle(store)
        XCTAssertNil(store.state.owners?.first?.error)
        XCTAssertEqual(store.state.owners?.first?.repositories, 2)
        XCTAssertEqual(store.state.repositories?.count, 2)
    }

    func testRemovingAnOwnerDropsOnlyItsRepositories() async throws {
        let daemon = InMemoryDaemon()
        daemon.saveRepositories([SavedRepository(id: 1, name: "mine", location: "/code/mine")])
        daemon.listOnGitHub("octo-org", .repositories(["api"]))
        _ = try await daemon.client.addRepositoryOwner("octo-org")
        let store = settings(daemon)
        await store.send(.task)
        await settle(store)
        XCTAssertEqual(store.state.repositories?.count, 2)

        await store.send(.removeOwnerTapped("octo-org"))
        await settle(store)
        XCTAssertEqual(store.state.owners, [])
        XCTAssertEqual(store.state.repositories?.map(\.name), ["mine"])
    }

    func testRenamingAndRemovingARepositoryChangesNoProject() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject("code", [Repository(path: "/code/mine")], "")
        daemon.saveRepositories([
            SavedRepository(id: 1, name: "mine", location: "/code/mine"),
            SavedRepository(id: 2, name: "other", location: "/code/other"),
        ])
        let store = settings(daemon)
        await store.send(.task)
        await settle(store)

        await store.send(.renameTapped(1))
        await store.send(\.binding.newName, "  Mine  ")
        await store.send(.renameSubmitted)
        await settle(store)
        XCTAssertNil(store.state.renaming)
        XCTAssertEqual(store.state.repositories?.first { $0.id == 1 }?.name, "Mine")

        await store.send(.removeTapped(2))
        await settle(store)
        XCTAssertEqual(store.state.repositories?.map(\.id), [1])
        let projects = try await daemon.client.projects()
        XCTAssertEqual(projects, [project])
    }

    func testAnOwnerStillBeingListedWhenLookingStopsIsMarkedUnfinished() async throws {
        // The daemon never gets to the end of the listing.
        let pending = [RepositoryOwner(login: "octo-org")]
        let listed = LockIsolated(false)
        let store = TestStore(initialState: SavedRepositoriesFeature.State()) {
            SavedRepositoriesFeature()
        } withDependencies: {
            $0.daemon.savedRepositories = { [] }
            $0.daemon.repositoryOwners = { pending }
            $0.daemon.addRepositoryOwner = { _ in pending }
            $0.daemon.refreshRepositoryOwners = { _ in
                listed.setValue(true)
                return [RepositoryOwner(login: "octo-org", refreshed: Date(timeIntervalSince1970: 60))]
            }
            $0.continuousClock = ImmediateClock()
        }
        store.exhaustivity = .off

        await store.send(\.binding.newOwner, "octo-org")
        await store.send(.addOwnerTapped)
        await store.receive(\.ownerAdded)
        XCTAssertEqual(store.state.listing, ["octo-org"])
        await store.receive(\.listingEnded, timeout: .seconds(5))
        XCTAssertEqual(store.state.listing, [], "it no longer shows as being listed")
        XCTAssertEqual(store.state.unfinished, ["octo-org"])

        // Refresh looks again, and one that lists it clears the mark.
        await store.send(.refreshTapped("octo-org"))
        await store.receive(\.refreshed)
        XCTAssertTrue(listed.value)
        XCTAssertEqual(store.state.unfinished, [])
        await settle(store)
    }

    func testAnOwnerWhoseListingCantBeLookedAtIsMarkedUnfinished() async throws {
        let pending = [RepositoryOwner(login: "octo-org")]
        let store = TestStore(initialState: SavedRepositoriesFeature.State()) {
            SavedRepositoriesFeature()
        } withDependencies: {
            $0.daemon.savedRepositories = { [] }
            $0.daemon.addRepositoryOwner = { _ in pending }
            // The daemon goes away while the owner is being listed.
            $0.daemon.repositoryOwners = { throw DaemonError.disconnected }
            $0.continuousClock = ImmediateClock()
        }
        store.exhaustivity = .off

        await store.send(\.binding.newOwner, "octo-org")
        await store.send(.addOwnerTapped)
        await store.receive(\.ownerAdded)
        XCTAssertEqual(store.state.listing, ["octo-org"])
        await store.receive(\.listingEnded, timeout: .seconds(5))
        XCTAssertEqual(store.state.listing, [])
        XCTAssertEqual(store.state.unfinished, ["octo-org"])
        XCTAssertNotNil(store.state.failure)
        await settle(store)
    }

    func testARefreshThatListsAnOwnerJustAddedEndsItsListing() async throws {
        let pending = [RepositoryOwner(login: "octo-org")]
        let store = TestStore(initialState: SavedRepositoriesFeature.State()) {
            SavedRepositoriesFeature()
        } withDependencies: {
            $0.daemon.savedRepositories = { [] }
            $0.daemon.addRepositoryOwner = { _ in pending }
            $0.daemon.repositoryOwners = { pending }
            $0.daemon.refreshRepositoryOwners = { _ in
                [RepositoryOwner(login: "octo-org", refreshed: Date(timeIntervalSince1970: 60))]
            }
            // Never advanced: looking again waits for as long as it's left to.
            $0.continuousClock = TestClock()
        }
        store.exhaustivity = .off

        await store.send(\.binding.newOwner, "octo-org")
        await store.send(.addOwnerTapped)
        await store.receive(\.ownerAdded)
        XCTAssertEqual(store.state.listing, ["octo-org"])

        await store.send(.refreshTapped("octo-org"))
        await store.receive(\.refreshed)
        XCTAssertEqual(store.state.listing, [])
        XCTAssertEqual(store.state.unfinished, [])
        // Nothing is left looking, so the effects end with no time passing.
        await settle(store)
    }

    func testARenameThatFailsOpensTheFieldAgainWithWhatWasTyped() async throws {
        let saved = [SavedRepository(id: 1, name: "mine", location: "/code/mine")]
        let store = TestStore(initialState: SavedRepositoriesFeature.State()) {
            SavedRepositoriesFeature()
        } withDependencies: {
            $0.daemon.savedRepositories = { saved }
            $0.daemon.repositoryOwners = { [] }
            $0.daemon.renameSavedRepository = { _, _ in throw DaemonError.disconnected }
        }
        store.exhaustivity = .off
        await store.send(.task)
        await settle(store)

        await store.send(.renameTapped(1))
        await store.send(\.binding.newName, "Mine")
        await store.send(.renameSubmitted)
        await settle(store)
        XCTAssertEqual(store.state.renaming, 1)
        XCTAssertEqual(store.state.newName, "Mine")
        XCTAssertNotNil(store.state.failure)
        XCTAssertEqual(store.state.repositories?.map(\.name), ["mine"])
    }

    func testWhenTheDaemonDoesntAnswerItSaysSo() async throws {
        let store = TestStore(initialState: SavedRepositoriesFeature.State()) {
            SavedRepositoriesFeature()
        } withDependencies: {
            $0.daemon.savedRepositories = { throw DaemonError.disconnected }
            $0.daemon.repositoryOwners = { [] }
        }
        store.exhaustivity = .off
        await store.send(.task)
        await settle(store)
        XCTAssertFalse(store.state.loading)
        XCTAssertNotNil(store.state.failure)
        XCTAssertNil(store.state.repositories)
    }

    private func settings(_ daemon: InMemoryDaemon) -> TestStoreOf<SavedRepositoriesFeature> {
        let store = TestStore(initialState: SavedRepositoriesFeature.State()) {
            SavedRepositoriesFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            $0.continuousClock = ImmediateClock()
        }
        store.exhaustivity = .off
        return store
    }
}
