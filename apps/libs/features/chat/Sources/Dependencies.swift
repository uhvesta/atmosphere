import CaptureKit
import ComposableArchitecture
import Foundation
import InferenceKit
import ModelsClient

/// What a chat's replies run on: the model for one of the app's uses, or
/// the one a chat picked in place of it. The app gives the live models;
/// tests give scripted ones.
public struct ChatRuntimeClient: Sendable {
    public var runtime: @Sendable (_ use: ModelUseKey, _ choice: ModelChoice) -> any ModelRuntime

    public init(runtime: @escaping @Sendable (_ use: ModelUseKey, _ choice: ModelChoice) -> any ModelRuntime) {
        self.runtime = runtime
    }

    /// Runs every chat on `runtime`.
    public static func always(_ runtime: any ModelRuntime) -> Self {
        ChatRuntimeClient { _, _ in runtime }
    }
}

extension ChatRuntimeClient: TestDependencyKey {
    /// A model with no answers: each turn fails, saying so.
    public static var testValue: Self { .always(ScriptedRuntime(steps: [])) }
}

/// Writes what the person says into the message field, on this device.
public struct DictationClient: Sendable {
    /// Asks for speech recognition and the microphone if the app hasn't
    /// yet, then listens. The stream has everything heard so far each time
    /// it grows, and ends after `stop`. Throws `DictationError`.
    public var start: @Sendable () async throws -> AsyncThrowingStream<String, any Error>
    /// Stops listening; the stream ends with the last words.
    public var stop: @Sendable () async -> Void

    public init(
        start: @escaping @Sendable () async throws -> AsyncThrowingStream<String, any Error>,
        stop: @escaping @Sendable () async -> Void
    ) {
        self.start = start
        self.stop = stop
    }

    /// The system's speech recognition, on the device only, in the
    /// device's language.
    public static func live() -> Self {
        let dictation = Dictation()
        return DictationClient(start: { try await dictation.start() }, stop: { dictation.stop() })
    }

    /// Hears `heard`, one after another, then waits to be stopped: for
    /// tests and previews.
    public static func scripted(_ heard: [String] = ["What does", "What does this word mean?"]) -> Self {
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
    /// for the language.
    public static func failing(_ error: DictationError) -> Self {
        DictationClient(start: { throw error }, stop: {})
    }
}

extension DictationClient: DependencyKey {
    public static let liveValue = DictationClient.live()
    public static var testValue: DictationClient { .failing(.unavailable(language: "en")) }
}

/// Reads the files the person picks to send with a message, as their text.
public struct FileReaderClient: Sendable {
    /// Reads the file at `url`. Throws `FileError`.
    public var read: @Sendable (_ url: URL) async throws -> PickedFile

    public init(read: @escaping @Sendable (_ url: URL) async throws -> PickedFile) {
        self.read = read
    }

    /// The most of a file's text a message sends.
    public static let textLimit = 8_000

    /// Reads `files`, by the URL's last path component: for tests and
    /// previews.
    public static func scripted(_ files: [String: PickedFile]) -> Self {
        FileReaderClient { url in
            guard let file = files[url.lastPathComponent] else {
                throw FileError.unreadable(name: url.lastPathComponent, reason: "no such file")
            }
            return file
        }
    }
}

extension FileReaderClient: DependencyKey {
    public static let liveValue = FileReaderClient { url in try PickedFile.read(url, limit: textLimit) }
    public static var testValue: FileReaderClient { .scripted([:]) }
}

extension DependencyValues {
    public var chatRuntime: ChatRuntimeClient {
        get { self[ChatRuntimeClient.self] }
        set { self[ChatRuntimeClient.self] = newValue }
    }

    public var dictation: DictationClient {
        get { self[DictationClient.self] }
        set { self[DictationClient.self] = newValue }
    }

    public var fileReader: FileReaderClient {
        get { self[FileReaderClient.self] }
        set { self[FileReaderClient.self] = newValue }
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
