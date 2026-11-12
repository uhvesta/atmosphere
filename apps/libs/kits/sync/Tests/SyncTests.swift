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
