@_exported import InferenceKit

/// One of the things an app asks models to do, such as chatting or naming
/// chats, by its use's raw value: the app's own `ModelUseCase` cases, for
/// screens shared between apps that don't know them.
public struct ModelUseKey: Hashable, Sendable, Codable, RawRepresentable {
    public let rawValue: String

    public init(rawValue: String) {
        self.rawValue = rawValue
    }

    public init<Use: ModelUseCase & RawRepresentable<String>>(_ use: Use) {
        rawValue = use.rawValue
    }
}
