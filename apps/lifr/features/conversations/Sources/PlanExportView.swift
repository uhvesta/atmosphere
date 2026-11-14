import ComposableArchitecture
import LifrAPI
import SwiftUI

/// Makes a prompt to plan with another assistant: what it's for, which
/// tasks it lists, and the prompt itself to copy or share.
struct PlanExportView: View {
    @Bindable var store: StoreOf<PlanExportFeature>

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("Goal", text: $store.goal, prompt: Text("Move house by March"), axis: .vertical)
                        .lineLimit(1...4)
                } footer: {
                    Text("What the other assistant plans. Leave it empty to say it there instead.")
                }
                Section {
                    ChipFlow(spacing: 8) {
                        chip(Text("Names Only"), .names)
                        chip(Text("Whole Workspace"), .everything)
                        ForEach(store.avenues) { avenue in
                            chip(Text(avenue.name), .avenue(avenue.id))
                        }
                        ForEach(store.projects, id: \.project.id) { pair in
                            chip(Text("\(pair.avenue.name) › \(pair.project.name)"), .project(pair.project.id))
                        }
                    }
                    .accessibilityElement(children: .contain)
                    .accessibilityLabel("Tasks Listed")
                    Toggle("Open Tasks Only", isOn: $store.openOnly)
                        .disabled(!store.listsTasks)
                    Toggle("Descriptions", isOn: $store.descriptions)
                        .disabled(!store.listsTasks)
                } footer: {
                    if store.listsTasks {
                        Text(
                            """
                            Listing tasks keeps the assistant from planning them again, but copies their titles \
                            out of Lifr, and their descriptions when that's on.
                            """)
                    } else {
                        Text("The prompt names your Avenues, Projects and labels only.")
                    }
                }
                if let preset = store.preset, preset.isUser {
                    Section {
                        Button {
                            store.send(.rememberTapped)
                        } label: {
                            Label(
                                store.remembered ? "Kept in \(preset.title)" : "Keep Goal and Scope in \(preset.title)",
                                systemImage: store.remembered ? "checkmark" : "bookmark")
                        }
                        .disabled(store.remembered)
                    } footer: {
                        Text("Planning with this skill next time starts from this goal and these tasks.")
                    }
                }
                Section {
                    Text(store.prompt)
                        .font(.footnote.monospaced())
                        .textSelection(.enabled)
                        .lineLimit(12)
                    Button {
                        store.send(.copyTapped)
                    } label: {
                        Label(store.copied ? "Copied" : "Copy Prompt", systemImage: "doc.on.doc")
                    }
                    ShareLink(item: store.prompt) {
                        Label("Share Prompt…", systemImage: "square.and.arrow.up")
                    }
                } header: {
                    Text("Prompt")
                } footer: {
                    if let preset = store.preset {
                        Text(
                            """
                            It carries the instructions of your skill \(preset.title). Paste it into any chat \
                            assistant, then copy its whole reply and choose Paste Plan Reply here to review it.
                            """)
                    } else {
                        Text(
                            """
                            Paste it into any chat assistant, then copy its whole reply and choose Paste Plan Reply \
                            here to review it.
                            """)
                    }
                }
            }
            .formStyle(.grouped)
            .navigationTitle("Plan with Another Assistant")
            #if os(iOS)
                // A large title is too long for the iPhone's width.
                .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { store.send(.doneTapped) }
                }
            }
            .alert($store.scope(state: \.alert, action: \.alert))
        }
    }

    /// One choice of which tasks the prompt lists; the chosen one is filled.
    private func chip(_ title: Text, _ scope: PlanPrompt.Scope) -> some View {
        let chosen = store.listed == scope
        return Button {
            store.send(.binding(.set(\.listed, scope)))
        } label: {
            title
                .font(.subheadline)
                .lineLimit(1)
                .padding(.horizontal, 10)
                .padding(.vertical, 6)
                .foregroundStyle(chosen ? AnyShapeStyle(.white) : AnyShapeStyle(.tint))
                .background(chosen ? AnyShapeStyle(.tint) : AnyShapeStyle(.tint.opacity(0.12)), in: Capsule())
        }
        .buttonStyle(.borderless)
        .accessibilityAddTraits(chosen ? .isSelected : [])
    }
}

/// Lays its views out in rows, starting a new row when one is full.
private struct ChipFlow: Layout {
    var spacing: CGFloat

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let rows = rows(width: proposal.width ?? .infinity, subviews)
        let width = rows.map(\.width).max() ?? 0
        let height = rows.map(\.height).reduce(0, +) + spacing * CGFloat(max(rows.count - 1, 0))
        return CGSize(width: width, height: height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var top = bounds.minY
        for row in rows(width: bounds.width, subviews) {
            var leading = bounds.minX
            for index in row.indices {
                let size = subviews[index].sizeThatFits(ProposedViewSize(width: bounds.width, height: nil))
                subviews[index].place(at: CGPoint(x: leading, y: top), proposal: ProposedViewSize(size))
                leading += size.width + spacing
            }
            top += row.height + spacing
        }
    }

    private struct Row {
        var indices: [Int] = []
        var width: CGFloat = 0
        var height: CGFloat = 0
    }

    private func rows(width: CGFloat, _ subviews: Subviews) -> [Row] {
        var rows: [Row] = []
        var row = Row()
        for index in subviews.indices {
            // A chip wider than a row is cut short to fit it.
            let size = subviews[index].sizeThatFits(ProposedViewSize(width: width, height: nil))
            let needed = row.indices.isEmpty ? size.width : row.width + spacing + size.width
            if needed > width, !row.indices.isEmpty {
                rows.append(row)
                row = Row()
            }
            row.width = row.indices.isEmpty ? size.width : row.width + spacing + size.width
            row.height = max(row.height, size.height)
            row.indices.append(index)
        }
        if !row.indices.isEmpty { rows.append(row) }
        return rows
    }
}
