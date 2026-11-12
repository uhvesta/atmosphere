import ChatUI
import SwiftUI

/// What an app adds to the shared chat's view: cards under replies, a
/// banner above the field, suggestions while the person types, and more.
/// Each part is empty unless the app gives it.
@MainActor
public protocol ChatThreadParts<Message> {
    associatedtype Message: ThreadEntry
    associatedtype Empty: View = EmptyView
    associatedtype Top: View = EmptyView
    associatedtype Extra: View = EmptyView
    associatedtype After: View = EmptyView
    associatedtype Bottom: View = EmptyView
    associatedtype Notices: View = EmptyView
    associatedtype Above: View = EmptyView
    associatedtype Chips: View = EmptyView
    associatedtype Attached: View = EmptyView
    associatedtype Menu: View = EmptyView

    /// What shows before the first message.
    @ViewBuilder func empty() -> Empty
    /// Above the messages, such as why the chat can't reply now.
    @ViewBuilder func top() -> Top
    /// Under a reply, such as what it asks to change.
    @ViewBuilder func extra(_ message: Message) -> Extra
    /// Under any message, after its branches.
    @ViewBuilder func after(_ message: Message) -> After
    /// After the messages and the reply being written.
    @ViewBuilder func bottom() -> Bottom
    /// Above the field, under the messages, each with its own divider.
    @ViewBuilder func notices() -> Notices
    /// Over the field's card, such as what matches what's typed.
    @ViewBuilder func above() -> Above
    /// Beside what's attached to the next message, such as a recording.
    @ViewBuilder func chips() -> Chips
    /// Whether `chips` has something to show now.
    var showsChips: Bool { get }
    /// Beside the photos and files sent with a message.
    @ViewBuilder func attached(_ message: Message) -> Attached
    /// At the end of a message's menu.
    @ViewBuilder func menu(_ message: Message) -> Menu
    /// The name of what the person picked for a message, shown on it.
    func tag(of message: Message) -> String?
    /// Undo for what a reply changed, when it changed something.
    func undo(of message: Message) -> (() -> Void)?
    /// What's picked for the next message, shown at the start of the field,
    /// and how to take it off.
    var token: ChatToken? { get }
    /// The microphone, in place of dictation's.
    var mic: MicButton? { get }
    /// Whether the field shows, so the person can write; without it,
    /// `notices` says why.
    var composes: Bool { get }
}

/// What's picked for the next message, such as a skill, at the start of
/// the field.
public struct ChatToken {
    public var name: String
    /// What the field asks for once it's picked.
    public var prompt: Text?
    public var remove: () -> Void

    public init(name: String, prompt: Text? = nil, remove: @escaping () -> Void) {
        self.name = name
        self.prompt = prompt
        self.remove = remove
    }
}

extension ChatThreadParts {
    public var showsChips: Bool { false }
    public func tag(of message: Message) -> String? { nil }
    public func undo(of message: Message) -> (() -> Void)? { nil }
    public var token: ChatToken? { nil }
    public var mic: MicButton? { nil }
    public var composes: Bool { true }
}

extension ChatThreadParts where Empty == EmptyView {
    public func empty() -> EmptyView { EmptyView() }
}

extension ChatThreadParts where Top == EmptyView {
    public func top() -> EmptyView { EmptyView() }
}

extension ChatThreadParts where Extra == EmptyView {
    public func extra(_ message: Message) -> EmptyView { EmptyView() }
}

extension ChatThreadParts where After == EmptyView {
    public func after(_ message: Message) -> EmptyView { EmptyView() }
}

extension ChatThreadParts where Bottom == EmptyView {
    public func bottom() -> EmptyView { EmptyView() }
}

extension ChatThreadParts where Notices == EmptyView {
    public func notices() -> EmptyView { EmptyView() }
}

extension ChatThreadParts where Above == EmptyView {
    public func above() -> EmptyView { EmptyView() }
}

extension ChatThreadParts where Chips == EmptyView {
    public func chips() -> EmptyView { EmptyView() }
}

extension ChatThreadParts where Attached == EmptyView {
    public func attached(_ message: Message) -> EmptyView { EmptyView() }
}

extension ChatThreadParts where Menu == EmptyView {
    public func menu(_ message: Message) -> EmptyView { EmptyView() }
}

/// The parts of a chat that adds only what shows before the first message
/// and under each reply.
public struct BasicParts<Message: ThreadEntry, Empty: View, Extra: View>: ChatThreadParts {
    let start: Empty
    let reply: (Message) -> Extra

    public func empty() -> Empty { start }
    public func extra(_ message: Message) -> Extra { reply(message) }
}
