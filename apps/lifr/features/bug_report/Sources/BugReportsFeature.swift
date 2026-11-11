import ComposableArchitecture
import Foundation
import LifrPasteboardClient

/// Settings › Bug Reports: the reports saved on this device, to view,
/// copy, share or delete.
@Reducer
package struct BugReportsFeature {
    @ObservableState
    package struct State: Equatable {
        package var saved = SavedReports()
        package var isLoaded = false
        /// The report being viewed.
        package var opened: BugReport?
        package var copied: BugReport.ID?
        package var error: BugReportError?

        package init(saved: SavedReports = SavedReports(), isLoaded: Bool = false) {
            self.saved = saved
            self.isLoaded = isLoaded
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case task
        case loaded(Result<SavedReports, BugReportError>)
        case openTapped(BugReport.ID)
        case copyTapped(BugReport.ID)
        case deleteTapped(UUID)
        case deleted(UUID, Result<Void, BugReportError>)
    }

    @Dependency(\.bugReports) private var reports
    @Dependency(\.pasteboard) private var pasteboard

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .binding:
                return .none

            case .task:
                return .run { [reports] send in
                    await send(
                        .loaded(
                            Result { () async throws(BugReportError) in
                                try await BugReportError.wrapping { try await reports.load() }
                            }))
                }

            case .loaded(.success(let saved)):
                state.saved = saved
                state.isLoaded = true
                state.error = nil
                return .none

            case .loaded(.failure(let error)):
                state.isLoaded = true
                state.error = error
                return .none

            case .openTapped(let id):
                state.opened = state.saved.reports.first { $0.id == id }
                return .none

            case .copyTapped(let id):
                guard let report = state.saved.reports.first(where: { $0.id == id }) else { return .none }
                state.copied = id
                return .run { [pasteboard, text = report.text] _ in await pasteboard.copy(text) }

            case .deleteTapped(let id):
                return .run { [reports] send in
                    await send(
                        .deleted(
                            id,
                            Result { () async throws(BugReportError) in
                                try await BugReportError.wrapping { try await reports.delete(id) }
                            }))
                }

            case .deleted(let id, .success):
                state.saved.reports.removeAll { $0.id == id }
                state.saved.unreadable.removeAll { $0.id == id }
                if state.opened?.id == id { state.opened = nil }
                state.error = nil
                return .none

            case .deleted(_, .failure(let error)):
                state.error = error
                return .none
            }
        }
    }
}
