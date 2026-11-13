import ComposableArchitecture
import Foundation
import HumataDaemonClient

/// The specs a session reads, in the order its agent gets them: the
/// project's instructions, the task above its subtask, and its own item's.
/// Each can be edited here and is saved to the board, which tells the
/// session as any spec edit does.
@Reducer
package struct SpecsFeature {
    /// One spec the session reads.
    package struct Page: Identifiable, Equatable, Sendable {
        package enum Kind: Equatable, Sendable {
            case project
            case task
            case subtask
        }

        /// The project's or the item's id.
        package var id: String
        package var kind: Kind
        package var title: String
        package var text: String

        package init(id: String, kind: Kind, title: String, text: String) {
            self.id = id
            self.kind = kind
            self.title = title
            self.text = text
        }
    }

    /// A spec being edited.
    package struct Draft: Equatable, Sendable {
        package var pageID: String
        package var text: String
        package var previewing = false
        package var saving = false
    }

    @ObservableState
    package struct State: Equatable {
        package var projectID: String?
        package var itemID: String?
        package var pages: IdentifiedArrayOf<Page> = []
        /// Until the first load answers.
        package var loading = false
        package var draft: Draft?
        @Presents package var alert: AlertState<Never>?

        package init(projectID: String?, itemID: String?) {
            self.projectID = projectID
            self.itemID = itemID
        }
    }

    package enum Action: Equatable {
        case refresh
        case loaded([Page])
        case failed(String)
        case editTapped(String)
        case draftChanged(String)
        case previewToggled(Bool)
        case cancelTapped
        case saveTapped
        case saved(Page)
        case saveFailed(String)
        case openTapped(String)
        case alert(PresentationAction<Never>)
        case delegate(Delegate)

        package enum Delegate: Equatable {
            /// The person wants the item on the board.
            case openItem(String)
        }
    }

    @Dependency(\.daemon) var daemon

    package init() {}

    package var body: some ReducerOf<Self> {
        Reduce { state, action in
            switch action {
            case .refresh:
                guard let projectID = state.projectID else { return .none }
                state.loading = state.pages.isEmpty
                let itemID = state.itemID
                return .run { [daemon] send in
                    do {
                        await send(.loaded(try await Self.pages(daemon, projectID, itemID)))
                    } catch {
                        await send(.failed(error.localizedDescription))
                    }
                }

            case .loaded(let pages):
                state.loading = false
                state.pages = IdentifiedArray(uniqueElements: pages)
                return .none

            case .failed(let message):
                state.loading = false
                state.alert = AlertState {
                    TextState("Couldn’t read this session’s specs")
                } message: {
                    TextState("\(message)\n\nCheck that Humata’s daemon is running, then open the Spec tab again.")
                }
                return .none

            case .editTapped(let id):
                guard let page = state.pages[id: id] else { return .none }
                state.draft = Draft(pageID: id, text: page.text)
                return .none

            case .draftChanged(let text):
                state.draft?.text = text
                return .none

            case .previewToggled(let previewing):
                state.draft?.previewing = previewing
                return .none

            case .cancelTapped:
                state.draft = nil
                return .none

            case .saveTapped:
                guard var draft = state.draft, !draft.saving, let page = state.pages[id: draft.pageID] else {
                    return .none
                }
                draft.saving = true
                state.draft = draft
                let text = draft.text
                return .run { [daemon] send in
                    do {
                        switch page.kind {
                        case .project:
                            _ = try await daemon.updateProject(page.id, ProjectUpdate(instructions: text))
                        case .task, .subtask:
                            _ = try await daemon.updateItem(page.id, ItemUpdate(spec: text))
                        }
                        var saved = page
                        saved.text = text
                        await send(.saved(saved))
                    } catch {
                        await send(.saveFailed(error.localizedDescription))
                    }
                }

            case .saved(let page):
                state.pages[id: page.id] = page
                state.draft = nil
                return .none

            case .saveFailed(let message):
                state.draft?.saving = false
                state.alert = AlertState {
                    TextState("Couldn’t save the spec")
                } message: {
                    TextState("\(message)\n\nYour edit is still here; try Save again once Humata’s daemon answers.")
                }
                return .none

            case .openTapped(let id):
                return .send(.delegate(.openItem(id)))

            case .alert, .delegate:
                return .none
            }
        }
        .ifLet(\.$alert, action: \.alert)
    }

    /// The specs a session on `itemID` in `projectID` reads, in order.
    private static func pages(_ daemon: DaemonClient, _ projectID: String, _ itemID: String?) async throws -> [Page] {
        async let projects = daemon.projects()
        async let items = itemID == nil ? [] : daemon.items(projectID)
        var pages: [Page] = []
        if let project = try await projects.first(where: { $0.id == projectID }) {
            pages.append(Page(id: project.id, kind: .project, title: project.name, text: project.instructions))
        }
        let board = try await items
        if let itemID, let item = board.first(where: { $0.id == itemID }) {
            if let parentID = item.fields.parentID, let parent = board.first(where: { $0.id == parentID }) {
                pages.append(Page(id: parent.id, kind: .task, title: parent.title, text: parent.spec))
                pages.append(Page(id: item.id, kind: .subtask, title: item.title, text: item.spec))
            } else {
                pages.append(Page(id: item.id, kind: .task, title: item.title, text: item.spec))
            }
        }
        return pages
    }
}
