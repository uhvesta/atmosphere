import Foundation
import HumataAppFeature
import InferenceKit
import InferenceKitLive
import InferenceServerKit

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

    private var port: UInt16?
    private let server = ModelServer { ModelServing.models() }

    func start() async -> UInt16? {
        if let port { return port }
        do throws(ModelServerError) {
            let started: UInt16
            if let preferred = try? await server.start(port: Self.preferredPort) {
                started = preferred
            } else {
                started = try await server.start(port: 0)
            }
            port = started
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

    private static func models() -> [ServedModel] {
        let downloaded = library.installed().filter { $0.spec.runtime == .mlx }.map { model in
            ServedModel(id: model.id.rawValue, runtime: EngineRuntime(model: model, host: host))
        }
        return [ServedModel(id: "apple", runtime: AppleRuntime())] + downloaded
    }
}

extension ModelServerService {
    static let live = ModelServerService { await ModelServing.shared.start() }
}
