import ComposableArchitecture
import Foundation
import HumataDaemonClient
import HumataReview

/// A folder on a host, read a level at a time as folders are opened, and
/// the file picked in it.
@Reducer
package struct FileBrowserFeature {
    /// Identified by its place, so another place's files shown in its stead
    /// stop what this one was reading instead of finishing it into them.
    @ObservableState
    package struct State: Equatable, Identifiable {
        package var id: [String?] { [place.host, place.sessionID, place.folder] }
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
        /// The selected file's text by line, split where it was read; empty
        /// when it isn't text.
        package var lines: [String] = []
        /// Why a folder couldn't be read, or the file shown read again, and
        /// what to do about it. While `root` is nil it's why nothing lists.
        package var failure: String?
        /// Why the selected file couldn't be read, while nothing of it shows.
        package var unread: String?

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
        /// The file, and its text by line when it is text.
        case fileRead(FileContents, lines: [String])
        /// The folder at this path from the root couldn't be read.
        case failed(String, String)
        /// The file at this path couldn't be read.
        case fileFailed(String, String)
    }

    @Dependency(\.daemon) var daemon

    package init() {}

    static func join(_ directory: String, _ name: String) -> String {
        if directory.isEmpty { return name }
        return directory.hasSuffix("/") ? directory + name : directory + "/" + name
    }

    /// Whether the file at `path` is Markdown, to show as it reads.
    static func isMarkdown(_ path: String) -> Bool {
        ["md", "markdown"].contains((path as NSString).pathExtension.lowercased())
    }

    package var body: some ReducerOf<Self> {
        Reduce { state, action in
            switch action {
            case .task:
                guard state.root == nil else { return .none }
                // Asked again after a read that failed, it shows it's reading.
                state.failure = nil
                return read("", at: state.place)

            case .refreshTapped:
                // Reads again every folder that is open, and the file shown.
                state.failure = nil
                state.unread = nil
                let open = [""] + state.expanded.sorted()
                // A closed folder's listing is dropped, so opening it
                // again reads it as it is by then.
                let shown = Set(open)
                state.folders = state.folders.filter { shown.contains($0.key) }
                state.cut.formIntersection(shown)
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
                state.lines = []
                state.unread = nil
                return readFile(path, at: state.place)

            case .fileRead(let contents, let lines):
                guard contents.path == state.selected else { return .none }
                state.failure = nil
                state.unread = nil
                state.contents = contents
                state.lines = lines
                return .none

            case .failed(let directory, let message):
                state.failure = message
                // Opened and never read, it closes, so opening it reads it again.
                if state.folders[directory] == nil { state.expanded.remove(directory) }
                return .none

            case .fileFailed(let path, let message):
                guard path == state.selected else { return .none }
                // With an earlier read still showing, the failure goes beside it.
                if state.contents?.path == path {
                    state.failure = message
                } else {
                    state.unread = message
                }
                return .none
            }
        }
    }

    private func read(_ directory: String, at place: HostPlace) -> Effect<Action> {
        .run { send in
            await send(.folderRead(try await daemon.listFolder(place, directory)))
        } catch: { error, send in
            await send(.failed(directory, error.localizedDescription))
        }
    }

    private func readFile(_ path: String, at place: HostPlace) -> Effect<Action> {
        .run { send in
            let contents = try await daemon.readFileAt(place, path)
            // Parsed here, off the main thread, so the view finds its blocks made.
            if Self.isMarkdown(path), let text = contents.text { _ = MarkdownBlock.cached(text) }
            // Split here too: a large file's lines aren't worked out each
            // time the view is drawn.
            await send(.fileRead(contents, lines: contents.text?.components(separatedBy: "\n") ?? []))
        } catch: { error, send in
            await send(.fileFailed(path, error.localizedDescription))
        }
    }
}
