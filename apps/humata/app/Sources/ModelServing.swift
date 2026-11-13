import Foundation
import HumataAppFeature
import HumataDaemonClient
import InferenceKit
import InferenceKitLive
import InferenceServerKit
import os

#if canImport(InferenceKitMLX)
    import InferenceKitMLX
#endif

/// Serves this Mac's models while the app runs: Apple's on-device model as
/// "apple", and each model downloaded for Humata by its id. Its port is the
/// same each launch when it's free, so a host's forward keeps working.
private actor ModelServing {
    static let shared = ModelServing()

    /// The port tried first, and the one sessions find after a restart.
    private static let preferredPort: UInt16 = 47830
    /// Where the port it last served on is kept.
    private static let portKey = "modelServerPort"

    private var port: UInt16?
    private let server = ModelServer { ModelServing.models() }

    func start() async -> UInt16? {
        if let port { return port }
        do throws(ModelServerError) {
            let started: UInt16
            // The port of the last launch next, if the preferred one was
            // taken then: hosts forward the port they were connected with,
            // and one that changes has them reconnected.
            let remembered = UInt16(exactly: UserDefaults.standard.integer(forKey: Self.portKey))
                .flatMap { $0 != 0 && $0 != Self.preferredPort ? $0 : nil }
            if let preferred = try? await server.start(port: Self.preferredPort) {
                started = preferred
            } else if let remembered, let again = try? await server.start(port: remembered) {
                started = again
            } else {
                started = try await server.start(port: 0)
            }
            UserDefaults.standard.set(Int(started), forKey: Self.portKey)
            port = started
            probe(on: started)
            return started
        } catch {
            // Sessions on it wait and say the server isn't running.
            FileHandle.standardError.write(Data("Humata: \(error)\n".utf8))
            return nil
        }
    }

    private static let library = ModelLibrary.live(userAgent: "Humata")

    private static let host: ModelHost = {
        var loaders: [RuntimeKind: ModelHost.Loader] = [:]
        #if canImport(InferenceKitMLX)
            // A closure that throws a type crashes the compiler when it's
            // stored straight into a dictionary's subscript.
            let mlx: ModelHost.Loader = { model, backend async throws(ModelRuntimeError) in
                try await MLXEngine.load(model, on: backend)
            }
            loaders[.mlx] = mlx
        #endif
        let host = ModelHost(load: ModelHost.loading(with: loaders))
        // Lets go of the model when the Mac runs low on memory.
        Task { await host.observeSystem() }
        return host
    }()

    /// The models downloaded, each measured, off the main thread.
    static func files() async -> [ModelFiles.Model] {
        await Task.detached(priority: .utility) {
            Self.library.installed().map { model in
                ModelFiles.Model(
                    id: model.id.rawValue, name: model.spec.name, bytes: Self.allocated(model.directory),
                    path: model.directory.path(percentEncoded: false))
            }
        }.value
    }

    static func delete(_ id: String) async throws {
        try await library.delete(ModelID(rawValue: id))
    }

    /// The disk space the files under `folder` take.
    private static func allocated(_ folder: URL) -> UInt64 {
        let keys: [URLResourceKey] = [.totalFileAllocatedSizeKey, .isRegularFileKey]
        guard let files = FileManager.default.enumerator(at: folder, includingPropertiesForKeys: keys) else {
            return 0
        }
        var bytes: UInt64 = 0
        for case let file as URL in files {
            let values = try? file.resourceValues(forKeys: Set(keys))
            if values?.isRegularFile == true {
                bytes += UInt64(values?.totalFileAllocatedSize ?? 0)
            }
        }
        return bytes
    }

    private static func models() -> [ServedModel] {
        let downloaded = library.installed().filter { $0.spec.runtime == .mlx }.map { model in
            ServedModel(id: model.id.rawValue, runtime: EngineRuntime(model: model, host: host))
        }
        return ([ServedModel(id: "apple", runtime: AppleRuntime())] + downloaded).map { model in
            let id = model.id
            var model = model
            model.detail = verdicts.withLock { $0[id] }.map(Self.detail)
            return model
        }
    }

    /// How well each model called tools when it was measured on this Mac,
    /// by its id; kept between launches, since measuring loads the model.
    private static let verdicts = OSAllocatedUnfairLock(initialState: storedVerdicts())
    private static let verdictsKey = "toolProbeVerdicts"

    private static func storedVerdicts() -> [String: ToolProbe.Verdict] {
        guard let data = UserDefaults.standard.data(forKey: verdictsKey) else { return [:] }
        return (try? JSONDecoder().decode([String: ToolProbe.Verdict].self, from: data)) ?? [:]
    }

    private static func detail(_ verdict: ToolProbe.Verdict) -> String {
        switch verdict {
        case .good: "Calls tools well"
        case .poor: "Missed simple tool calls on this Mac; it may not work well as an agent"
        }
    }

    /// Measures, one at a time and in the background, each model that
    /// hasn't been yet, so the picker can say which call tools well. A
    /// model that couldn't be asked is tried again next launch.
    private func probe(on port: UInt16) {
        guard let base = URL(string: "http://127.0.0.1:\(port)/v1") else { return }
        Task.detached(priority: .background) {
            for model in Self.models() {
                let id = model.id
                guard Self.verdicts.withLock({ $0[id] }) == nil else { continue }
                guard let verdict = await ToolProbe.run(base: base, model: id) else { continue }
                let all = Self.verdicts.withLock { verdicts in
                    verdicts[id] = verdict
                    return verdicts
                }
                if let data = try? JSONEncoder().encode(all) {
                    UserDefaults.standard.set(data, forKey: Self.verdictsKey)
                }
            }
        }
    }
}

extension ModelServerService {
    static let live = ModelServerService { await ModelServing.shared.start() }
}

extension ModelFiles {
    static let live = ModelFiles(
        folder: ModelLibrary.defaultRoot.path(percentEncoded: false),
        installed: { await ModelServing.files() },
        delete: { id in try await ModelServing.delete(id) })
}
