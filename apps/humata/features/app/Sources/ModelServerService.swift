import ComposableArchitecture

/// Humata's own model server, which serves the Mac's models to local-model
/// sessions here and, forwarded over SSH, on hosts.
package struct ModelServerService: Sendable {
    /// Starts the server unless it's running, and returns its port on
    /// 127.0.0.1; nil when it can't listen.
    package var start: @Sendable () async -> UInt16?

    package init(start: @escaping @Sendable () async -> UInt16?) {
        self.start = start
    }
}

extension ModelServerService: TestDependencyKey {
    /// No server: sessions on the app's server wait, saying why.
    package static let testValue = ModelServerService { nil }
}

extension DependencyValues {
    package var modelServer: ModelServerService {
        get { self[ModelServerService.self] }
        set { self[ModelServerService.self] = newValue }
    }
}
