import ComposableArchitecture
import Foundation
import HumataDaemonClient
import SwiftUI

/// The plan beside a session's chat: its project's items in the order to
/// take them, each after what it waits on and a milestone's tasks right
/// under it, with their statuses and whether they're blocked. Shown or put
/// away from the header, which every session remembers; read only while
/// shown, as often as the board.
@Reducer
package struct PlanPanelFeature {
    @ObservableState
    package struct State: Equatable {
        /// Nil for a session in no project, which has no plan.
        package var projectID: Project.ID?
        /// The item the session works on, marked in the plan.
        package var itemID: Item.ID?
        /// Whether it shows beside the chat; one setting for every session.
        @Shared(.appStorage("planPanelShown")) package var shown = false
        /// The items in the plan's order, sorted once as they're read.
        package var planned: [Item] = []
        /// Read at least once, so none means the board has none.
        package var loaded = false
        /// Why the plan couldn't be read, and what to do; nil once it was.
        package var failure: String?

        package init(projectID: Project.ID?, itemID: Item.ID?) {
            self.projectID = projectID
            self.itemID = itemID
        }
    }

    package enum Action {
        /// The panel appeared: reads the plan now and again as often as the
        /// board does, until it goes.
        case task
        case loaded([Item])
        case failed(String)
        case toggleTapped
        case itemTapped(Item.ID)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate: Equatable {
            case openItem(Item.ID)
        }
    }

    /// As often as the board is read again.
    static let refreshInterval = Duration.seconds(2)

    @Dependency(\.daemon) var daemon
    @Dependency(\.continuousClock) var clock

    package init() {}

    package var body: some ReducerOf<Self> {
        Reduce { state, action in
            switch action {
            case .task:
                guard let project = state.projectID else { return .none }
                return .run { send in
                    // Most reads show what the last did; only one with
                    // something new is sent, so the panel isn't drawn again.
                    var last: Result<[Item], Failure>?
                    while !Task.isCancelled {
                        let read: Result<[Item], Failure>
                        do {
                            read = .success(try await daemon.items(project))
                        } catch {
                            read = .failure(Failure(message: error.localizedDescription))
                        }
                        if read != last {
                            last = read
                            switch read {
                            case .success(let items): await send(.loaded(items))
                            case .failure(let failure): await send(.failed(failure.message))
                            }
                        }
                        try await clock.sleep(for: Self.refreshInterval)
                    }
                }

            case .loaded(let items):
                state.loaded = true
                state.failure = nil
                state.planned = items.enumerated()
                    .sorted { ($0.element.planOrder, $0.offset) < ($1.element.planOrder, $1.offset) }
                    .map(\.element)
                return .none

            case .failed(let message):
                state.failure = message
                return .none

            case .toggleTapped:
                state.$shown.withLock { $0.toggle() }
                return .none

            case .itemTapped(let id):
                return .send(.delegate(.openItem(id)))

            case .delegate:
                return .none
            }
        }
    }

    /// A read that failed, told apart from the next by its message.
    private struct Failure: Error, Equatable {
        var message: String
    }
}

/// The header's button that shows or puts away the plan.
struct PlanPanelToggle: View {
    let store: StoreOf<PlanPanelFeature>

    var body: some View {
        Button(store.shown ? "Hide Plan" : "Show Plan", systemImage: "list.bullet.indent") {
            store.send(.toggleTapped)
        }
        .labelStyle(.iconOnly)
        .buttonStyle(.borderless)
        .help(
            store.shown
                ? "Put away the project's plan"
                : "Show the project's items beside the chat, in the order to take them")
    }
}

/// The plan, as a list beside the chat.
struct PlanPanelView: View {
    let store: StoreOf<PlanPanelFeature>

    var body: some View {
        List {
            if let failure = store.failure {
                Label("Couldn't read the plan: \(failure)", systemImage: "exclamationmark.triangle.fill")
                    .foregroundStyle(.secondary)
            }
            if store.loaded, store.planned.isEmpty {
                Text("No items on the board yet. Add them on the board, or have the agent propose them.")
                    .foregroundStyle(.secondary)
            }
            ForEach(store.planned) { item in
                row(item)
            }
        }
        .listStyle(.sidebar)
        .safeAreaInset(edge: .top, spacing: 0) {
            Text("Plan")
                .font(.headline)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
        }
        .task { await store.send(.task).finish() }
    }

    private func row(_ item: Item) -> some View {
        Button {
            store.send(.itemTapped(item.id))
        } label: {
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                Image(systemName: symbol(item))
                    .foregroundStyle(item.blocked ? Color.orange : item.closed ? Color.green : Color.secondary)
                    .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: 2) {
                    Text(item.title)
                        .font(item.isMilestone ? .callout.bold() : .callout)
                        .lineLimit(2)
                        .strikethrough(item.fields.status == .wontDo)
                    Text(item.blocked ? "\(item.status.title), blocked" : item.status.title)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            .padding(.leading, item.parentID == nil ? 0 : 14)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .listRowBackground(item.id == store.itemID ? Color.accentColor.opacity(0.15) : nil)
        .help(item.id == store.itemID ? "This session's item; open it on the board" : "Open \(item.title) on the board")
    }

    private func symbol(_ item: Item) -> String {
        if item.closed { return "checkmark.circle.fill" }
        if item.blocked { return "lock" }
        return item.isMilestone ? "flag" : "circle"
    }
}
