/// How long a model thinks before it answers.
public enum ThinkingLevel: String, Hashable, Codable, Sendable, CaseIterable {
    /// It answers straight away. A model that always thinks still does,
    /// but its reasoning isn't shown.
    case off
    /// Its usual budget.
    case on
    /// Four times its usual budget, for harder questions.
    case long

    /// The most tokens it thinks for, or nil when it answers straight away.
    public var tokens: Int? {
        switch self {
        case .off: nil
        case .on: 512
        case .long: 2_048
        }
    }
}

/// What one conversation runs on, in place of the app's defaults. Each
/// field left nil follows the default, so a change of default reaches it.
public struct ModelChoice: Hashable, Codable, Sendable {
    /// The model, or nil for the one picked for the use.
    public var model: ModelID?
    /// How long it thinks, or nil for `ThinkingLevel.on`.
    public var thinking: ThinkingLevel?
    /// Its window in tokens, from the model's `contextChoices`, or nil for
    /// the window picked for the model.
    public var window: Int?

    public init(model: ModelID? = nil, thinking: ThinkingLevel? = nil, window: Int? = nil) {
        self.model = model
        self.thinking = thinking
        self.window = window
    }

    /// Following every default.
    public static let defaults = ModelChoice()

    /// How long it thinks.
    public var thinkingLevel: ThinkingLevel { thinking ?? .on }
}
