import ComposableArchitecture
import Foundation
import HumataBrowser
import HumataDaemonClient
import XCTest

@MainActor
final class FileBrowserFeatureTests: XCTestCase {
    private func store(_ daemon: InMemoryDaemon, _ place: HostPlace) -> TestStoreOf<FileBrowserFeature> {
        let store = TestStore(initialState: FileBrowserFeature.State(place: place)) {
            FileBrowserFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off
        return store
    }

    func testFoldersAreReadAsTheyAreOpenedAndFilesWhenPicked() async {
        let daemon = InMemoryDaemon()
        daemon.addFile("/srv/app/README.md", Data("# App\n".utf8), on: "build")
        daemon.addFile("/srv/app/src/main.rs", Data("fn main() {}\n".utf8), on: "build")
        daemon.addFile("/srv/app/.env", Data("SECRET=1\n".utf8), on: "build")
        let store = store(daemon, HostPlace(host: "build", folder: "/srv/app"))

        await store.send(.task)
        await store.receive(\.folderRead)
        // Hidden files aren't listed, and folders come first.
        XCTAssertEqual(store.state.rows.map(\.path), ["src", "README.md"])
        XCTAssertEqual(store.state.root, "/srv/app")

        await store.send(.folderToggled("src"))
        await store.receive(\.folderRead)
        XCTAssertEqual(store.state.rows.map(\.path), ["src", "src/main.rs", "README.md"])
        XCTAssertEqual(store.state.rows.map(\.depth), [0, 1, 0])

        await store.send(.fileSelected("src/main.rs"))
        await store.receive(\.fileRead)
        XCTAssertEqual(store.state.contents?.text, "fn main() {}\n")
        XCTAssertEqual(store.state.absolutePath, "/srv/app/src/main.rs")

        // Closing a folder hides what's in it without reading it again.
        await store.send(.folderToggled("src"))
        XCTAssertEqual(store.state.rows.map(\.path), ["src", "README.md"])
    }

    func testAnImageIsReadAsItsBytes() async {
        let daemon = InMemoryDaemon()
        let png = Data([0x89, 0x50, 0x4E, 0x47])
        daemon.addFile("/Users/me/shot.png", png)
        let store = store(daemon, HostPlace())

        await store.send(.task)
        await store.receive(\.folderRead)
        await store.send(.fileSelected("shot.png"))
        await store.receive(\.fileRead)
        XCTAssertNil(store.state.contents?.text)
        XCTAssertEqual(store.state.contents?.data, png)
    }

    func testAHostThatCantBeReachedSaysSo() async {
        let daemon = InMemoryDaemon()
        daemon.setOffline("build", true)
        let store = store(daemon, HostPlace(host: "build"))

        await store.send(.task)
        await store.receive(\.failed)
        XCTAssertNotNil(store.state.failure)
        XCTAssertNil(store.state.root)
    }
}
