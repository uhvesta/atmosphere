import ComposableArchitecture
import CoreTransferable
import SwiftUI
import UniformTypeIdentifiers

package struct BugReportView: View {
    @Bindable package var store: StoreOf<BugReportFeature>

    package init(store: StoreOf<BugReportFeature>) {
        self.store = store
    }

    package var body: some View {
        Form {
            Section("What happened") {
                TextEditor(text: $store.report.happened)
                    .frame(minHeight: 88)
                    .accessibilityLabel("What happened")
            }
            Section("What did you expect") {
                TextEditor(text: $store.report.expected)
                    .frame(minHeight: 88)
                    .accessibilityLabel("What did you expect")
            }
            Section {
                Toggle("Include my text and screenshots", isOn: $store.report.includesPersonal)
            } footer: {
                if store.report.includesPersonal {
                    Text("The report shows your tasks, chats and screens as they were.")
                } else {
                    Text(
                        """
                        Each piece of text from your tasks and chats becomes a placeholder, and screenshots \
                        are left out. What you write above is kept.
                        """)
                }
            }
            if let recording = store.report.recording {
                Section("Recorded") {
                    LabeledContent("Steps", value: "\(recording.steps.count + recording.dropped)")
                    LabeledContent(
                        "Screenshots", value: store.report.includesPersonal ? "\(recording.screenshots.count)" : "0")
                }
            }
            Section {
                LabeledContent("Device", value: "\(store.report.device.model), \(store.report.device.system)")
                LabeledContent("Lifr", value: store.report.device.appVersion)
                LabeledContent("Assistant model", value: store.report.device.assistantModel)
            }
            if let error = store.error {
                Section { Text(error.description).foregroundStyle(.red) }
            }
            Section {
                Button(store.copied ? "Copied" : "Copy") { store.send(.copyTapped) }
                ShareLink(
                    item: BugReportFile(report: store.kept),
                    preview: SharePreview(Text("Lifr bug report"))
                ) {
                    Text("Share")
                }
            }
        }
        .formStyle(.grouped)
        .navigationTitle("Report a Bug")
        .toolbar {
            ToolbarItem(placement: .cancellationAction) {
                Button("Cancel") { store.send(.cancelTapped) }
            }
            ToolbarItem(placement: .confirmationAction) {
                Button("Save") { store.send(.saveTapped) }
                    .disabled(!store.canSave)
            }
        }
    }
}

/// A report as one JSON file, screenshots included, to share.
package struct BugReportFile: Transferable {
    package var report: BugReport

    package init(report: BugReport) {
        self.report = report
    }

    package static var transferRepresentation: some TransferRepresentation {
        DataRepresentation(exportedContentType: .json) { file in
            let encoder = JSONEncoder()
            encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
            encoder.dateEncodingStrategy = .iso8601
            return try encoder.encode(file.report)
        }
        .suggestedFileName("Lifr bug report.json")
    }
}
