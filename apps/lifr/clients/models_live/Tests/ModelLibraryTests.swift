import ComposableArchitecture
import CryptoKit
import Foundation
import LifrModelsClient
import LifrModelsLive
import XCTest

final class ModelLibraryTests: XCTestCase {
    private let repo = "org/model"
    private let commit = "0123456789abcdef0123456789abcdef01234567"
    private let bytes = Data((0..<1_000).map { UInt8($0 % 251) })
    private let root = FileManager.default.temporaryDirectory.appending(
        path: "models-\(UUID())", directoryHint: .isDirectory)
    private let hub = StubHub()

    override func setUp() async throws {
        StubProtocol.hub.setValue(hub)
        hub.files["/\(repo)/resolve/\(commit)/model.litertlm"] = bytes
    }

    override func tearDown() async throws {
        StubProtocol.hub.setValue(nil)
        try? FileManager.default.removeItem(at: root)
    }

    private func library(free: Int64 = 100_000_000_000) -> ModelLibrary {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [StubProtocol.self]
        return ModelLibrary(
            root: root, session: URLSession(configuration: configuration), capacity: { _ in free }, chunkSize: 300,
            catalog: [])
    }

    private var sha256: String {
        SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined()
    }

    private func spec(digest: FileDigest? = nil) -> ModelSpec {
        ModelSpec(
            id: "hf/org/model/model.litertlm", name: "model", summary: repo, runtime: .liteRT,
            source: ModelSource(
                repo: repo, commit: commit,
                files: [ModelFile(path: "model.litertlm", size: Int64(bytes.count), digest: digest ?? .sha256(sha256))]
            ), contextTokens: 4_096, memoryBytes: 0, license: "mit")
    }

    private func serveRepository() {
        hub.json["/api/models/\(repo)/revision/main"] = Data(
            #"{"sha":"\#(commit)","gated":false,"cardData":{"license":"mit"}}"#.utf8)
        hub.json["/api/models/\(repo)/tree/\(commit)"] = Data(
            #"""
            [{"type":"file","oid":"1","size":12,"path":"README.md"},
             {"type":"file","oid":"2","size":\#(bytes.count),"path":"model.litertlm",
              "lfs":{"oid":"\#(sha256)","size":\#(bytes.count)}}]
            """#.utf8)
    }

    private static func attempt(_ library: ModelLibrary, _ spec: ModelSpec) async -> Result<
        InstalledModel, ModelDownloadError
    > {
        do {
            return .success(try await library.download(spec))
        } catch {
            return .failure(error)
        }
    }

    // MARK: Resolving

    func testALinkResolvesToTheModelAtItsCommit() async throws {
        serveRepository()

        let spec = try await library().resolve("https://huggingface.co/\(repo)", sha256: nil)

        XCTAssertEqual(spec, self.spec())
    }

    func testHuggingFacesRefusalsSayWhatHappened() async {
        let cases: [(Int, HubError)] = [(401, .notAllowed(repo)), (404, .notFound(repo)), (429, .rateLimited)]
        for (status, expected) in cases {
            hub.statuses["/api/models/\(repo)/revision/main"] = status

            do {
                _ = try await library().resolve("https://huggingface.co/\(repo)", sha256: nil)
                XCTFail("resolved despite \(status)")
            } catch {
                XCTAssertEqual(error, .hub(expected))
            }
        }
    }

    // MARK: Downloading

    func testADownloadIsCheckedAndMovedIntoPlace() async throws {
        let library = library()

        let model = try await library.download(spec())

        XCTAssertEqual(try Data(contentsOf: model.location(of: try XCTUnwrap(model.spec.source?.files.first))), bytes)
        XCTAssertEqual(library.installed().map(\.id), [model.id])
        XCTAssertEqual(
            library.installed().first?.directory.resolvingSymlinksInPath(), model.directory.resolvingSymlinksInPath())
        let staging = root.appending(path: ".staging/\(model.id.directoryName)")
        XCTAssertFalse(FileManager.default.fileExists(atPath: staging.path()))
        XCTAssertEqual(try root.resourceValues(forKeys: [.isExcludedFromBackupKey]).isExcludedFromBackup, true)
        XCTAssertEqual(hub.ranges.count, 4)
    }

    func testAModelAlreadyInPlaceIsntDownloadedAgain() async throws {
        let library = library()
        _ = try await library.download(spec())
        hub.ranges = []

        _ = try await library.download(spec())

        XCTAssertTrue(hub.ranges.isEmpty)
    }

    func testAPausedDownloadGoesOnFromWhereItStopped() async throws {
        let library = library()
        hub.hold = 300
        let started = expectation(description: "the second chunk was asked for")
        hub.onHold = { started.fulfill() }

        let spec = spec()
        let first = Task { await Self.attempt(library, spec) }
        await fulfillment(of: [started], timeout: 5)
        await library.client.pause(spec.id)
        let paused = await first.value
        var downloads = library.client.downloads().makeAsyncIterator()
        let now = await downloads.next()

        XCTAssertEqual(paused.failure, .paused)
        XCTAssertEqual(now?[spec.id], .paused(received: 300, total: 1_000))
        hub.hold = nil
        hub.ranges = []

        let model = try await library.download(spec)

        XCTAssertEqual(hub.ranges.first, "bytes=300-599")
        XCTAssertEqual(try Data(contentsOf: model.location(of: try XCTUnwrap(model.spec.source?.files.first))), bytes)
    }

    func testADownloadPausedBeforeLifrQuitIsStillPausedNextTime() async throws {
        let before = library()
        hub.hold = 300
        let started = expectation(description: "the second chunk was asked for")
        hub.onHold = { started.fulfill() }
        let spec = spec()
        let first = Task { await Self.attempt(before, spec) }
        await fulfillment(of: [started], timeout: 5)
        await before.client.pause(spec.id)
        _ = await first.value
        hub.ranges = []

        let after = library()
        after.resumeUnfinished()
        var downloads = after.client.downloads().makeAsyncIterator()
        let now = await downloads.next()

        XCTAssertEqual(now?[spec.id], .paused(received: 300, total: 1_000))
        let unfinished = await after.client.unfinished()
        XCTAssertEqual(unfinished, [spec])
        XCTAssertEqual(hub.ranges, [])
    }

    func testADownloadRunningWhenLifrQuitStartsAgainFromWhereItWas() async throws {
        let before = library()
        hub.hold = 300
        let started = expectation(description: "the second chunk was asked for")
        hub.onHold = { started.fulfill() }
        let spec = spec()
        let first = Task { await Self.attempt(before, spec) }
        await fulfillment(of: [started], timeout: 5)
        hub.hold = nil
        hub.ranges = []

        let after = library()
        after.resumeUnfinished()
        let model = try await after.download(spec)

        XCTAssertEqual(hub.ranges.first, "bytes=300-599")
        XCTAssertEqual(try Data(contentsOf: model.location(of: try XCTUnwrap(model.spec.source?.files.first))), bytes)
        let unfinished = await after.client.unfinished()
        XCTAssertEqual(unfinished, [])
        first.cancel()
        await before.client.pause(spec.id)
    }

    func testRemovingAnUnfinishedDownloadForgetsIt() async throws {
        let library = library()
        hub.hold = 300
        let started = expectation(description: "the second chunk was asked for")
        hub.onHold = { started.fulfill() }
        let spec = spec()
        let first = Task { await Self.attempt(library, spec) }
        await fulfillment(of: [started], timeout: 5)

        try await library.delete(spec.id)
        _ = await first.value

        let unfinished = await library.client.unfinished()
        XCTAssertEqual(unfinished, [])
    }

    func testAFileThatDoesntMatchItsHashIsDeleted() async throws {
        let library = library()
        let wrong = spec(digest: .sha256(String(repeating: "0", count: 64)))

        do {
            _ = try await library.download(wrong)
            XCTFail("installed a file with the wrong hash")
        } catch {
            XCTAssertEqual(error, .digestMismatch(file: "model.litertlm"))
        }

        XCTAssertTrue(library.installed().isEmpty)
        hub.ranges = []
        _ = try? await library.download(wrong)
        XCTAssertEqual(hub.ranges.first, "bytes=0-299")
    }

    func testASmallFileIsCheckedByItsGitBlobHash() async throws {
        let header = Data("blob \(bytes.count)\0".utf8)
        let blob = Insecure.SHA1.hash(data: header + bytes).map { String(format: "%02x", $0) }.joined()

        let model = try await library().download(spec(digest: .gitBlobSHA1(blob)))

        XCTAssertEqual(model.id, spec().id)
    }

    func testADownloadThatWontFitDoesntStart() async {
        do {
            _ = try await library(free: 1_000_000_000).download(spec())
            XCTFail("started without room")
        } catch {
            XCTAssertEqual(error, .notEnoughSpace(needed: 1_000_001_100, available: 1_000_000_000))
        }
        XCTAssertTrue(hub.ranges.isEmpty)
    }

    func testARedirectToHuggingFacesCDNIsFollowed() async throws {
        hub.redirects["/\(repo)/resolve/\(commit)/model.litertlm"] = URL(string: "https://us.aws.cdn.hf.co/blob")
        hub.files["/blob"] = bytes

        let model = try await library().download(spec())

        XCTAssertEqual(model.id, spec().id)
    }

    func testARedirectOffHuggingFaceIsRefused() async {
        hub.redirects["/\(repo)/resolve/\(commit)/model.litertlm"] = URL(string: "https://files.example.com/blob")
        hub.files["/blob"] = bytes
        let library = library()

        do {
            _ = try await library.download(spec())
            XCTFail("followed a redirect off Hugging Face")
        } catch {
            XCTAssertEqual(error, .hub(.redirectNotAllowed("files.example.com")))
        }
        XCTAssertTrue(library.installed().isEmpty)
        XCTAssertFalse(hub.paths.contains("/blob"))
    }

    // MARK: Deleting and choosing

    func testDeletingAModelRemovesItsFilesAndItsUses() async throws {
        let library = library()
        let model = try await library.download(spec())
        library.select(model.id, for: .assistant)
        library.select(model.id, for: .titles)

        try await library.delete(model.id)

        XCTAssertTrue(library.installed().isEmpty)
        XCTAssertFalse(FileManager.default.fileExists(atPath: model.directory.path()))
        XCTAssertEqual(library.selection(for: .assistant), .apple)
        XCTAssertEqual(library.selection(for: .titles), .apple)
    }

    func testTheChoiceOfModelIsKept() {
        library().select("gemma-4-e2b", for: .skills)

        XCTAssertEqual(library().selection(for: .skills), "gemma-4-e2b")
        XCTAssertEqual(library().selection(for: .assistant), .apple)
    }

    func testNamingAndSkillsRunOnTheAssistantsModelUntilPicked() {
        let library = library()
        library.select("gemma-4-e2b", for: .assistant)
        XCTAssertEqual(library.selection(for: .titles), "gemma-4-e2b")
        XCTAssertEqual(library.selection(for: .skills), "gemma-4-e2b")

        library.select(.apple, for: .titles)
        XCTAssertEqual(library.selection(for: .titles), .apple)
        XCTAssertEqual(library.selection(for: .skills), "gemma-4-e2b")
        library.select(.apple, for: .assistant)
        XCTAssertEqual(library.selection(for: .skills), .apple)
    }
}

extension Result {
    fileprivate var failure: Failure? {
        if case .failure(let error) = self { error } else { nil }
    }
}

/// Hugging Face, as far as the tests need it: JSON for the API, files that
/// honor Range, statuses and redirects. `hold` keeps the request for a
/// range starting there waiting until it's cancelled.
final class StubHub: @unchecked Sendable {
    private let lock = NSLock()
    private var _json: [String: Data] = [:]
    private var _files: [String: Data] = [:]
    private var _statuses: [String: Int] = [:]
    private var _redirects: [String: URL] = [:]
    private var _ranges: [String] = []
    private var _paths: [String] = []
    private var _hold: Int?
    private var _onHold: (@Sendable () -> Void)?

    private func locked<T>(_ body: () -> T) -> T {
        lock.lock()
        defer { lock.unlock() }
        return body()
    }

    var json: [String: Data] {
        get { locked { _json } }
        set { locked { _json = newValue } }
    }
    var files: [String: Data] {
        get { locked { _files } }
        set { locked { _files = newValue } }
    }
    var statuses: [String: Int] {
        get { locked { _statuses } }
        set { locked { _statuses = newValue } }
    }
    var redirects: [String: URL] {
        get { locked { _redirects } }
        set { locked { _redirects = newValue } }
    }
    var ranges: [String] {
        get { locked { _ranges } }
        set { locked { _ranges = newValue } }
    }
    var paths: [String] { locked { _paths } }
    var hold: Int? {
        get { locked { _hold } }
        set { locked { _hold = newValue } }
    }
    var onHold: (@Sendable () -> Void)? {
        get { locked { _onHold } }
        set { locked { _onHold = newValue } }
    }

    func respond(to protocolInstance: URLProtocol) {
        guard let client = protocolInstance.client, let url = protocolInstance.request.url else { return }
        let path = url.path(percentEncoded: false)
        locked { _paths.append(path) }
        func reply(_ status: Int, _ body: Data, headers: [String: String] = [:]) {
            guard
                let response = HTTPURLResponse(
                    url: url, statusCode: status, httpVersion: "HTTP/1.1", headerFields: headers)
            else { return }
            client.urlProtocol(protocolInstance, didReceive: response, cacheStoragePolicy: .notAllowed)
            client.urlProtocol(protocolInstance, didLoad: body)
            client.urlProtocolDidFinishLoading(protocolInstance)
        }
        if let target = redirects[path] {
            guard
                let response = HTTPURLResponse(
                    url: url, statusCode: 302, httpVersion: "HTTP/1.1",
                    headerFields: ["Location": target.absoluteString])
            else { return }
            var next = protocolInstance.request
            next.url = target
            client.urlProtocol(protocolInstance, wasRedirectedTo: next, redirectResponse: response)
            // A followed redirect stops this request. A refused one waits
            // for the redirect itself as the answer, as from a server.
            DispatchQueue.global().asyncAfter(deadline: .now() + 0.2) {
                guard let stub = protocolInstance as? StubProtocol, !stub.stopped.value else { return }
                client.urlProtocol(protocolInstance, didReceive: response, cacheStoragePolicy: .notAllowed)
                client.urlProtocolDidFinishLoading(protocolInstance)
            }
            return
        }
        if let status = statuses[path] { return reply(status, Data()) }
        if let body = json[path] { return reply(200, body) }
        guard let file = files[path] else { return reply(404, Data()) }
        guard let header = protocolInstance.request.value(forHTTPHeaderField: "Range") else { return reply(200, file) }
        locked { _ranges.append(header) }
        let bounds = header.dropFirst("bytes=".count).split(separator: "-").compactMap { Int($0) }
        guard bounds.count == 2, bounds[0] <= bounds[1], bounds[0] < file.count else { return reply(416, Data()) }
        if let hold, hold == bounds[0] {
            onHold?()
            return
        }
        let end = min(bounds[1], file.count - 1)
        reply(
            206, file.subdata(in: bounds[0]..<(end + 1)),
            headers: ["Content-Range": "bytes \(bounds[0])-\(end)/\(file.count)"])
    }
}

final class StubProtocol: URLProtocol, @unchecked Sendable {
    static let hub = LockIsolated<StubHub?>(nil)
    let stopped = LockIsolated(false)

    override static func canInit(with request: URLRequest) -> Bool { true }
    override static func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        guard let hub = Self.hub.value else {
            client?.urlProtocol(self, didFailWithError: URLError(.notConnectedToInternet))
            return
        }
        hub.respond(to: self)
    }

    override func stopLoading() { stopped.setValue(true) }
}
