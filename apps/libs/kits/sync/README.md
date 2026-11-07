# SyncKit

Syncs an app's JournalKit records between devices. It fetches what other
devices changed and merges it field by field. What can't merge is held for
the person to review, and the rest is sent. The cloud is two closures, so
the same `Syncer` runs against CloudKit in the app and against an in-memory
cloud in tests and previews. It's plain Swift, so it builds with the
hermetic toolchains.

## Use it

```starlark
deps = ["//apps/libs/kits/sync"]                # features
deps = ["//apps/libs/kits/sync/cloudkit"]       # only the app's entry point
```

Describe the app's records as a `SyncSnapshot`: every record by ID, plus
logs such as journal entries, which never change once written. Then hand
the `Syncer` a `SyncStore` that loads them, imports what came in, and saves
what the person settled:

```swift
import SyncKit

let store = SyncStore<Notebook>(
    load: { try await database.snapshot() },
    importSynced: { synced in try await database.apply(synced) },
    resolve: { records in try await database.save(records) })

// In tests and previews:
let cloud = InMemoryCloud<Notebook>()
let syncer = Syncer(cloud: cloud.connection(device: "phone"), store: store, storage: .inMemory())

// In the app:
let syncer = try CloudKitSync.syncer(
    container: "iCloud.com.example.notes", zone: "Notes", device: deviceName,
    subsystem: "com.example.notes", store: store)

let state = await syncer.sync()                 // .idle, .syncing or .failed(SyncError)
let settled = await syncer.resolve([note.id: [.title: .theirs]])
```

## What it gives you

- `Syncer` is an actor. `sync()` runs one round and returns the
  `SyncState`, and `stream()` reports each change to that state.
  `state.review` holds the conflicts, each with the other device's name.
  `resolve(_:)` takes the side the person chose for each field.
- `SyncLedger` remembers what each record looked like when it last synced,
  which the three-way merge uses as its base. `LedgerStorage` keeps the
  ledger.
- `SyncError` says what failed and what to do about it: sign in to iCloud,
  free up iCloud storage, wait for the network, and so on.
- `sync/cloudkit` runs a `CKSyncEngine` in the private database. Each item
  is encrypted end to end, and the engine's state is kept in files on the
  device. Without a container, such as in a build signed without a team, it
  throws `SyncError.notSetUp`.

Test it with `bazel test //apps/libs/kits/sync:sync_test`, which syncs two
devices of JournalKit's sample notes app through an in-memory cloud. The
CloudKit target's tests (`--config=xcode`) check the record encoding,
error mapping and files. CloudKit itself needs a signed app and an iCloud
account, so it's tried by hand.
