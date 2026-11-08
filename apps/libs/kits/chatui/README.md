# ChatUI

SwiftUI views for an app's assistant. Each view is built from plain data and
closures and has no store, so it works with TCA, with an observable model,
or in a preview. The views cover:

- messages, with their thinking, branches and summaries, and the
  assistant's replies shown as Markdown: headings, lists, quotes, code
  with Copy, rules, tables and inline styles, read again only from the
  last block as a reply is written
- the message field, the find bar and notices
- checkpoints
- rows and search snippets for the conversation list
- skills to pick and turn on
- a proposal's card and its editor
- rows for the change log
- models to download
- a compact chat to put inside another screen

## Use it

```starlark
deps = ["//apps/libs/kits/chatui"]
```

Build with `--config=xcode`, since SwiftUI needs Xcode.

```swift
import ChatUI

MessageView(
    role: message.role, text: message.text, thinking: message.thinking,
    showsThinking: expanded.contains(message.id),
    toggleThinking: { expanded.formSymmetricDifference([message.id]) },
    select: { selecting = $0 })

HStack {
    MessageField(text: $input, prompt: Text("Ask anything"), submit: send)
    SendButton(canSend: !input.isEmpty, canStop: isReplying, send: send, stop: stop)
}

// A chat beside what the person is reading.
CompactChat(
    messages: chat.messages, reply: reply, input: $input,
    canSend: !input.isEmpty, canStop: reply != nil, send: send, stop: stop)
```

## What it gives you

- **Messages:** `MessageView`, `ThinkingDisclosure`, `ReplyInProgress`,
  `BranchSwitcher`, `AttachmentChip` and `Suggestions`.
- **Composing:** `MessageField`, `SendButton`, `FindBar`, `ChatNotice` and
  `SummaryMarker`.
- **History:** `CheckpointList`, `ConversationRow`, and
  `Snippet.around(_:in:)` for the words a search found.
- **Skills and commands:** `SkillToken`, the "/name" a message starts with
  (`ChatComposer.token(_:remove:)` puts it in the field), `SlashSuggestions`
  for what's typed after a /, and `SkillToggle`.
- **Changes:** `ProposalCard`, `ProposalEditor`, `ActivityEntryRow` and
  `FieldLine`. Each takes `ChangedField`s, so the app chooses how its
  fields are named.
- **Models:** `ModelRow` shows a download's progress, the context window,
  and buttons to start, pause or remove. `ModelTuningView` shows where a
  downloaded model runs and why, what was measured of it on the GPU and
  the CPU side by side (speed, memory, and per token the CPU's time, the
  battery the CPU drew and the GPU's busy time), a button to measure it,
  and a picker to choose.
- **Odds and ends:** `TextMenu` and `SelectTextSheet` for copying and
  selecting text, `sheetLayout`, `DismissButton`, and `Binding.or` and
  `.orEmpty` for optional bindings.

Model text has the accessibility identifier `ModelText.identifier`, so
FlowKit's flows leave it out of their references. The views' words are
looked up in the app's String Catalog, and the app's `update_strings`
collects them from this kit too.

The library compiles in Swift 5 mode, like the apps' views. SwiftUI's
bindings take Sendable closures, and views' own closures aren't Sendable.
The tests cover what the views work out from their data, such as changes
grouped by record and search snippets. Run them with
`bazel test --config=xcode //apps/libs/kits/chatui:chatui_test`. The views
themselves are checked by the apps' flow snapshots.
