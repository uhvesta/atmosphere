import ComposableArchitecture
import Foundation
import InferenceKit
import LangReadrCore
import LangReadrServices

/// The model the reader asks, as a tutor. The app gives the live one, on
/// the device's model; tests give one on a script.
package enum TutorKey: TestDependencyKey {
    /// A tutor whose model has no answers: each turn fails, saying so.
    package static var testValue: Tutor { Tutor(runtime: ScriptedRuntime(steps: []), reader: Language("en")) }
}

/// What the reader asks of the system.
private enum ServicesKey: DependencyKey {
    static let liveValue = ReaderServices.live
    /// Services that find no words and can't read photos or pages.
    static let testValue = ReaderServices(
        detectLanguage: { _ in nil }, words: { _, _ in [] }, transliterate: { _ in nil },
        recognizeText: { _ throws(RecognitionError) in throw .failed("tests read no photos") },
        fetchPage: { url throws(PageError) in throw .unreachable(host: url.host() ?? "", reason: "tests are offline") })
}

/// Where the books are kept.
package struct ShelfClient: Sendable {
    package var books: @Sendable () -> (books: [Book], unreadable: [ShelfError])
    package var save: @Sendable (Book) throws(ShelfError) -> Void
    package var delete: @Sendable (Book) throws(ShelfError) -> Void

    package init(
        books: @escaping @Sendable () -> (books: [Book], unreadable: [ShelfError]),
        save: @escaping @Sendable (Book) throws(ShelfError) -> Void,
        delete: @escaping @Sendable (Book) throws(ShelfError) -> Void
    ) {
        self.books = books
        self.save = save
        self.delete = delete
    }

    package static func live(_ shelf: Shelf) -> Self {
        ShelfClient(
            books: { shelf.books() }, save: { book throws(ShelfError) in try shelf.save(book) },
            delete: { book throws(ShelfError) in try shelf.delete(book) })
    }

    /// A shelf in memory, starting with `books`.
    package static func inMemory(_ books: [Book] = []) -> Self {
        let kept = LockIsolated(books)
        return ShelfClient(
            books: { (kept.value.sorted { $0.importedAt > $1.importedAt }, []) },
            save: { book in
                kept.withValue { books in
                    books.removeAll { $0.id == book.id }
                    books.append(book)
                }
            },
            delete: { book in kept.withValue { $0.removeAll { $0.id == book.id } } })
    }
}

extension ShelfClient: DependencyKey {
    package static let liveValue = ShelfClient.live(.standard)
    package static var testValue: ShelfClient { .inMemory() }
}

extension DependencyValues {
    package var tutor: Tutor {
        get { self[TutorKey.self] }
        set { self[TutorKey.self] = newValue }
    }

    package var readerServices: ReaderServices {
        get { self[ServicesKey.self] }
        set { self[ServicesKey.self] = newValue }
    }

    package var shelf: ShelfClient {
        get { self[ShelfClient.self] }
        set { self[ShelfClient.self] = newValue }
    }
}

/// What a model's work came to: its answer, or why there's none.
func outcome<T>(_ body: () async throws(ModelRuntimeError) -> T) async -> Result<T, ModelRuntimeError> {
    do {
        return .success(try await body())
    } catch {
        return .failure(error)
    }
}
