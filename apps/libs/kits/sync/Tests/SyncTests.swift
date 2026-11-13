import Foundation
import JournalKit
import SyncKit
import TestingKit

private func pair() -> (InMemoryCloud<Notebook>, Device, Device) {
    let cloud = InMemoryCloud<Notebook>()
    var notebook = Notebook()
    notebook.put(shopping)
    return (cloud, Device("iPhone", cloud: cloud, notebook: notebook), Device("Mac", cloud: cloud))
}

/// Both devices have `shopping`, synced.
private func synced() async throws -> (InMemoryCloud<Notebook>, Device, Device) {
    let (cloud, phone, mac) = pair()
    await phone.syncer.sync()
    await mac.syncer.sync()
    try expectEqual(try mac.records.note(shopping.id), shopping)
    return (cloud, phone, mac)
}

func itemTests() -> [TestCase] {
    [
        TestCase("given a removed record, then it's saved without a value and reads back removed") {
            let item = CloudItem<Notebook>.record(RecordID(.note, shopping.id), nil)
            let data = try JSONEncoder().encode(item)
            try expect(!(String(bytes: data, encoding: .utf8) ?? "").contains("_1"))
            try expectEqual(try JSONDecoder().decode(CloudItem<Notebook>.self, from: data), item)
        },
        TestCase("given a log, then it's saved as the log itself") {
            let edit = Edit(id: UUID(), summary: "Renamed")
            let data = try JSONEncoder().encode(CloudItem<Notebook>.log(edit))
            try expectEqual(try JSONDecoder().decode(Edit.self, from: data), edit)
            try expectEqual(try JSONDecoder().decode(CloudItem<Notebook>.self, from: data), .log(edit))
        },
    ]
}

func syncTests() -> [TestCase] {
    [
        TestCase.async("given a change while a sync runs, when sync is asked for again, then the change is sent") {
            let cloud = InMemoryCloud<Notebook>()
            let stored = Mutexed(Notebook())
            let first = Mutexed(true)
            let (loading, loaded) = AsyncStream<Void>.makeStream()
            let (gate, open) = AsyncStream<Void>.makeStream()
            let store = SyncStore<Notebook>(
                load: {
                    let notebook = stored.value
                    var isFirst = false
                    first.update { was in
                        isFirst = was
                        was = false
                    }
                    if isFirst {
                        loaded.yield()
                        for await _ in gate {}
                    }
                    return notebook
                },
                importSynced: { synced in
                    stored.update { notebook in for (id, value) in synced.records { notebook.items[id] = value } }
                },
                resolve: { _ in })
            let syncer = Syncer(cloud: cloud.connection(device: "iPhone"), store: store, storage: .inMemory())

            let running = Task { await syncer.sync() }
            for await _ in loading { break }
            stored.update { $0.put(shopping) }
            let asked = Task { await syncer.sync() }
            // Lets the second ask reach the syncer before the first run
            // goes on.
            for _ in 0..<100 { await Task.yield() }
            open.finish()
            _ = await running.value
            _ = await asked.value

            let mac = Device("Mac", cloud: cloud)
            await mac.syncer.sync()
            try expectEqual(try mac.records.note(shopping.id), shopping)
        },
        TestCase.async("given nothing changed, when sync runs again soon, then the ledger isn't written again") {
            let cloud = InMemoryCloud<Notebook>()
            let stored = Mutexed(Notebook())
            stored.update { $0.put(shopping) }
            let store = SyncStore<Notebook>(
                load: { stored.value }, importSynced: { _ in stored.value }, resolve: { _ in })
            let memory = LedgerStorage<Notebook>.inMemory()
            let saves = Mutexed(0)
            let storage = LedgerStorage<Notebook>(
                load: { try memory.load() },
                save: { ledger in
                    saves.update { $0 += 1 }
                    try memory.save(ledger)
                })
            let clock = Mutexed(Date(timeIntervalSince1970: 1_000_000))
            let syncer = Syncer(
                cloud: cloud.connection(device: "iPhone"), store: store, storage: storage, now: { clock.value })
            await syncer.sync()
            let afterFirst = saves.value

            clock.update { $0 += 60 }
            let idle = await syncer.sync()
            try expectEqual(saves.value, afterFirst)
            try expectEqual(idle.lastSynced, clock.value)

            clock.update { $0 += 60 * 60 }
            await syncer.sync()
            try expect(saves.value > afterFirst)
            try expectEqual(try memory.load().lastSynced, clock.value)
        },
        TestCase.async("given an edit made during a fetch of another device's change, then both are kept") {
            let (cloud, phone, _) = pair()
            await phone.syncer.sync()
            let stored = Mutexed(Notebook())
            let editsWhileFetching = Mutexed(false)
            let connection = cloud.connection(device: "Mac")
            let fetching = Cloud<Notebook>(
                fetch: {
                    let changes = try await connection.fetch()
                    if editsWhileFetching.value { stored.update { $0.put(Note(id: shopping.id, title: "Groceries")) } }
                    return changes
                },
                send: connection.send)
            let store = SyncStore<Notebook>(
                load: { stored.value },
                importSynced: { synced in
                    stored.update { notebook in for (id, value) in synced.records { notebook.items[id] = value } }
                },
                resolve: { _ in })
            let mac = Syncer(cloud: fetching, store: store, storage: .inMemory())
            await mac.sync()

            phone.edit { notebook in
                var note = shopping
                note.body = "Milk"
                notebook.put(note)
            }
            await phone.syncer.sync()
            editsWhileFetching.update { $0 = true }
            await mac.sync()

            let note = try stored.value.note(shopping.id)
            try expectEqual(note.title, "Groceries")
            try expectEqual(note.body, "Milk")
        },
        TestCase.async("given a note on one device, when both sync, then the other has it") {
            _ = try await synced()
        },
        TestCase.async("given each device changed a different field, when both sync, then both changes are kept") {
            let (_, phone, mac) = try await synced()
            phone.edit { $0.put(Note(id: shopping.id, title: "Groceries")) }
            mac.edit { notebook in
                var note = shopping
                note.body = "Milk"
                notebook.put(note)
            }
            await phone.syncer.sync()
            await mac.syncer.sync()
            await phone.syncer.sync()
            for device in [phone, mac] {
                let note = try device.records.note(shopping.id)
                try expectEqual(note.title, "Groceries")
                try expectEqual(note.body, "Milk")
            }
        },
        TestCase.async("given both devices retitled a note, when they sync, then the second holds it for review") {
            let (_, phone, mac) = try await synced()
            phone.edit { $0.put(Note(id: shopping.id, title: "Groceries")) }
            mac.edit { $0.put(Note(id: shopping.id, title: "Weekly shop")) }
            await phone.syncer.sync()
            let state = await mac.syncer.sync()
            try expectEqual(state.review.map(\.conflict.kind), [.fields([.title])])
            try expectEqual(try mac.records.note(shopping.id).title, "Weekly shop")
        },
        TestCase.async("given a held note, when the person keeps the other device's title, then both devices have it") {
            let (_, phone, mac) = try await synced()
            phone.edit { $0.put(Note(id: shopping.id, title: "Groceries")) }
            mac.edit { $0.put(Note(id: shopping.id, title: "Weekly shop")) }
            await phone.syncer.sync()
            await mac.syncer.sync()
            let state = await mac.syncer.resolve([RecordID(.note, shopping.id): [.title: .theirs]])
            try expectEqual(state.review, [])
            await phone.syncer.sync()
            try expectEqual(try mac.records.note(shopping.id).title, "Groceries")
            try expectEqual(try phone.records.note(shopping.id).title, "Groceries")
        },
        TestCase.async("given a held note, when the person chooses offline, then it isn't asked again") {
            let (cloud, phone, mac) = try await synced()
            phone.edit { $0.put(Note(id: shopping.id, title: "Groceries")) }
            mac.edit { $0.put(Note(id: shopping.id, title: "Weekly shop")) }
            await phone.syncer.sync()
            await mac.syncer.sync()
            cloud.setOnline(false, device: "Mac")
            let state = await mac.syncer.resolve([RecordID(.note, shopping.id): [.title: .mine]])
            try expectEqual(state.status, .failed(.unreachable("offline")))
            try expectEqual(state.review, [])
            cloud.setOnline(true, device: "Mac")
            await mac.syncer.sync()
            await phone.syncer.sync()
            try expectEqual(try phone.records.note(shopping.id).title, "Weekly shop")
        },
        TestCase.async("given a held note, when the person keeps their own title, then the other device gets it") {
            let (_, phone, mac) = try await synced()
            phone.edit { $0.put(Note(id: shopping.id, title: "Groceries")) }
            mac.edit { $0.put(Note(id: shopping.id, title: "Weekly shop")) }
            await phone.syncer.sync()
            await mac.syncer.sync()
            await mac.syncer.resolve([RecordID(.note, shopping.id): [.title: .mine]])
            await phone.syncer.sync()
            try expectEqual(try phone.records.note(shopping.id).title, "Weekly shop")
        },
        TestCase.async("given a device that can't save what it fetched, when both sync again, then nothing reverts") {
            let (_, phone, mac) = try await synced()
            phone.edit { $0.put(Note(id: shopping.id, title: "Groceries")) }
            await phone.syncer.sync()
            mac.failsImport.update { $0 = true }
            let failed = await mac.syncer.sync()
            try expect(failed.status != .idle)
            mac.failsImport.update { $0 = false }
            await mac.syncer.sync()
            await phone.syncer.sync()
            try expectEqual(try phone.records.note(shopping.id).title, "Groceries")
        },
        TestCase.async("given a note one device can't read, when it syncs, then the other device keeps it") {
            let (_, phone, mac) = try await synced()
            let id = RecordID(NoteKind.note, shopping.id)
            mac.edit { notebook in
                notebook.items[id] = nil
                notebook.unreadable = [id]
            }
            await mac.syncer.sync()
            await phone.syncer.sync()
            try expectEqual(try phone.records.note(shopping.id), shopping)
        },
        TestCase.async("given a device offline, when it syncs, then it fails and sends once it's back") {
            let (cloud, phone, mac) = pair()
            cloud.setOnline(false, device: "iPhone")
            let state = await phone.syncer.sync()
            try expectEqual(state.status, .failed(.unreachable("offline")))
            cloud.setOnline(true, device: "iPhone")
            await phone.syncer.sync()
            await mac.syncer.sync()
            try expectEqual(try mac.records.note(shopping.id), shopping)
        },
        TestCase.async("given logs on both devices, when they sync twice, then each has every log once") {
            let (_, phone, mac) = try await synced()
            phone.edit { $0.logs.append(Edit(id: UUID(), summary: "On the phone")) }
            mac.edit { $0.logs.append(Edit(id: UUID(), summary: "On the Mac")) }
            for _ in 0..<2 {
                await phone.syncer.sync()
                await mac.syncer.sync()
            }
            try expectEqual(Set(phone.records.logs.map(\.summary)), ["On the phone", "On the Mac"])
            try expectEqual(mac.records.logs.count, 2)
        },
        TestCase.async("given a removed note, when both sync, then the other device removes it too") {
            let (_, phone, mac) = try await synced()
            phone.edit { $0.items[RecordID(.note, shopping.id)] = nil }
            await phone.syncer.sync()
            await mac.syncer.sync()
            try expectEqual(mac.records.items[RecordID(.note, shopping.id)], nil)
        },
    ]
}
