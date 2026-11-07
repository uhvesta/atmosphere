/// A model this device can run now, which a conversation or a sheet can
/// pick in place of the default. Nothing on this list needs downloading.
package struct RunnableModel: Hashable, Identifiable, Sendable {
    package var id: ModelID
    package var name: String
    /// The window it runs at unless something picks another: the one
    /// picked for it in Settings, or its own.
    package var window: Int
    /// The windows it can run at, smallest first. Empty when it offers no
    /// choice, as with Apple's.
    package var windows: [Int]

    package init(id: ModelID, name: String, window: Int, windows: [Int]) {
        self.id = id
        self.name = name
        self.window = window
        self.windows = windows
    }

    /// Whether it can think before it answers. Apple's model can't.
    package var thinks: Bool { id != .apple }

    package init(_ model: InstalledModel) {
        let spec = model.spec
        let windows = Set(spec.contextChoices + [spec.contextTokens])
        self.init(
            id: spec.id, name: spec.name, window: model.window,
            windows: spec.contextChoices.isEmpty ? [] : windows.sorted())
    }

    static let apple = RunnableModel(
        id: .apple, name: ModelCatalog.apple.name, window: ModelCatalog.apple.contextTokens, windows: [])
}

extension ModelLibraryClient {
    /// Apple's model, then each one downloaded, by name.
    package func runnable() async -> [RunnableModel] {
        let downloaded = await installed().filter { $0.id != .apple }.map(RunnableModel.init)
        return [.apple] + downloaded.sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
    }
}
