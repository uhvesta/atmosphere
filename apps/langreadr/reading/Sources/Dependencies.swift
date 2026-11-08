import CaptureKit
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

/// Writes what the reader says into the message field, on this device.
package struct DictationClient: Sendable {
    /// Asks for speech recognition and the microphone if LangReadr hasn't
    /// yet, then listens. The stream has everything heard so far each time
    /// it grows, and ends after `stop`. Throws `DictationError`.
    package var start: @Sendable () async throws -> AsyncThrowingStream<String, any Error>
    /// Stops listening; the stream ends with the last words.
    package var stop: @Sendable () async -> Void

    package init(
        start: @escaping @Sendable () async throws -> AsyncThrowingStream<String, any Error>,
        stop: @escaping @Sendable () async -> Void
    ) {
        self.start = start
        self.stop = stop
    }

    /// The system's speech recognition, on the device only, in the
    /// reader's language.
    package static func live() -> Self {
        let dictation = Dictation()
        return DictationClient(start: { try await dictation.start() }, stop: { dictation.stop() })
    }

    /// Hears `heard`, one after another, then waits to be stopped: for
    /// tests and previews.
    package static func scripted(_ heard: [String] = ["What does", "What does this word mean?"]) -> Self {
        let listening = LockIsolated<AsyncThrowingStream<String, any Error>.Continuation?>(nil)
        return DictationClient(
            start: {
                let (stream, continuation) = AsyncThrowingStream.makeStream(of: String.self)
                for words in heard { continuation.yield(words) }
                listening.setValue(continuation)
                return stream
            },
            stop: { listening.withValue { $0?.finish() } })
    }

    /// Dictation that isn't available, as with no on-device recognizer
    /// for the reader's language.
    package static func failing(_ error: DictationError) -> Self {
        DictationClient(start: { throw error }, stop: {})
    }
}

extension DictationClient: DependencyKey {
    package static let liveValue = DictationClient.live()
    package static var testValue: DictationClient { .failing(.unavailable(language: "en")) }
}

/// Reads the files the reader picks to send with a message, as their text.
package struct FileReaderClient: Sendable {
    /// Reads the file at `url`. Throws `FileError`.
    package var read: @Sendable (_ url: URL) async throws -> PickedFile

    package init(read: @escaping @Sendable (_ url: URL) async throws -> PickedFile) {
        self.read = read
    }

    /// The most of a file's text a message sends.
    package static let textLimit = 8_000

    /// Reads `files`, by the URL's last path component: for tests and
    /// previews.
    package static func scripted(_ files: [String: PickedFile]) -> Self {
        FileReaderClient { url in
            guard let file = files[url.lastPathComponent] else {
                throw FileError.unreadable(name: url.lastPathComponent, reason: "no such file")
            }
            return file
        }
    }
}

extension FileReaderClient: DependencyKey {
    package static let liveValue = FileReaderClient { url in try PickedFile.read(url, limit: textLimit) }
    package static var testValue: FileReaderClient { .scripted([:]) }
}

extension DependencyValues {
    package var dictation: DictationClient {
        get { self[DictationClient.self] }
        set { self[DictationClient.self] = newValue }
    }

    package var fileReader: FileReaderClient {
        get { self[FileReaderClient.self] }
        set { self[FileReaderClient.self] = newValue }
    }

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
