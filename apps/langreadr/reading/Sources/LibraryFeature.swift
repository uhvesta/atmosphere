import ComposableArchitecture
import Foundation
import InferenceKit
import LangReadrCore
import LangReadrServices

/// What's imported to read: the person's books, the one open, and
/// imports. Every change to a book is kept on the shelf.
@Reducer
package struct LibraryFeature {
    @ObservableState
    package struct State: Equatable {
        package var books: IdentifiedArrayOf<Book> = []
        package var isImporting = false
        @Presents package var importing: ImportFeature.State?
        /// The book being read.
        @Presents package var reader: ReaderFeature.State?
        @Presents package var alert: AlertState<Never>?
        @Presents package var settings: SettingsFeature.State?

        package init(books: IdentifiedArrayOf<Book> = []) { self.books = books }
    }

    package enum Action {
        case task
        case importTapped(ImportKind)
        case photoPicked(Data)
        case fileChosen(URL)
        /// The person couldn't pick what they meant to import.
        case pickFailed(String)
        case drafted(Result<Draft, ImportFailure>)
        case bookTapped(Book.ID)
        case deleteTapped(Book.ID)
        case saveFailed(ShelfError)
        case importing(PresentationAction<ImportFeature.Action>)
        case reader(PresentationAction<ReaderFeature.Action>)
        case alert(PresentationAction<Never>)
        case settingsTapped
        case settings(PresentationAction<SettingsFeature.Action>)
    }

    @Dependency(\.shelf) private var shelf
    @Dependency(\.tutor) private var tutor
    @Dependency(\.readerServices) private var services
    @Dependency(\.date.now) private var now

    package init() {}

    package var body: some ReducerOf<Self> {
        Reduce { state, action in
            switch action {
            case .task:
                let read = shelf.books()
                state.books = IdentifiedArray(uniqueElements: read.books)
                if let unreadable = read.unreadable.first { state.alert = Self.alert(unreadable.description) }
                return .none
            case .importTapped(let kind):
                state.importing = ImportFeature.State(kind: kind)
                return .none
            case .importing(.presented(.delegate(.submit(let kind, let text)))):
                return draft { [services] () async throws(ImportFailure) -> Draft in
                    switch kind {
                    case .web:
                        let url: URL
                        let html: String
                        do throws(PageError) {
                            url = try ReaderServices.link(text)
                            html = try await services.fetchPage(url)
                        } catch {
                            throw .page(error)
                        }
                        return try ImportFailure.reading { () throws(ImportError) in
                            try Importer.html(html, source: .web(url))
                        }
                    case .text, .photo, .file:
                        return try ImportFailure.reading { () throws(ImportError) in try Importer.text(text) }
                    }
                }(&state)
            case .photoPicked(let data):
                return draft { [services] () async throws(ImportFailure) -> Draft in
                    let lines: [String]
                    do throws(RecognitionError) {
                        lines = try await services.recognizeText(data)
                    } catch {
                        throw .photo(error)
                    }
                    return try ImportFailure.reading { () throws(ImportError) in
                        try Importer.text(lines.joined(separator: "\n"), source: .photo)
                    }
                }(&state)
            case .fileChosen(let url):
                return draft { () async throws(ImportFailure) -> Draft in
                    let scoped = url.startAccessingSecurityScopedResource()
                    defer { if scoped { url.stopAccessingSecurityScopedResource() } }
                    let data: Data
                    do {
                        data = try Data(contentsOf: url)
                    } catch {
                        throw .unreadableFile(name: url.lastPathComponent, reason: error.localizedDescription)
                    }
                    return try ImportFailure.reading { () throws(ImportError) in
                        try Importer.file(data, name: url.lastPathComponent)
                    }
                }(&state)
            case .pickFailed(let reason):
                state.alert = Self.alert(reason)
                return .none
            case .drafted(.success(let draft)):
                state.isImporting = false
                let language = services.detectLanguage(draft.sample()) ?? tutor.reader
                let book = draft.book(in: language, at: now)
                state.books.insert(book, at: 0)
                state.importing = nil
                state.reader = ReaderFeature.State(book: book)
                return save(book)
            case .drafted(.failure(let failure)):
                state.isImporting = false
                state.importing?.isImporting = false
                state.alert = Self.alert(failure.description)
                return .none
            case .bookTapped(let id):
                guard let book = state.books[id: id], state.reader?.book.id != id else { return .none }
                state.reader = ReaderFeature.State(book: book)
                return .none
            case .deleteTapped(let id):
                guard let book = state.books.remove(id: id) else { return .none }
                if state.reader?.book.id == id { state.reader = nil }
                return .run { send in
                    do throws(ShelfError) {
                        try shelf.delete(book)
                    } catch {
                        await send(.saveFailed(error))
                    }
                }
            case .saveFailed(let error):
                state.alert = Self.alert(error.description)
                return .none
            case .settingsTapped:
                state.settings = SettingsFeature.State()
                return .none
            case .importing, .reader, .alert, .settings:
                return .none
            }
        }
        .ifLet(\.$importing, action: \.importing) { ImportFeature() }
        .ifLet(\.$reader, action: \.reader) { ReaderFeature() }
        .ifLet(\.$settings, action: \.settings) { SettingsFeature() }
        .onChange(of: \.reader?.book) { _, book in
            Reduce { state, _ in
                guard let book, state.books[id: book.id] != book else { return .none }
                state.books[id: book.id] = book
                return save(book)
            }
        }
        .ifLet(\.$alert, action: \.alert)
    }

    /// Reads a draft with `read`, sending what it came to.
    private func draft(_ read: @escaping @Sendable () async throws(ImportFailure) -> Draft) -> (inout State) -> Effect<
        Action
    > {
        { state in
            state.isImporting = true
            state.importing?.isImporting = true
            return .run { send in
                do throws(ImportFailure) {
                    await send(.drafted(.success(try await read())))
                } catch {
                    await send(.drafted(.failure(error)))
                }
            }
        }
    }

    private func save(_ book: Book) -> Effect<Action> {
        .run { send in
            do throws(ShelfError) {
                try shelf.save(book)
            } catch {
                await send(.saveFailed(error))
            }
        }
    }

    private static func alert(_ message: String) -> AlertState<Never> {
        AlertState {
            TextState("Something went wrong")
        } message: {
            TextState(message)
        }
    }
}

/// Why an import didn't make a book.
package enum ImportFailure: Error, Equatable, CustomStringConvertible {
    case importer(ImportError)
    case page(PageError)
    case photo(RecognitionError)
    case unreadableFile(name: String, reason: String)

    package var description: String {
        switch self {
        case .importer(let error): error.description
        case .page(let error): error.description
        case .photo(let error): error.description
        case .unreadableFile(let name, let reason):
            "\(name) couldn't be opened: \(reason). Check it's downloaded to the device, then try again."
        }
    }

    static func reading(_ read: () throws(ImportError) -> Draft) throws(ImportFailure) -> Draft {
        do throws(ImportError) {
            return try read()
        } catch {
            throw .importer(error)
        }
    }
}

/// The ways in.
package enum ImportKind: String, Identifiable, Hashable, Sendable, CaseIterable {
    case text, photo, web, file

    package var id: String { rawValue }
}

/// Pasting text, or giving a web page's link.
@Reducer
package struct ImportFeature {
    @ObservableState
    package struct State: Equatable {
        package var kind: ImportKind
        package var text = ""
        package var isImporting = false

        package init(kind: ImportKind, text: String = "") {
            self.kind = kind
            self.text = text
        }

        package var canImport: Bool {
            !isImporting && !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        }
    }

    package enum Action: BindableAction, Equatable {
        case binding(BindingAction<State>)
        case importTapped
        case cancelTapped
        case delegate(Delegate)

        @CasePathable
        package enum Delegate: Equatable {
            case submit(ImportKind, String)
        }
    }

    @Dependency(\.dismiss) private var dismiss

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .importTapped:
                guard state.canImport else { return .none }
                return .send(.delegate(.submit(state.kind, state.text)))
            case .cancelTapped:
                return .run { _ in await dismiss() }
            case .binding, .delegate:
                return .none
            }
        }
    }
}
