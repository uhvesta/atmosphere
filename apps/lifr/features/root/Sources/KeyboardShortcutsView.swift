import SwiftUI

/// Help › Keyboard Shortcuts: every shortcut Lifr has, by where it works.
struct KeyboardShortcutsView: View {
    @Environment(\.dismiss) private var dismiss

    private static let groups: [(title: LocalizedStringKey, items: [(keys: String, name: LocalizedStringKey)])] = [
        (
            "Anywhere",
            [
                ("⌘N", "New Task"), ("⇧⌘N", "New Sub task"), ("⌥⌘N", "New Project"),
                ("⌘F", "Find"), ("⌘K", "Go to Task"), ("⌘R", "Sync Now"),
                ("⌘Z", "Undo"), ("⇧⌘Z", "Redo"),
                ("⌘1", "Go to Today"), ("⌘2", "Go to Upcoming"), ("⌘3", "Go to Calendar"),
            ]
        ),
        (
            "The open task",
            [
                ("⌥⌘1 – ⌥⌘5", "To Do, In Progress, Procrastinated, Done, Won't Do"),
                ("⌃0 – ⌃4", "No priority, Low, Medium, High, Urgent"),
                ("⌘T", "Due Today"), ("⇧⌘T", "Due Tomorrow"),
                ("⇧⌘M", "Move to another Avenue or Project"),
            ]
        ),
        (
            "A list",
            [
                ("↑ ↓", "Move between tasks"), ("⇧↑ ⇧↓", "Pick several tasks"),
                ("Return", "Open the task"), ("Space", "Change its status"), ("Delete", "Move to the Trash"),
            ]
        ),
    ]

    var body: some View {
        NavigationStack {
            Form {
                ForEach(Array(Self.groups.enumerated()), id: \.offset) { _, group in
                    Section(group.title) {
                        ForEach(Array(group.items.enumerated()), id: \.offset) { _, item in
                            LabeledContent {
                                Text(item.keys).monospaced()
                            } label: {
                                Text(item.name)
                            }
                        }
                    }
                }
            }
            .formStyle(.grouped)
            .navigationTitle("Keyboard Shortcuts")
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
        }
        #if os(macOS)
            .frame(minWidth: 460, minHeight: 520)
        #endif
    }
}
