import Foundation

/// Which model each use runs on, as the person picked it, saved as JSON in
/// `file`.
public final class ModelSelection<Use: ModelUseCase>: Sendable {
    private let file: URL?
    private let picked: Locked<[Use: ModelID]>

    /// - Parameter file: Where the picks are kept; nil keeps them in memory.
    ///   A missing or unreadable file starts with none picked.
    public init(file: URL?) {
        self.file = file
        let saved = file.flatMap { try? JSONDecoder().decode([Use: ModelID].self, from: Data(contentsOf: $0)) }
        picked = Locked(saved ?? [:])
    }

    /// What runs `use`; see `ModelUseCase.model(in:)`.
    public func model(for use: Use) -> ModelID {
        use.model(in: picked.value)
    }

    /// Runs `use` on `id` from now on. Picking a use's standard model forgets
    /// the pick, so the use follows its default again.
    public func select(_ id: ModelID, for use: Use) {
        save(
            picked.withValue { picked in
                picked[use] = id == use.standard ? nil : id
                return picked
            })
    }

    /// Forgets every pick of `id`, such as once it's deleted: those uses go
    /// back to their defaults.
    public func forget(_ id: ModelID) {
        save(
            picked.withValue { picked in
                for (use, selected) in picked where selected == id { picked[use] = nil }
                return picked
            })
    }

    /// Whether any use runs on `id`.
    public func isUsed(_ id: ModelID) -> Bool {
        Use.allCases.contains { model(for: $0) == id }
    }

    private func save(_ picked: [Use: ModelID]) {
        guard let file else { return }
        try? FileManager.default.createDirectory(
            at: file.deletingLastPathComponent(), withIntermediateDirectories: true)
        try? JSONEncoder().encode(picked).write(to: file, options: .atomic)
    }
}
