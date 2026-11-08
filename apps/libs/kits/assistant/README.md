# AssistantKit

What an app's assistant can do. Skills are generic over the app's tools,
with search, and the built-in skills stay up to date. Proposals are generic
over the app's actions: the person applies or declines one before anything
is saved. It's plain Swift with Foundation, so it builds with the hermetic
toolchains, Linux included.

## Use it

```starlark
deps = ["//apps/libs/kits/assistant"]
```

```swift
import AssistantKit

enum NoteTool: String, SkillTool {
    case search, edit
    var writes: Bool { self == .edit }
    static var builtInSkills: [Skill<Self>] {
        [Skill(id: "tidy", summary: "Tidies a note", instructions: "…", tools: [.search, .edit],
              draftsPlans: false)]
    }
}

let skills = Skill<NoteTool>.merged(stored: saved)    // built-ins kept current
let found = Skill.search("tidy", in: Skill.pickable(skills))

var proposal = Proposal<NoteAction, JournalStep<NoteRecord>>(actions: actions, steps: steps)
proposal.decision = .applied
```

## What it gives you

- `Skill` holds instructions, the tools it may call, whether it drafts
  plans, whether it's on, and whether the assistant may pick it by itself
  (`autoPicks` and `useWhen`); `pickable(_:chat:app:)` lets one chat turn
  that picking off or on. The person's own skills have IDs that start
  with `user:`. `SkillDraft` is a skill the assistant wrote from the
  person's description, for an editor to fill in.
- `Skill.merged(stored:)` keeps whether the person turned each built-in
  skill on or off and let it pick itself. Everything else comes from the
  app as it is now, so new built-ins and new instructions take effect. The
  person's own skills follow, sorted by ID.
- `Skill.search` ranks titles that start with the query first, then titles
  that contain its letters in order, then summaries that contain every word.
- `Proposal` is what a writing tool would change: the actions, the steps
  they make, and the person's `ProposalDecision`. Nothing is saved while
  it's pending.

Test it with `bazel test //apps/libs/kits/assistant:assistant_test`.
