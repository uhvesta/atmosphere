import ComposableArchitecture
import Foundation
import InferenceKit
import LangReadrCore
import LangReadrServices
import LangReadrServicesFake
import XCTest

@testable import LangReadrReading

@MainActor
final class LibraryFeatureTests: XCTestCase {
    private func library(
        books: [Book] = [], shelf: ShelfClient = .inMemory(), services: ReaderServices = .fake()
    ) -> TestStoreOf<LibraryFeature> {
        let store = TestStore(initialState: LibraryFeature.State(books: IdentifiedArray(uniqueElements: books))) {
            LibraryFeature()
        } withDependencies: {
            $0.shelf = shelf
            $0.readerServices = services
            $0.tutor = tutor([])
            $0.date.now = epoch
        }
        store.exhaustivity = .off
        return store
    }

    private func submit(_ kind: ImportKind, _ text: String, in store: TestStoreOf<LibraryFeature>) async {
        await store.send(.importTapped(kind))
        await store.send(.importing(.presented(.binding(.set(\.text, text)))))
        await store.send(.importing(.presented(.importTapped)))
        await store.receive(\.drafted)
    }

    func testPastedTextIsABookInItsLanguageFirstAndOpen() async throws {
        let store = library(services: .fake(language: Language("fr")))
        await submit(.text, "Bonjour.", in: store)
        await submit(.text, "# Deux\nBonsoir.", in: store)
        XCTAssertEqual(store.state.books.map(\.title), ["Deux", "Bonjour."])
        XCTAssertEqual(store.state.books.first?.language, Language("fr"))
        XCTAssertEqual(store.state.reader?.book.id, store.state.books.first?.id)
        XCTAssertNil(store.state.importing)
        XCTAssertNil(store.state.alert)
    }

    func testNothingToImportCantBeImported() async {
        let store = library()
        await store.send(.importTapped(.text))
        await store.send(.importing(.presented(.binding(.set(\.text, "  \n ")))))
        XCTAssertEqual(store.state.importing?.canImport, false)
        await store.send(.importing(.presented(.importTapped)))
        XCTAssertTrue(store.state.books.isEmpty)
    }

    func testAFileWithNothingInItIsRefusedSayingWhy() async throws {
        let file = FileManager.default.temporaryDirectory.appendingPathComponent("\(UUID().uuidString).txt")
        try Data("   ".utf8).write(to: file)
        defer { try? FileManager.default.removeItem(at: file) }
        let store = library()
        await store.send(.fileChosen(file))
        await store.receive(\.drafted.failure)
        XCTAssertTrue(store.state.books.isEmpty)
        XCTAssertNil(store.state.reader)
        XCTAssertNotNil(store.state.alert)
        XCTAssertFalse(store.state.isImporting)
    }

    func testAPageThatCantBeReachedIsRefusedSayingWhy() async {
        let store = library()
        await submit(.web, "example.com/story", in: store)
        XCTAssertTrue(store.state.books.isEmpty)
        XCTAssertNotNil(store.state.alert)
        XCTAssertEqual(store.state.importing?.isImporting, false, "the link stays, to fix and try again")
    }

    func testAWebPagesArticleIsTheBook() async throws {
        let html = "<html><nav>Menú</nav><article><h1>El río</h1><p>El agua corre.</p></article></html>"
        let store = library(services: .fake(page: { _ throws(PageError) in html }))
        await submit(.web, "example.com/rio", in: store)
        let book = try store.state.books.first.unwrap()
        XCTAssertEqual(book.paragraphs.map(\.text), ["El agua corre."])
        XCTAssertEqual(book.source, .web(try URL(string: "https://example.com/rio").unwrap()))
    }

    func testBooksAreOnTheShelfNextTimeAndADeletedOneIsnt() async throws {
        let shelf = ShelfClient.inMemory()
        let first = library(shelf: shelf)
        await submit(.text, "Uno.", in: first)
        await submit(.text, "Dos.", in: first)
        await first.send(.deleteTapped(try first.state.books.last.unwrap().id))
        await first.finish()

        let next = library(shelf: shelf)
        await next.send(.task)
        XCTAssertEqual(next.state.books.map(\.title), ["Dos."])
        XCTAssertNil(next.state.alert)
    }

    func testWhatsDoneInTheOpenBookIsKeptOnTheShelf() async throws {
        let book = try spanish("Ciao.")
        let shelf = ShelfClient.inMemory([book])
        let store = library(books: [book], shelf: shelf)
        await store.send(.bookTapped(book.id))
        await store.send(.reader(.presented(.languagePicked(Language("it")))))
        await store.finish()
        XCTAssertEqual(store.state.books.first?.language, Language("it"))
        XCTAssertEqual(shelf.books().books.first?.language, Language("it"))
    }

    func testCancellingAnImportAddsNothing() async {
        let store = library()
        await store.send(.importTapped(.text))
        await store.send(.importing(.presented(.binding(.set(\.text, "Bonjour.")))))
        await store.send(.importing(.presented(.cancelTapped)))
        await store.receive(\.importing.dismiss)
        XCTAssertNil(store.state.importing)
        XCTAssertTrue(store.state.books.isEmpty)
    }

    func testABookThatCantBeSavedSaysWhy() async {
        var shelf = ShelfClient.inMemory()
        shelf.save = { book throws(ShelfError) in
            throw .write(book: book.title, underlying: CocoaError(.fileWriteOutOfSpace))
        }
        let store = library(shelf: shelf)
        await submit(.text, "Bonjour.", in: store)
        await store.receive(\.saveFailed)
        XCTAssertNotNil(store.state.alert)
    }

    func testAPhotoThatCantBeReadSaysWhy() async {
        let store = library()
        await store.send(.pickFailed("The photo couldn't be loaded."))
        XCTAssertNotNil(store.state.alert)
        XCTAssertTrue(store.state.books.isEmpty)
    }
}
