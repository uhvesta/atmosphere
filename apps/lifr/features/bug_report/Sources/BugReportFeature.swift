import ComposableArchitecture
import Foundation
import LifrModelsClient

/// Describing a problem: what happened and what was expected, whether the
/// app's text and screenshots go with it, then Save, Copy or Share.
@Reducer
package struct BugReportFeature {
    @ObservableState
    package struct State: Equatable {
        /// Everything recorded, before redaction.
        package var report: BugReport
        package var isSaving = false
        package var copied = false
        package var error: BugReportError?

        package init(report: BugReport) {
            self.report = report
        }

        /// A new report on this device, with `recording` if there was one.
        /// Call it from a reducer, so it reads the reducer's dependencies.
        package static func new(recording: BugReport.Recording?) -> Self {
            @Dependency(\.uuid) var uuid
            @Dependency(\.date.now) var now
            @Dependency(\.deviceInfo) var deviceInfo
            @Dependency(\.models) var models
            var device = deviceInfo.current()
            device.assistantModel = models.selection(.assistant).rawValue
            return State(report: BugReport(id: uuid(), createdAt: now, device: device, recording: recording))
        }

        /// What Save, Copy and Share give: without the app's text and the
        /// screenshots unless the person included them.
        package var kept: BugReport { Redactor.redact(report) }

        package var canSave: Bool {
            !isSaving && !report.happened.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case saveTapped
        case saved(Result<Void, BugReportError>)
        case copyTapped
        case cancelTapped
    }

    @Dependency(\.bugReports) private var reports
    @Dependency(\.pasteboard) private var pasteboard
    @Dependency(\.dismiss) private var dismiss

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .binding:
                state.copied = false
                state.error = nil
                return .none

            case .saveTapped:
                guard state.canSave else { return .none }
                state.isSaving = true
                state.error = nil
                return .run { [reports, kept = state.kept] send in
                    await send(
                        .saved(
                            Result { () async throws(BugReportError) in
                                try await BugReportError.wrapping { try await reports.save(kept) }
                            }))
                }

            case .saved(.success):
                state.isSaving = false
                return .run { [dismiss] _ in await dismiss() }

            case .saved(.failure(let error)):
                state.isSaving = false
                state.error = error
                return .none

            case .copyTapped:
                state.copied = true
                return .run { [pasteboard, text = state.kept.text] _ in await pasteboard.copy(text) }

            case .cancelTapped:
                return .run { [dismiss] _ in await dismiss() }
            }
        }
    }
}

extension BugReportError {
    /// Runs `body`, which throws only `BugReportError`s.
    static func wrapping<Value>(_ body: () async throws -> Value) async throws(BugReportError) -> Value {
        do {
            return try await body()
        } catch let error as BugReportError {
            throw error
        } catch {
            throw .fileSystem(operation: .write, path: "BugReports", reason: error.localizedDescription)
        }
    }
}
