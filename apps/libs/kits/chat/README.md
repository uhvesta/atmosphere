# ChatKit

Conversations with an assistant. ChatKit is generic over the app's messages
and over what the app keeps with each conversation. It covers branches,
checkpoints and forks; summaries that compact a long conversation; the
newest messages that fit a model; and search by words and times. It's plain
Swift with Foundation, so it builds with the hermetic toolchains, Linux
included.

## Use it

```starlark
deps = ["//apps/libs/kits/chat"]
```

```swift
import ChatKit

struct Message: ChatEntry {
    var id: UUID, role: ChatRole, text: String, at: Date
    var attachments: [Attachment] = []
}

struct Kept: ConversationContext {
    var draft = ""
    init() {}
}

var chat = Conversation<Message, Kept>(id: UUID(), createdAt: now)
chat.append(question)
chat.append(answer)
chat.draft = "and then"                         // reaches into the context

chat.go(before: question.id)                    // edit an earlier message…
chat.append(edited)                             // …which starts a branch
let alternatives = chat.branches(at: edited.id)

let prompt = (chat.summary.map { [$0.summary.text] } ?? [])
    + chat.unsummarized.newest(fitting: budget).map(\.text)

let query = ConversationQuery("milk last week", now: now, calendar: .current)
let match = chat.match(query)
```

## What it gives you

- `Conversation` stores messages as a tree. `messages` is the path that's
  shown, and `hidden` holds everything else. `append`, `go(to:)` and
  `go(before:)` move along the tree and keep every branch.
  `checkpoints(changes:)` lists the points you can return to, and
  `fork(at:as:now:name:)` copies one branch into a new conversation.
- `ChatSummary` is kept under the last message it covers, so every branch
  through that message shares it. `compactable` lists what the next summary
  would cover. The last two messages always stay word for word.
- `ConversationQuery` reads words and times such as "yesterday" or "in
  March". `match(_:related:)` finds a conversation by its name, by a message
  on any branch, or by related words.
- `Conversation` decodes fields saved before they existed as their
  defaults. `Context` is encoded alongside the conversation's own fields,
  so a new context field should also read as empty when it's missing.

Test it with `bazel test //apps/libs/kits/chat:chat_test`.
