import ComposableArchitecture
import Foundation
import LifrAssistantClient
import LifrCore
import LifrStoreClient

extension AssistantTools {
    /// Applies what these tools proposed to `store`, as the person does,
    /// and returns the records saved.
    func applied(
        to store: StoreClient, now: Date, conversation: UUID, model: String? = nil
    ) async throws -> Workspace {
        if let proposal {
            let made = try await Proposal.make(
                proposal.actions, on: try await store.load(), now: now, timeZone: .gmt, conversation: conversation,
                model: model)
            _ = try await withDependencies {
                $0.date.now = now
                $0.uuid = .incrementing
            } operation: {
                try await store.save(made.operation)
            }
        }
        return try await store.load()
    }
}
