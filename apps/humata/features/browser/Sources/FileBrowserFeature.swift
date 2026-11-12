import ComposableArchitecture
import Foundation
import HumataDaemonClient

/// A folder on a host, read a level at a time as folders are opened, and
/// the file picked in it.
@Reducer
package struct FileBrowserFeature {
    @ObservableState
    package struct State: Equatable {
        package var place: HostPlace
        /// The absolute folder browsed from, once read.
        package var root: String?
        /// Each folder read so far, by its path from the root; "" is the root.
        package var folders: [String: [FolderEntry]] = [:]
        /// The folders that had more in them than were listed.
        package var cut: Set<String> = []
        package var expanded: Set<String> = []
        package var selected: String?
        /// The selected file, once read.
        package var contents: FileContents?
        /// Why a folder or file couldn't be read, and what to do about it.
        package var failure: String?

        package init(place: HostPlace) {
            self.place = place
        }

        /// The folders and files shown, in order, each with how deep it is.
        package var rows: [Row] {
            var rows: [Row] = []
            func add(_ directory: String, depth: Int) {
                for entry in folders[directory] ?? [] {
                    let path = FileBrowserFeature.join(directory, entry.name)
                    rows.append(Row(path: path, entry: entry, depth: depth))
                    if entry.isDirectory, expanded.contains(path) { add(path, depth: depth + 1) }
                }
            }
            add("", depth: 0)
            return rows
        }

        /// The selected file's absolute path on its host.
        package var absolutePath: String? {
            guard let root, let selected else { return nil }
            return FileBrowserFeature.join(root, selected)
        }
    }

    package struct Row: Equatable, Identifiable {
        package var path: String
        package var entry: FolderEntry
        package var depth: Int

        package var id: String { path }
    }

    package enum Action {
        /// It appeared: reads the root.
        case task
        case refreshTapped
        case folderRead(Folder)
        /// Opens a folder, reading it the first time, or closes it.
        case folderToggled(String)
        case fileSelected(String)
        case fileRead(FileContents)
        case failed(String)
    }

    @Dependency(\.daemon) var daemon

    package init() {}

    static func join(_ directory: String, _ name: String) -> String {
        if directory.isEmpty { return name }
        return directory.hasSuffix("/") ? directory + name : directory + "/" + name
    }

    package var body: some ReducerOf<Self> {
        Reduce { state, action in
            switch action {
            case .task:
                guard state.root == nil else { return .none }
                return read("", at: state.place)

            case .refreshTapped:
                // Reads again every folder that is open, and the file shown.
                let open = [""] + state.expanded.sorted()
                let place = state.place
                var effects = open.map { read($0, at: place) }
                if let selected = state.selected { effects.append(readFile(selected, at: place)) }
                return .merge(effects)

            case .folderRead(let folder):
                state.root = folder.root
                state.failure = nil
                state.folders[folder.directory] = folder.entries
                if folder.truncated {
                    state.cut.insert(folder.directory)
                } else {
                    state.cut.remove(folder.directory)
                }
                return .none

            case .folderToggled(let path):
                if state.expanded.remove(path) != nil { return .none }
                state.expanded.insert(path)
                guard state.folders[path] == nil else { return .none }
                return read(path, at: state.place)

            case .fileSelected(let path):
                guard state.selected != path else { return .none }
                state.selected = path
                state.contents = nil
                return readFile(path, at: state.place)

            case .fileRead(let contents):
                guard contents.path == state.selected else { return .none }
                state.failure = nil
                state.contents = contents
                return .none

            case .failed(let message):
                state.failure = message
                return .none
            }
        }
    }

    private func read(_ directory: String, at place: HostPlace) -> Effect<Action> {
        .run { send in
            await send(.folderRead(try await daemon.listFolder(place, directory)))
        } catch: { error, send in
            await send(.failed(error.localizedDescription))
        }
    }

    private func readFile(_ path: String, at place: HostPlace) -> Effect<Action> {
        .run { send in
            await send(.fileRead(try await daemon.readFileAt(place, path)))
        } catch: { error, send in
            await send(.failed(error.localizedDescription))
        }
    }
}
