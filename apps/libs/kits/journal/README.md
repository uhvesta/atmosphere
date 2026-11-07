# JournalKit

A history of every change to an app's records. You can revert any change
field by field, merge it with another device's history, and pack old changes
into monthly archives. JournalKit is generic over the app's `Journaled`
records, and it's plain Swift with Foundation, so it builds with the
hermetic toolchains, Linux included.

## Use it

```starlark
deps = ["//apps/libs/kits/journal"]
```

Conform one enum over all of the app's record kinds, so a journal entry can
hold any of them:

```swift
import JournalKit

enum NoteRecord: Journaled {
    case folder(Folder), note(Note)
    enum Kind: String, Codable { case folder, note }
    enum Field: String, Codable { case title, body, folder, trashedAt }
    enum Value: Hashable, Sendable { case text(String), id(UUID), date(Date) }

    var id: RecordID<Kind> { … }
    static func fields(of kind: Kind) -> [Field] { … }
    var fieldValues: [Field: Value] { … }
    func copying(_ field: Field, from other: Self) -> Self { … }
    static var trashField: Field? { .trashedAt }
}

// Record a change.
let entry = JournalEntry(id: UUID(), actor: .person, at: now, steps: [JournalStep(before: old, after: new)])

// Undo it, as far as later changes allow.
switch step.revert(current: store[step.record]) { … }

// Merge another device's version.
switch NoteRecord.merge(base: lastSynced, mine: local, theirs: remote) { … }
```

## What it gives you

- `JournalEntry` is one change: who made it, when, in which conversation,
  which earlier entries it reverts, and its `JournalStep`s, each holding a
  record's before and after. `step.fields` lists the `FieldChange`s.
- `step.revert(current:)` says how to undo a step: remove the record it
  added, restore the one it removed, or put back each field it changed.
  Fields changed again since are skipped, so undoing never overwrites a
  later change. `SkippedRevert` records what was skipped and why, so the
  app can tell the person.
- `Journaled.merge(base:mine:theirs:)` is a three-way merge, field by field.
  Fields in one `group` merge as one. When it can't merge, it returns a
  `SyncConflict` for the person to settle by field or by side.
- `entries.archiving(before:)` packs entries older than a cutoff into
  `ArchivedEntry`s grouped by `JournalMonth`. An archived entry keeps only
  the fields that changed. `merging(_:)` combines the archives of two
  devices.

Bookkeeping that every change bumps, such as an updated-at time, shouldn't
be a `Field`, or every merge would conflict. Carry it over in
`keepingBookkeeping(of:)` instead.

The tests run on a small sample notes app, `Tests/Notes.swift`, which kits
built on this one reuse. Run them with
`bazel test //apps/libs/kits/journal:journal_test`.
