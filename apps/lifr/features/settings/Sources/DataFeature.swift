import ComposableArchitecture
import Foundation
import LifrBackupClient
import LifrCore
import LifrStoreClient

/// Settings › Data: export everything, import a Lifr export or a CSV file
/// with a look at what it adds first, and the automatic backups.
@Reducer
package struct DataFeature {
    @ObservableState
    package struct State: Equatable {
        package var workspace: Workspace
        package var backups: [Backup] = []
        /// The file the exporter is saving, while it shows.
        package var export: ExportFile?
        /// A Lifr file's import, waiting for the person to confirm it.
        package var preview: Preview?
        /// A CSV file's import, waiting for its columns.
        package var csv: CSVImport?
        /// The last import, for Undo, and how many records it added.
        package var imported: Imported?
        package var isWorking = false
        @Presents package var alert: AlertState<Never>?

        package init(workspace: Workspace) {
            self.workspace = workspace
        }

        /// Records saved elsewhere, such as by sync or another window.
        package mutating func show(_ snapshot: Snapshot) {
            workspace = snapshot.workspace
        }
    }

    /// A file to save: one JSON file, or one Markdown file per Avenue.
    package enum ExportFile: Equatable, Sendable {
        case everything(name: String, Data)
        case markdown([String: Data])
    }

    /// What a Lifr file would add.
    package struct Preview: Equatable, Sendable {
        /// The file's name, or the backup's day.
        package var source: String
        package var plan: ImportPlan
    }

    /// A CSV file and which columns hold what.
    package struct CSVImport: Equatable, Sendable {
        package var source: String
        package var table: CSVTable
        package var mapping: CSVMapping
        /// The Avenues rows can go to, open ones first by their order.
        package var avenues: [Avenue]
    }

    package struct Imported: Equatable, Sendable {
        package var entry: UUID
        package var count: Int
    }

    /// Why importing or exporting failed, as the person reads it.
    package struct Failure: Error, Equatable {
        package var title: String
        package var reason: String

        static func export(_ reason: String) -> Self {
            Self(title: String(localized: "Couldn't export"), reason: reason)
        }
        static func open(_ reason: String) -> Self {
            Self(title: String(localized: "Couldn't open the file"), reason: reason)
        }
        static func openBackup(_ reason: String) -> Self {
            Self(title: String(localized: "Couldn't open the backup"), reason: reason)
        }
        static func importing(_ reason: String) -> Self {
            Self(title: String(localized: "Couldn't import"), reason: reason)
        }
        static func undo(_ reason: String) -> Self {
            Self(title: String(localized: "Couldn't undo the import"), reason: reason)
        }
    }

    package enum Action {
        case appeared
        case backupsListed([Backup])
        case exportTapped
        case markdownTapped
        /// The exporter closed, saved or not.
        case exportFinished(Result<Void, any Error>)
        /// A file picked to import, or a backup's bytes.
        case fileRead(source: String, Data, isCSV: Bool)
        case filePicked(Result<URL, any Error>)
        case backupTapped(CalendarDate)
        case importConfirmed
        case importCancelled
        case csvMapped(CSVMapping)
        case csvConfirmed
        case csvCancelled
        case saved(Saved, count: Int)
        case undoTapped
        case undone(Saved)
        case failed(Failure)
        case alert(PresentationAction<Never>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate {
            case saved(Snapshot)
        }
    }

    @Dependency(\.store) private var store
    @Dependency(\.backups) private var backups
    @Dependency(\.date.now) private var now
    @Dependency(\.timeZone) private var timeZone

    package init() {}

    package var body: some ReducerOf<Self> {
        Reduce { state, action in
            switch action {
            case .alert, .delegate:
                return .none

            case .appeared:
                return .run { [backups] send in await send(.backupsListed(await backups.list())) }

            case .backupsListed(let list):
                state.backups = list
                return .none

            case .exportTapped:
                do throws(ImportError) {
                    let data = try state.workspace.exported(at: now).encoded()
                    let day = CalendarDate(now, in: timeZone)
                    state.export = .everything(name: "Lifr \(day.description).json", data)
                } catch {
                    state.alert = Self.alert(Failure.export(error.description))
                }
                return .none

            case .markdownTapped:
                var files: [String: Data] = [:]
                for id in state.workspace.avenues.keys {
                    guard let name = state.workspace.markdownFileName(of: id),
                        let text = state.workspace.markdown(of: id)
                    else { continue }
                    files[name] = Data(text.utf8)
                }
                state.export = .markdown(files)
                return .none

            case .exportFinished(.success):
                state.export = nil
                return .none

            case .exportFinished(.failure(let error)):
                state.export = nil
                state.alert = Self.alert(Failure.export(error.localizedDescription))
                return .none

            case .filePicked(.success(let url)):
                return .run { send in
                    let scoped = url.startAccessingSecurityScopedResource()
                    defer { if scoped { url.stopAccessingSecurityScopedResource() } }
                    do {
                        let data = try Data(contentsOf: url)
                        let isCSV = ["csv", "tsv", "txt"].contains(url.pathExtension.lowercased())
                        await send(.fileRead(source: url.lastPathComponent, data, isCSV: isCSV))
                    } catch {
                        await send(.failed(Failure.open(error.localizedDescription)))
                    }
                }

            case .filePicked(.failure(let error)):
                state.alert = Self.alert(Failure.open(error.localizedDescription))
                return .none

            case .backupTapped(let day):
                return .run { [backups] send in
                    do throws(BackupError) {
                        let data = try await backups.read(day)
                        await send(.fileRead(source: day.description, data, isCSV: false))
                    } catch {
                        await send(.failed(Failure.openBackup(error.description)))
                    }
                }

            case .fileRead(let source, let data, isCSV: true):
                do throws(ImportError) {
                    guard let text = String(data: data, encoding: .utf8) ?? String(data: data, encoding: .isoLatin1)
                    else { throw ImportError.unreadable(line: nil, String(localized: "it isn't text")) }
                    let table = try CSVTable(parsing: text)
                    guard let mapping = CSVMapping(guessingFrom: table.columns) else {
                        throw ImportError.unreadable(line: 1, String(localized: "it has no columns"))
                    }
                    let avenues = state.workspace.avenues.values.filter { $0.archivedAt == nil }
                        .sorted { ($0.sortOrder, $0.id.uuidString) < ($1.sortOrder, $1.id.uuidString) }
                    state.csv = CSVImport(source: source, table: table, mapping: mapping, avenues: avenues)
                } catch {
                    state.alert = Self.alert(Failure.importing(error.description))
                }
                return .none

            case .fileRead(let source, let data, isCSV: false):
                do throws(ImportError) {
                    let plan = try state.workspace.importPlan(LifrExport.decoded(from: data))
                    state.preview = Preview(source: source, plan: plan)
                } catch {
                    state.alert = Self.alert(Failure.importing(error.description))
                }
                return .none

            case .importConfirmed:
                guard let preview = state.preview, !preview.plan.isEmpty else {
                    state.preview = nil
                    return .none
                }
                state.preview = nil
                state.isWorking = true
                let plan = preview.plan
                return .run { [store] send in
                    let saved = try await store.save { workspace, context in workspace.apply(plan, in: context) }
                    await send(.saved(saved, count: plan.records.count))
                } catch: { error, send in
                    await send(.failed(Failure.importing("\(error)")))
                }

            case .importCancelled:
                state.preview = nil
                return .none

            case .csvConfirmed:
                guard let csv = state.csv else { return .none }
                let tasks: [Workspace.NewTask]
                do throws(ImportError) {
                    tasks = try state.workspace.newTasks(from: csv.table, mapping: csv.mapping)
                } catch {
                    // The sheet stays, so another column can be picked.
                    state.alert = Self.alert(Failure.importing(error.description))
                    return .none
                }
                state.csv = nil
                state.isWorking = true
                return .run { [store] send in
                    await send(.saved(try await store.createTasks(tasks), count: tasks.count))
                } catch: { error, send in
                    await send(.failed(Failure.importing("\(error)")))
                }

            case .csvMapped(let mapping):
                state.csv?.mapping = mapping
                return .none

            case .csvCancelled:
                state.csv = nil
                return .none

            case .saved(let saved, let count):
                state.isWorking = false
                state.workspace = saved.workspace
                state.imported = saved.change.entry.map { Imported(entry: $0, count: count) }
                return .send(.delegate(.saved(saved.snapshot)))

            case .undoTapped:
                guard let imported = state.imported else { return .none }
                state.imported = nil
                state.isWorking = true
                return .run { [store] send in
                    await send(.undone(try await store.revert(entries: [imported.entry])))
                } catch: { error, send in
                    await send(.failed(Failure.undo("\(error)")))
                }

            case .undone(let saved):
                state.isWorking = false
                state.workspace = saved.workspace
                return .send(.delegate(.saved(saved.snapshot)))

            case .failed(let failure):
                state.isWorking = false
                state.alert = Self.alert(failure)
                return .none
            }
        }
        .ifLet(\.$alert, action: \.alert)
    }

    private static func alert(_ failure: Failure) -> AlertState<Never> {
        AlertState {
            TextState(failure.title)
        } message: {
            TextState(failure.reason)
        }
    }
}
