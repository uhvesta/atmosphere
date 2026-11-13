import ComposableArchitecture
import Foundation

/// The models downloaded for Humata on this Mac, which its model server
/// serves. They're the app's files, not humatad's, so Storage reads them
/// here.
package struct ModelFiles: Sendable {
    package struct Model: Hashable, Sendable, Identifiable {
        package var id: String
        package var name: String
        /// The disk space its files take.
        package var bytes: UInt64
        /// Its folder.
        package var path: String

        package init(id: String, name: String, bytes: UInt64, path: String) {
            self.id = id
            self.name = name
            self.bytes = bytes
            self.path = path
        }
    }

    /// The folder the models are in, to show in Finder.
    package var folder: String
    /// Each model downloaded, measured off the main thread.
    package var installed: @Sendable () async -> [Model]
    package var delete: @Sendable (_ id: String) async throws -> Void

    package init(
        folder: String, installed: @escaping @Sendable () async -> [Model],
        delete: @escaping @Sendable (_ id: String) async throws -> Void
    ) {
        self.folder = folder
        self.installed = installed
        self.delete = delete
    }
}

extension ModelFiles: TestDependencyKey {
    /// No models downloaded.
    package static let testValue = ModelFiles(folder: "", installed: { [] }, delete: { _ in })
}

extension DependencyValues {
    package var modelFiles: ModelFiles {
        get { self[ModelFiles.self] }
        set { self[ModelFiles.self] = newValue }
    }
}
