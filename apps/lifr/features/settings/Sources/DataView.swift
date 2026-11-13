import ComposableArchitecture
import LifrBackupClient
import LifrCore
import SwiftUI
import UniformTypeIdentifiers

/// Settings › Data.
package struct DataView: View {
    @Bindable package var store: StoreOf<DataFeature>
    @State private var picksFile = false

    package init(store: StoreOf<DataFeature>) {
        self.store = store
    }

    package var body: some View {
        Form {
            Section {
                Button("Export Everything…") { store.send(.exportTapped) }
                Button("Export as Markdown…") { store.send(.markdownTapped) }
            } header: {
                Text("Export")
            } footer: {
                Text(
                    """
                    Export Everything makes one file with every task, comment and change, which Lifr can import \
                    again. Markdown makes a file per Avenue, for reading anywhere.
                    """)
            }
            Section {
                Button("Import…") { picksFile = true }
                if let imported = store.imported {
                    LabeledContent("Records added", value: imported.count.formatted())
                    Button("Undo Import") { store.send(.undoTapped) }
                }
            } header: {
                Text("Import")
            } footer: {
                Text(
                    """
                    A file Lifr exported, or a CSV file with a column of titles. You see what it adds before \
                    anything changes, and only what isn't here already is added.
                    """)
            }
            Section {
                if store.backups.isEmpty {
                    Text("No backups yet. The first is made the next time Lifr opens.")
                        .foregroundStyle(.secondary)
                }
                ForEach(store.backups) { backup in
                    BackupRow(backup: backup) { store.send(.backupTapped(backup.day)) }
                }
            } header: {
                Text("Backups")
            } footer: {
                Text(
                    """
                    Lifr backs up this device's records once a day and keeps the last seven days. Importing one \
                    adds back only what's missing since.
                    """)
            }
        }
        .formStyle(.grouped)
        .navigationTitle("Data")
        .disabled(store.isWorking)
        .task { await store.send(.appeared).finish() }
        .fileImporter(isPresented: $picksFile, allowedContentTypes: [.json, .commaSeparatedText, .plainText]) {
            store.send(.filePicked($0))
        }
        .fileExporter(
            isPresented: Binding(
                get: { store.export != nil }, set: { if !$0 { store.send(.exportFinished(.success(()))) } }),
            document: store.export.map(ExportDocument.init),
            contentType: store.export.map(ExportDocument.contentType) ?? .json,
            defaultFilename: store.export.map(ExportDocument.defaultName)
        ) { result in
            store.send(.exportFinished(result.map { _ in () }))
        }
        .sheet(
            isPresented: Binding(get: { store.preview != nil }, set: { if !$0 { store.send(.importCancelled) } })
        ) {
            if let preview = store.preview {
                ImportPreviewView(
                    preview: preview,
                    confirm: { store.send(.importConfirmed) },
                    cancel: { store.send(.importCancelled) })
            }
        }
        .sheet(isPresented: Binding(get: { store.csv != nil }, set: { if !$0 { store.send(.csvCancelled) } })) {
            if store.csv != nil {
                CSVImportView(store: store)
            }
        }
        .alert($store.scope(state: \.alert, action: \.alert))
    }
}

private struct BackupRow: View {
    /// A backup's day, which is a calendar day, not a moment.
    private static let dayStyle = {
        var style = Date.FormatStyle.dateTime.weekday(.wide).day().month(.wide)
        style.timeZone = .gmt
        return style
    }()

    var backup: Backup
    var restore: () -> Void

    var body: some View {
        HStack {
            VStack(alignment: .leading) {
                Text(backup.day.noonUTC, format: Self.dayStyle)
                Text(Int64(backup.size), format: .byteCount(style: .file))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer()
            Button("Import…", action: restore)
        }
    }
}

/// What a Lifr file adds, before it's added.
private struct ImportPreviewView: View {
    var preview: DataFeature.Preview
    var confirm: () -> Void
    var cancel: () -> Void

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    LabeledContent("Avenues", value: preview.plan.count(of: .avenue).formatted())
                    LabeledContent("Projects", value: preview.plan.count(of: .project).formatted())
                    LabeledContent("Tasks", value: preview.plan.count(of: .task).formatted())
                    LabeledContent("Comments", value: preview.plan.count(of: .comment).formatted())
                    LabeledContent("Labels", value: preview.plan.count(of: .label).formatted())
                    LabeledContent("Reminders", value: preview.plan.count(of: .reminder).formatted())
                    LabeledContent("Saved filters", value: preview.plan.count(of: .savedFilter).formatted())
                } header: {
                    Text("To add")
                } footer: {
                    if preview.plan.isEmpty {
                        Text("Everything in this file is here already, so importing it changes nothing.")
                    } else {
                        Text("Undo Import in Settings › Data takes them out again.")
                    }
                }
                Section {
                    LabeledContent("Already here", value: preview.plan.alreadyHere.formatted())
                } footer: {
                    Text("Records that are here already are left as they are.")
                }
            }
            .formStyle(.grouped)
            .navigationTitle(preview.source)
            #if os(iOS)
                .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel", action: cancel)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Import", action: confirm)
                        .disabled(preview.plan.isEmpty)
                }
            }
        }
    }
}

/// Which of a CSV file's columns hold what, with its first rows.
private struct CSVImportView: View {
    @Bindable var store: StoreOf<DataFeature>

    var body: some View {
        if let csv = store.csv {
            NavigationStack {
                Form {
                    Section {
                        titleColumn
                        column("Notes", \.details)
                        column("Due date", \.due)
                        column("Avenue", \.avenue)
                        column("Status", \.status)
                    } header: {
                        Text("Columns")
                    } footer: {
                        Text("Due dates are read as YYYY-MM-DD, and Avenues by name or key.")
                    }
                    Section {
                        Picker("Avenue", selection: mapping(\.avenueID)) {
                            ForEach(csv.avenues) { avenue in
                                Text(avenue.name).tag(avenue.id)
                            }
                        }
                    } footer: {
                        Text("For rows that don't name an Avenue.")
                    }
                    Section {
                        ForEach(csv.table.rows.prefix(3), id: \.line) { row in
                            Text(row.fields.indices.contains(csv.mapping.title) ? row.fields[csv.mapping.title] : "")
                                .lineLimit(1)
                        }
                        LabeledContent("Rows", value: csv.table.rows.count.formatted())
                    } header: {
                        Text("First rows")
                    }
                }
                .formStyle(.grouped)
                .navigationTitle(csv.source)
                #if os(iOS)
                    .navigationBarTitleDisplayMode(.inline)
                #endif
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        Button("Cancel") { store.send(.csvCancelled) }
                    }
                    ToolbarItem(placement: .confirmationAction) {
                        Button("Import") { store.send(.csvConfirmed) }
                            .disabled(csv.table.rows.isEmpty)
                    }
                }
            }
        }
    }

    private func mapping<Value>(_ field: WritableKeyPath<CSVMapping, Value>) -> Binding<Value> {
        Binding(
            get: { store.csv?.mapping[keyPath: field] ?? CSVMapping(title: 0)[keyPath: field] },
            set: { value in
                guard var mapping = store.csv?.mapping else { return }
                mapping[keyPath: field] = value
                store.send(.csvMapped(mapping))
            })
    }

    private var titleColumn: some View {
        Picker("Title", selection: mapping(\.title)) {
            ForEach(Array((store.csv?.table.columns ?? []).enumerated()), id: \.offset) { index, name in
                Text(verbatim: name.isEmpty ? "\(index + 1)" : name).tag(index)
            }
        }
    }

    private func column(_ title: LocalizedStringKey, _ field: WritableKeyPath<CSVMapping, Int?>) -> some View {
        Picker(title, selection: mapping(field)) {
            Text("None").tag(Int?.none)
            ForEach(Array((store.csv?.table.columns ?? []).enumerated()), id: \.offset) { index, name in
                Text(verbatim: name.isEmpty ? "\(index + 1)" : name).tag(Int?.some(index))
            }
        }
    }
}

/// An export, as the system's file exporter saves it.
private struct ExportDocument: FileDocument {
    static let readableContentTypes: [UTType] = []
    static let writableContentTypes: [UTType] = [.json, .folder]

    var file: DataFeature.ExportFile

    init(_ file: DataFeature.ExportFile) {
        self.file = file
    }

    init(configuration: ReadConfiguration) throws {
        throw CocoaError(.featureUnsupported)
    }

    static func contentType(_ file: DataFeature.ExportFile) -> UTType {
        switch file {
        case .everything: .json
        case .markdown: .folder
        }
    }

    static func defaultName(_ file: DataFeature.ExportFile) -> String {
        switch file {
        case .everything(let name, _): name
        case .markdown: String(localized: "Lifr Avenues")
        }
    }

    func fileWrapper(configuration: WriteConfiguration) throws -> FileWrapper {
        switch file {
        case .everything(_, let data):
            return FileWrapper(regularFileWithContents: data)
        case .markdown(let files):
            return FileWrapper(directoryWithFileWrappers: files.mapValues { FileWrapper(regularFileWithContents: $0) })
        }
    }
}
