/// What the assistant asked to change in one reply: the actions, as data
/// the app can make again on its records as they are when the person
/// applies them, and the steps showing what they'd change. Nothing of it
/// is saved until the person applies it; reading tools run at once.
public struct Proposal<Action: Hashable & Codable & Sendable, Step: Hashable & Codable & Sendable>: Hashable,
    Codable, Sendable
{
    public var actions: [Action]
    /// What the actions change, as the records were when they were
    /// proposed or last edited.
    public var steps: [Step]
    public var decision: Decision?

    public typealias Decision = ProposalDecision

    public init(actions: [Action], steps: [Step], decision: Decision? = nil) {
        self.actions = actions
        self.steps = steps
        self.decision = decision
    }

    /// Whether the person hasn't applied or declined it yet: only then can
    /// it be edited, applied or declined.
    public var isPending: Bool { decision == nil }
}

/// What the person did with a proposal.
public enum ProposalDecision: String, Hashable, Codable, Sendable {
    case applied, declined
}
