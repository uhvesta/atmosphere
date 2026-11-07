/// A model this device can run now, which a chat or a sheet can pick in
/// place of the default. Nothing on this list needs downloading.
public struct RunnableModel: Hashable, Identifiable, Sendable {
    public var id: ModelID
    public var name: String
    /// The window it runs at unless something picks another: the one
    /// picked for it in Settings, or its own.
    public var window: Int
    /// The windows it can run at, smallest first. Empty when it offers no
    /// choice, as with Apple's.
    public var windows: [Int]

    public init(id: ModelID, name: String, window: Int, windows: [Int]) {
        self.id = id
        self.name = name
        self.window = window
        self.windows = windows
    }

    /// Whether it can think before it answers. Apple's model can't.
    public var thinks: Bool { id != .apple }

    public init(_ model: InstalledModel) {
        let spec = model.spec
        let windows = Set(spec.contextChoices + [spec.contextTokens])
        self.init(
            id: spec.id, name: spec.name, window: model.window,
            windows: spec.contextChoices.isEmpty ? [] : windows.sorted())
    }

    public static let apple = RunnableModel(
        id: .apple, name: ModelCatalog.apple.name, window: ModelCatalog.apple.contextTokens, windows: [])

    /// Apple's model, then each of `installed` but Apple's, by name.
    public static func list(_ installed: [InstalledModel]) -> [RunnableModel] {
        let downloaded = installed.filter { $0.id != .apple }.map(RunnableModel.init)
        return [.apple] + downloaded.sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
    }
}
