import AssistantKit
import SwiftUI

/// The skill picked for the next message: tap it to pick another, or
/// remove it.
public struct SkillPill: View {
    let title: String
    let change: () -> Void
    let remove: () -> Void

    public init(title: String, change: @escaping () -> Void, remove: @escaping () -> Void) {
        self.title = title
        self.change = change
        self.remove = remove
    }

    public var body: some View {
        HStack(spacing: 4) {
            Button(action: change) {
                Label(title, systemImage: "sparkles")
                    .font(.callout.weight(.medium))
            }
            .buttonStyle(.plain)
            .accessibilityHint(Text("Picks another skill"))
            Button(action: remove) {
                Label("Remove \(title)", systemImage: "xmark.circle.fill")
                    .labelStyle(.iconOnly)
                    .foregroundStyle(.secondary)
            }
            .buttonStyle(.plain)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background(.tint.opacity(0.2), in: .capsule)
    }
}

/// The button to find a skill for the next message, or the pill of the
/// one picked.
public struct SkillButton: View {
    let picked: String?
    let find: () -> Void
    let remove: () -> Void

    public init(picked: String?, find: @escaping () -> Void, remove: @escaping () -> Void) {
        self.picked = picked
        self.find = find
        self.remove = remove
    }

    public var body: some View {
        if let picked {
            SkillPill(title: picked, change: find, remove: remove)
        } else {
            Button(action: find) {
                Label("Use a Skill", systemImage: "sparkles")
                    .labelStyle(.iconOnly)
                    .font(.title2)
            }
        }
    }
}

/// Skills to pick from, by title and what each does, saying why the list
/// is empty when it is.
public struct SkillChoices<Tool: SkillTool>: View {
    let skills: [Skill<Tool>]
    let query: String
    let pick: (Skill<Tool>.ID) -> Void

    public init(_ skills: [Skill<Tool>], query: String, pick: @escaping (Skill<Tool>.ID) -> Void) {
        self.skills = skills
        self.query = query
        self.pick = pick
    }

    public var body: some View {
        List(skills) { skill in
            Button {
                pick(skill.id)
            } label: {
                VStack(alignment: .leading, spacing: 2) {
                    Text(skill.title)
                    Text(skill.summary)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .contentShape(.rect)
            }
            .buttonStyle(.plain)
        }
        .overlay {
            if skills.isEmpty {
                if query.isEmpty {
                    ContentUnavailableView(
                        "No skills on", systemImage: "sparkles",
                        description: Text("Turn skills on in Manage Skills."))
                } else {
                    ContentUnavailableView.search(text: query)
                }
            }
        }
    }
}

/// Finds a skill for the next message by its name or what it does: on the
/// Mac a popover's content, on iOS a sheet with the search at the top.
public struct SkillFinder<Tool: SkillTool>: View {
    let skills: [Skill<Tool>]
    @Binding var query: String
    let pick: (Skill<Tool>.ID) -> Void
    let manage: () -> Void
    let cancel: () -> Void

    public init(
        _ skills: [Skill<Tool>], query: Binding<String>, pick: @escaping (Skill<Tool>.ID) -> Void,
        manage: @escaping () -> Void, cancel: @escaping () -> Void
    ) {
        self.skills = skills
        _query = query
        self.pick = pick
        self.manage = manage
        self.cancel = cancel
    }

    public var body: some View {
        #if os(macOS)
            VStack(spacing: 0) {
                TextField("Find a skill", text: $query)
                    .textFieldStyle(.roundedBorder)
                    .padding(10)
                SkillChoices(skills, query: query, pick: pick)
                Divider()
                HStack {
                    Button("Manage Skills…", action: manage)
                    Spacer()
                    Button("Cancel", action: cancel)
                        .keyboardShortcut(.cancelAction)
                }
                .padding(10)
            }
            .frame(width: 340, height: 420)
        #else
            NavigationStack {
                // The search field and Manage Skills sit above the list:
                // at the bottom they'd float over its last rows.
                SkillChoices(skills, query: query, pick: pick)
                    .searchable(
                        text: $query, placement: .navigationBarDrawer(displayMode: .always),
                        prompt: Text("Find a skill")
                    )
                    .navigationTitle("Skills")
                    .navigationBarTitleDisplayMode(.inline)
                    .toolbar {
                        ToolbarItem(placement: .cancellationAction) {
                            Button("Cancel", action: cancel)
                        }
                        ToolbarItem(placement: .primaryAction) {
                            Button("Manage Skills…", action: manage)
                        }
                    }
            }
            .presentationDetents([.medium, .large])
        #endif
    }
}

/// A skill to turn on or off, with what it does and whether it's picked
/// automatically.
public struct SkillToggle<Tool: SkillTool>: View {
    let skill: Skill<Tool>
    let toggle: (Bool) -> Void

    public init(_ skill: Skill<Tool>, toggle: @escaping (Bool) -> Void) {
        self.skill = skill
        self.toggle = toggle
    }

    public var body: some View {
        Toggle(isOn: Binding(get: { skill.isOn }, set: toggle)) {
            VStack(alignment: .leading, spacing: 2) {
                Text(skill.title)
                Text(skill.autoPicks ? "\(skill.summary) Automatic." : skill.summary)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }
}

/// The skills matching what's typed after a / in the message field, in a
/// card above it that scrolls. Picking one becomes the message's skill.
public struct SkillSuggestions<Tool: SkillTool>: View {
    let skills: [Skill<Tool>]
    let pick: (Skill<Tool>.ID) -> Void

    public init(_ skills: [Skill<Tool>], pick: @escaping (Skill<Tool>.ID) -> Void) {
        self.skills = skills
        self.pick = pick
    }

    public var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(skills) { skill in
                    Button {
                        pick(skill.id)
                    } label: {
                        VStack(alignment: .leading, spacing: 2) {
                            // What's typed to pick it: "/query".
                            Text(verbatim: "/\(skill.id)")
                                .font(.body.monospaced())
                            if !skill.summary.isEmpty {
                                Text(skill.summary)
                                    .font(.caption)
                                    .foregroundStyle(Color.secondary)
                                    .lineLimit(2)
                            }
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, 14)
                        .padding(.vertical, 10)
                        .contentShape(.rect)
                    }
                    .buttonStyle(.plain)
                    if skill.id != skills.last?.id {
                        Divider().padding(.leading, 14)
                    }
                }
            }
        }
        .scrollBounceBehavior(.basedOnSize)
        .frame(maxHeight: 240)
        .fixedSize(horizontal: false, vertical: true)
        // The shadow goes on the card alone: on the scroll view, it would be
        // drawn under each row and wash out their gray text.
        .background {
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .fill(.background)
                .shadow(color: .black.opacity(0.08), radius: 10, y: 2)
        }
        .overlay {
            RoundedRectangle(cornerRadius: 18, style: .continuous).strokeBorder(.quaternary)
        }
    }
}
