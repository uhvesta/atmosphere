/// A model this device can run now, which a chat or a sheet can pick in
/// place of the default. Nothing on this list needs downloading.
public struct RunnableModel: Hashable, Identifiable, Sendable {
    public var id: ModelID
    public var name: String
    /// What it's good for, in a few words.
    public var summary: String
    /// The window it runs at unless something picks another: the one
    /// picked for it in Settings, or its own.
    public var window: Int
    /// The windows it can run at, smallest first. Empty when it offers no
    /// choice, as with Apple's.
    public var windows: [Int]

    public init(id: ModelID, name: String, summary: String = "", window: Int, windows: [Int]) {
        self.id = id
        self.name = name
        self.summary = summary
        self.window = window
        self.windows = windows
    }

    /// Whether it can think before it answers. Apple's model can't.
    public var thinks: Bool { id != .apple }

    public init(_ model: InstalledModel) {
        let spec = model.spec
        let windows = Set(spec.contextChoices + [spec.contextTokens])
        self.init(
            id: spec.id, name: spec.name, summary: spec.summary, window: model.window,
            windows: spec.contextChoices.isEmpty ? [] : windows.sorted())
    }

    public static let apple = RunnableModel(
        id: .apple, name: ModelCatalog.apple.name, summary: ModelCatalog.apple.summary,
        window: ModelCatalog.apple.contextTokens, windows: [])

    /// Apple's model, then each of `installed` but Apple's, by name.
    public static func list(_ installed: [InstalledModel]) -> [RunnableModel] {
        var seen: Set<ModelID> = [.apple]
        // A model downloaded in more than one format is one model to pick.
        let downloaded = installed.filter { seen.insert($0.id).inserted }.map(RunnableModel.init)
        return [.apple] + downloaded.sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
    }
}
