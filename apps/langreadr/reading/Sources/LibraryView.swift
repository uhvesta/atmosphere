import ComposableArchitecture
import LangReadrCore
import PhotosUI
import SwiftUI
import UniformTypeIdentifiers

/// LangReadr: the library, and the book open in it. A stack on the iPhone;
/// on the Mac, the library is a sidebar and the book fills the window.
package struct LangReadrView: View {
    @Bindable var store: StoreOf<LibraryFeature>

    package init(store: StoreOf<LibraryFeature>) { self.store = store }

    package var body: some View {
        #if os(macOS)
            NavigationSplitView {
                LibraryView(store: store)
                    .navigationSplitViewColumnWidth(min: 240, ideal: 280)
            } detail: {
                if let reader = store.scope(state: \.reader, action: \.reader.presented) {
                    NavigationStack { ReaderView(store: reader) }.id(reader.id)
                } else {
                    ContentUnavailableView(
                        "Nothing open", systemImage: "book",
                        description: Text("Import something to read, or pick a book."))
                }
            }
        #else
            NavigationStack {
                LibraryView(store: store)
                    .navigationDestination(item: $store.scope(state: \.reader, action: \.reader)) {
                        ReaderView(store: $0)
                    }
            }
        #endif
    }
}

/// The books, the newest first, and the ways to import another.
struct LibraryView: View {
    @Bindable var store: StoreOf<LibraryFeature>
    @State private var photo: PhotosPickerItem?
    @State private var choosingFile = false
    @State private var choosingPhoto = false

    var body: some View {
        List(selection: selection) {
            ForEach(store.books) { book in
                VStack(alignment: .leading, spacing: 2) {
                    Text(book.title).font(.headline).lineLimit(1)
                    Text("\(book.language.name()) · \(book.minutes) min")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                }
                .tag(book.id)
                .contentShape(Rectangle())
                #if os(iOS)
                    .onTapGesture { store.send(.bookTapped(book.id)) }
                    .accessibilityAddTraits(.isButton)
                    .accessibilityAction { store.send(.bookTapped(book.id)) }
                #endif
                .contextMenu {
                    Button("Delete", systemImage: "trash", role: .destructive) { store.send(.deleteTapped(book.id)) }
                }
            }
            .onDelete { offsets in
                for id in offsets.map({ store.books[$0].id }) { store.send(.deleteTapped(id)) }
            }
        }
        .overlay {
            if store.books.isEmpty {
                ContentUnavailableView(
                    "Nothing to read yet", systemImage: "books.vertical",
                    description: Text("Import text, a photo, a web page or a file in the language you're learning."))
            }
        }
        .navigationTitle("Library")
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button("Settings", systemImage: "gearshape") { store.send(.settingsTapped) }
            }
        }
        .sheet(item: $store.scope(state: \.settings, action: \.settings)) { SettingsView(store: $0) }
        .task { await store.send(.task).finish() }
        .safeAreaInset(edge: .bottom) { importBar }
        .sheet(item: $store.scope(state: \.importing, action: \.importing)) { ImportSheet(store: $0) }
        .photosPicker(isPresented: $choosingPhoto, selection: $photo, matching: .images)
        .onChange(of: photo) { _, item in
            guard let item else { return }
            photo = nil
            Task {
                if let data = try? await item.loadTransferable(type: Data.self) {
                    store.send(.photoPicked(data))
                } else {
                    store.send(.pickFailed("That photo couldn't be opened. Pick another, or take a screenshot of it."))
                }
            }
        }
        .fileImporter(
            isPresented: $choosingFile,
            allowedContentTypes: [.plainText, .html, .text, .init(filenameExtension: "md") ?? .text]
        ) { result in
            switch result {
            case .success(let url): store.send(.fileChosen(url))
            case .failure(let error):
                store.send(.pickFailed("The file couldn't be opened: \(error.localizedDescription). Pick another."))
            }
        }
        .alert($store.scope(state: \.alert, action: \.alert))
    }

    private var selection: Binding<UUID?> {
        Binding(
            get: { store.reader?.book.id },
            set: { id in if let id { store.send(.bookTapped(id)) } })
    }

    private var importBar: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text("Import").font(.caption).foregroundStyle(.secondary)
                if store.isImporting { ProgressView().controlSize(.small) }
            }
            HStack {
                Button("Text", systemImage: "doc.plaintext") { store.send(.importTapped(.text)) }
                Button("Photo", systemImage: "photo") { choosingPhoto = true }
                Button("Web", systemImage: "globe") { store.send(.importTapped(.web)) }
                Button("File", systemImage: "folder") { choosingFile = true }
            }
            .buttonStyle(.bordered)
            #if os(macOS)
                // The sidebar is too narrow for each icon and title.
                .labelStyle(.titleOnly)
            #endif
            .disabled(store.isImporting)
        }
        .padding()
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.bar)
    }
}

/// Pasting text, or giving a web page's link.
struct ImportSheet: View {
    @Bindable var store: StoreOf<ImportFeature>

    var body: some View {
        NavigationStack {
            Form {
                if store.kind == .web {
                    TextField("https://", text: $store.text)
                        .autocorrectionDisabled()
                        #if os(iOS)
                            .textInputAutocapitalization(.never)
                            .keyboardType(.URL)
                        #endif
                    Text("The page's headings and paragraphs are read; its menus and ads are left out.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                } else {
                    TextEditor(text: $store.text).frame(minHeight: 200)
                    Text("Start a line with # for a chapter.").font(.caption).foregroundStyle(.secondary)
                }
            }
            .navigationTitle(store.kind == .web ? "Web page" : "Text")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { store.send(.cancelTapped) } }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Import") { store.send(.importTapped) }.disabled(!store.canImport)
                }
            }
        }
        #if os(macOS)
            .frame(minWidth: 460, minHeight: 320)
        #endif
    }
}
