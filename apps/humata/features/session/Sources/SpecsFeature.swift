import ComposableArchitecture
import Foundation
import HumataDaemonClient
import HumataReview

/// The specs a session reads, in the order its agent gets them: the
/// project's instructions, the task above its subtask, and its own item's.
/// Each can be edited here and is saved to the board, which tells the
/// session as any spec edit does. Focused, one is shown wide beside the
/// chat, with what an edit changed marked and the versions the board's
/// journal kept.
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

    /// A spec as one change on the board left it.
    package struct Version: Identifiable, Equatable, Sendable {
        /// The board change that wrote it.
        package var id: UInt64
        package var time: Date
        package var author: BoardAuthor
        package var text: String

        package init(id: UInt64, time: Date, author: BoardAuthor, text: String) {
            self.id = id
            self.time = time
            self.author = author
            self.text = text
        }
    }

    /// The focused spec's versions, one of them shown, on its own or
    /// compared with the spec now.
    package struct Versions: Equatable, Sendable {
        package var pageID: String
        /// Newest first; nil until they're read.
        package var list: [Version]?
        /// The version shown; nil shows the spec now.
        package var selected: Version.ID?
        package var comparing = false
        package var comparison: [ComparedBlock]?
        package var restoring = false

        package init(pageID: String) {
            self.pageID = pageID
        }

        package var shown: Version? {
            list?.first { $0.id == selected }
        }
    }

    package enum Alert: Equatable, Sendable {
        case restore(pageID: String, version: Version.ID)
    }

    @ObservableState
    package struct State: Equatable {
        package var projectID: String?
        package var itemID: String?
        package var pages: IdentifiedArrayOf<Page> = []
        /// Until the first load answers.
        package var loading = false
        package var draft: Draft?
        /// The spec shown wide beside the chat, by its page's id.
        package var focused: String?
        /// The focused spec's lines, from 1, that changed when it last
        /// read again; cleared when another spec is focused.
        package var changed: Set<Int> = []
        /// The focused spec's versions, while they're shown.
        package var versions: Versions?
        @Presents package var alert: AlertState<Alert>?

        package init(projectID: String?, itemID: String?) {
            self.projectID = projectID
            self.itemID = itemID
        }
    }

    package enum Action: Equatable {
        case refresh
        /// The specs read, with the focused one's lines that changed.
        case loaded([Page], changed: Set<Int>)
        case failed(String)
        case editTapped(String)
        case draftChanged(String)
        case previewToggled(Bool)
        case cancelTapped
        case saveTapped
        case saved(Page)
        case saveFailed(String)
        case openTapped(String)
        /// Shows one spec wide beside the chat; nil shows them all again.
        case focusTapped(String?)
        case versionsToggled
        case versionsListed(pageID: String, [Version])
        case versionsFailed(VersionsDoing, String)
        /// A version to show; nil for the spec now.
        case versionPicked(Version.ID?)
        case compareToggled
        case compared(pageID: String, version: Version.ID, [ComparedBlock])
        /// Asks before the shown version is saved over the spec.
        case restoreTapped
        case restored(Page)
        case alert(PresentationAction<Alert>)
        case delegate(Delegate)

        package enum Delegate: Equatable {
            /// The person wants the item on the board.
            case openItem(String)
        }
    }

    /// What was being done with the spec's versions when it failed.
    package enum VersionsDoing: Equatable, Sendable {
        case listing
        case restoring
    }

    @Dependency(\.daemon) var daemon

    private enum CancelID {
        case versions
        case compare
    }

    package init() {}

    package var body: some ReducerOf<Self> {
        Reduce { state, action in
            switch action {
            case .refresh:
                guard let projectID = state.projectID else { return .none }
                state.loading = state.pages.isEmpty
                let itemID = state.itemID
                let focused = state.focused.flatMap { state.pages[id: $0] }
                let load: Effect<Action> = .run { [daemon] send in
                    do {
                        let pages = try await Self.pages(daemon, projectID, itemID)
                        // Off the main thread: a long spec's diff takes a moment.
                        var changed: Set<Int> = []
                        if let old = focused, let now = pages.first(where: { $0.id == old.id }) {
                            changed = MarkdownComparison.changedLines(
                                from: Self.lines(old.text), to: Self.lines(now.text))
                        }
                        await send(.loaded(pages, changed: changed))
                    } catch {
                        await send(.failed(error.localizedDescription))
                    }
                }
                return .merge(load, listVersions(state))

            case .loaded(let pages, let changed):
                state.loading = false
                if let focused = state.focused,
                    pages.first(where: { $0.id == focused })?.text
                        != state.pages[id: focused]?.text
                {
                    state.changed = changed
                }
                state.pages = IdentifiedArray(uniqueElements: pages)
                if let focused = state.focused, state.pages[id: focused] == nil {
                    state.focused = nil
                    state.versions = nil
                }
                return compare(state)

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
                return listVersions(state)

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

            case .focusTapped(let id):
                guard state.focused != id else { return .none }
                state.focused = id.flatMap { state.pages[id: $0]?.id }
                state.changed = []
                state.versions = nil
                return .cancel(id: CancelID.versions)

            case .versionsToggled:
                guard state.versions == nil, let focused = state.focused else {
                    state.versions = nil
                    return .cancel(id: CancelID.versions)
                }
                state.versions = Versions(pageID: focused)
                return listVersions(state)

            case .versionsListed(let pageID, let list):
                guard state.versions?.pageID == pageID else { return .none }
                state.versions?.list = list
                if let selected = state.versions?.selected, !list.contains(where: { $0.id == selected }) {
                    state.versions?.selected = nil
                }
                return compare(state)

            case .versionsFailed(let doing, let message):
                let title: String
                let advice: String
                switch doing {
                case .listing:
                    if state.versions?.list == nil { state.versions = nil }
                    title = String(localized: "Couldn’t list the spec’s versions")
                    advice = String(
                        localized: "The spec is unchanged. Show its versions again once what’s said above is fixed.")
                case .restoring:
                    state.versions?.restoring = false
                    title = String(localized: "Couldn’t restore the version")
                    advice = String(
                        localized: """
                            The spec shown may be older than the board’s: check the task on the board before \
                            restoring again.
                            """)
                }
                state.alert = AlertState {
                    TextState(title)
                } message: {
                    TextState("\(message)\n\n\(advice)")
                }
                return .none

            case .versionPicked(let id):
                guard state.versions != nil, state.versions?.selected != id else { return .none }
                state.versions?.selected = id
                state.versions?.comparison = nil
                return compare(state)

            case .compareToggled:
                guard state.versions != nil else { return .none }
                state.versions?.comparing.toggle()
                state.versions?.comparison = nil
                return compare(state)

            case .compared(let pageID, let version, let blocks):
                guard state.versions?.pageID == pageID, state.versions?.selected == version,
                    state.versions?.comparing == true
                else { return .none }
                state.versions?.comparison = blocks
                return .none

            case .restoreTapped:
                guard let versions = state.versions, let version = versions.shown, !versions.restoring,
                    let page = state.pages[id: versions.pageID]
                else { return .none }
                state.alert = AlertState {
                    TextState("Restore this version?")
                } actions: {
                    ButtonState(role: .destructive, action: .restore(pageID: page.id, version: version.id)) {
                        TextState("Restore")
                    }
                    ButtonState(role: .cancel) {
                        TextState("Cancel")
                    }
                } message: {
                    TextState(
                        "“\(page.title)” is saved as this version, over the spec as it is now. It’s a board change "
                            + "of its own, so the session is told, and the spec as it is now stays in the list.")
                }
                return .none

            case .alert(.presented(.restore(let pageID, let id))):
                guard state.versions?.pageID == pageID, let page = state.pages[id: pageID],
                    let version = state.versions?.list?.first(where: { $0.id == id })
                else { return .none }
                state.versions?.restoring = true
                return .run { [daemon] send in
                    do {
                        _ = try await daemon.updateItem(page.id, ItemUpdate(spec: version.text))
                        var restored = page
                        restored.text = version.text
                        await send(.restored(restored))
                    } catch {
                        await send(.versionsFailed(.restoring, error.localizedDescription))
                    }
                }

            case .restored(let page):
                let old = state.pages[id: page.id]?.text ?? ""
                state.pages[id: page.id] = page
                state.versions?.restoring = false
                state.versions?.selected = nil
                state.versions?.comparing = false
                state.versions?.comparison = nil
                // A spec is short: marked here, not in an effect.
                state.changed = MarkdownComparison.changedLines(from: Self.lines(old), to: Self.lines(page.text))
                return listVersions(state)

            case .alert, .delegate:
                return .none
            }
        }
        .ifLet(\.$alert, action: \.alert)
    }

    /// The focused spec's versions read, while they're shown: each board
    /// change to its spec, newest first. Project instructions aren't on the
    /// board, so they have none.
    private func listVersions(_ state: State) -> Effect<Action> {
        guard let pageID = state.versions?.pageID, let projectID = state.projectID else { return .none }
        guard state.pages[id: pageID]?.kind != .project else {
            return .send(.versionsListed(pageID: pageID, []))
        }
        return .run { [daemon] send in
            do {
                let versions = try await daemon.itemLog(projectID, pageID).compactMap { change -> Version? in
                    guard let after = change.after, after.spec != change.before?.spec
                    else { return nil }
                    return Version(id: change.id, time: change.time, author: change.author, text: after.spec)
                }
                await send(.versionsListed(pageID: pageID, versions))
            } catch {
                await send(.versionsFailed(.listing, error.localizedDescription))
            }
        }
        .cancellable(id: CancelID.versions, cancelInFlight: true)
    }

    /// The version shown and the spec now compared, when that's asked for.
    private func compare(_ state: State) -> Effect<Action> {
        guard let versions = state.versions, versions.comparing, let version = versions.shown,
            let page = state.pages[id: versions.pageID]
        else { return .none }
        return .run { send in
            let blocks = MarkdownComparison.compare(Self.lines(version.text), with: Self.lines(page.text))
            await send(.compared(pageID: page.id, version: version.id, blocks))
        }
        .cancellable(id: CancelID.compare, cancelInFlight: true)
    }

    /// `text` split into lines, without the empty one after a last newline.
    static func lines(_ text: String) -> [String] {
        var lines = text.components(separatedBy: "\n")
        if lines.last == "" { lines.removeLast() }
        return lines
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
