import Foundation
import LifrCore
import LifrTesting

/// Two devices that changed the same records offline, then synced: per
/// record, the version written last wins on both.
private func sync(_ phone: Fixture, _ mac: Fixture) {
    func newer(_ from: Workspace, than local: Workspace) -> [LifrTask] {
        from.tasks.values.filter { task in
            local.tasks[task.id].map { task.updatedAt > $0.updatedAt } ?? true
        }
    }
    let toMac = newer(phone.workspace, than: mac.workspace)
    let toPhone = newer(mac.workspace, than: phone.workspace)
    let phoneAvenues = Array(phone.workspace.avenues.values)
    phone.workspace.applySynced(
        tasks: toPhone,
        avenues: mac.workspace.avenues.values.filter {
            phone.workspace.avenues[$0.id] == nil
        })
    mac.workspace.applySynced(tasks: toMac, avenues: phoneAvenues.filter { mac.workspace.avenues[$0.id] == nil })
}

private func copy(_ fixture: Fixture, device: String) -> Fixture {
    Fixture(copying: fixture, device: device)
}

func reconciliationTests() -> [TestCase] {
    [
        TestCase("given the parent closed on one device and a Sub task added on another, then the parent reopens") {
            let phone = try Fixture()
            let parent = try phone.task("Parent")
            let mac = copy(phone, device: "Mac")
            try mac.task("Added offline", parent: mac.get(parent))
            phone.advance(days: 1)
            _ = try phone.workspace.setStatus(.done, of: parent.id, in: phone.context)
            sync(phone, mac)
            let fixes = phone.workspace.reconcile(in: phone.context)
            try expectEqual(fixes, [.reopenedParent(task: parent.id)])
            try expectEqual(phone.get(parent).status, .inProgress)
            let event = phone.workspace.history.last
            try expectEqual(event?.actor.kind, .system)
            try expectEqual(event?.reason, .reconciledClosedParent)
        },
        TestCase("given two devices that gave tasks the same code, then the newer task gets a new code") {
            let phone = try Fixture()
            let mac = copy(phone, device: "Mac")
            let older = try phone.task("Older")
            mac.advance(days: 1)
            let newer = try mac.task("Newer")
            try expectEqual(older.code, newer.code)
            sync(phone, mac)
            let fixes = phone.workspace.reconcile(in: phone.context)
            try expectEqual(phone.get(older).code, older.code)
            try expect(phone.get(newer).code != older.code)
            try expectEqual(fixes, [.recoded(task: newer.id, from: newer.code, to: phone.get(newer).code)])
        },
        TestCase("given both devices reconcile the same records, then they make the same fixes") {
            let phone = try Fixture()
            let parent = try phone.task("Parent")
            let mac = copy(phone, device: "Mac")
            try phone.task("Older")
            _ = try phone.workspace.setStatus(.done, of: parent.id, in: phone.context)
            mac.advance(days: 1)
            try mac.task("Newer")
            try mac.task("Sub task", parent: mac.get(parent))
            _ = try mac.workspace.createAvenue(key: "HOME", name: "Home", in: mac.context)
            _ = try phone.workspace.createAvenue(key: "HOME", name: "Home", in: phone.context)
            sync(phone, mac)
            let phoneFixes = phone.workspace.reconcile(in: phone.context)
            let macFixes = mac.workspace.reconcile(in: mac.context)
            try expectEqual(phoneFixes, macFixes)
            try expectEqual(
                phone.workspace.tasks,
                mac.workspace.tasks.mapValues { task in
                    var task = task
                    task.updatedAt = phone.workspace.tasks[task.id]!.updatedAt
                    task.lastActivityAt = phone.workspace.tasks[task.id]!.lastActivityAt
                    return task
                })
            try expectEqual(phone.workspace.avenues, mac.workspace.avenues)
        },
        TestCase("given two Avenues with the same key, then the newer one gets a numeric suffix to rename") {
            let phone = try Fixture()
            let mac = copy(phone, device: "Mac")
            let older = try phone.workspace.createAvenue(key: "HOME", name: "Home", in: phone.context)
            mac.advance(days: 1)
            let newer = try mac.workspace.createAvenue(key: "HOME", name: "House", in: mac.context)
            sync(phone, mac)
            _ = phone.workspace.reconcile(in: phone.context)
            try expectEqual(phone.workspace.avenues[older.id]?.key.rawValue, "HOME")
            try expectEqual(phone.workspace.avenues[newer.id]?.key.rawValue, "HOME2")
            try expect(phone.workspace.avenues[newer.id]!.key.needsRename)
            try phone.workspace.setKey("HOUSE", ofAvenue: newer.id)
            try expect(!phone.workspace.avenues[newer.id]!.key.needsRename)
        },
        TestCase("given a 10-letter key that collides, then the suffix still fits in 10 characters") {
            let phone = try Fixture()
            let mac = copy(phone, device: "Mac")
            _ = try phone.workspace.createAvenue(key: "HOUSEHOLDS", name: "One", in: phone.context)
            mac.advance(days: 1)
            let newer = try mac.workspace.createAvenue(key: "HOUSEHOLDS", name: "Two", in: mac.context)
            sync(phone, mac)
            _ = phone.workspace.reconcile(in: phone.context)
            try expectEqual(phone.workspace.avenues[newer.id]?.key.rawValue, "HOUSEHOLD2")
        },
        TestCase("given a Sub task whose parent became a Sub task elsewhere, then it moves to the top-level task") {
            let phone = try Fixture()
            let top = try phone.task("Top")
            let middle = try phone.task("Middle")
            let mac = copy(phone, device: "Mac")
            let leaf = try phone.task("Leaf", parent: middle)
            mac.advance(days: 1)
            _ = try mac.workspace.setParent(top.id, of: middle.id, in: mac.context)
            sync(phone, mac)
            let fixes = phone.workspace.reconcile(in: phone.context)
            try expectEqual(phone.get(leaf).parentID, top.id)
            try expectEqual(fixes, [.reparented(task: leaf.id, from: middle.id, to: top.id)])
        },
        TestCase("given two tasks made each other's parent on two devices, then the older one becomes top-level") {
            let phone = try Fixture()
            let older = try phone.task("Older")
            let newer = try phone.task("Newer")
            let mac = copy(phone, device: "Mac")
            phone.advance(days: 1)
            mac.advance(days: 1)
            _ = try phone.workspace.setParent(older.id, of: newer.id, in: phone.context)
            _ = try mac.workspace.setParent(newer.id, of: older.id, in: mac.context)
            sync(phone, mac)
            _ = phone.workspace.reconcile(in: phone.context)
            try expectEqual(phone.get(older).parentID, nil)
            try expectEqual(phone.get(newer).parentID, older.id)
        },
        TestCase("given a parent moved Avenue while a Sub task was added elsewhere, then the Sub task follows") {
            let phone = try Fixture()
            let home = try phone.workspace.createAvenue(key: "HOME", name: "Home", in: phone.context)
            let parent = try phone.task("Parent")
            let mac = copy(phone, device: "Mac")
            _ = try phone.workspace.move(parent.id, toAvenue: home.id, project: nil, in: phone.context)
            let subtask = try mac.task("Sub task", parent: mac.get(parent))
            phone.workspace.applySynced(tasks: [mac.get(subtask)])
            let fixes = phone.workspace.reconcile(in: phone.context)
            try expectEqual(phone.get(subtask).avenueID, home.id)
            try expectEqual(fixes, [.alignedWithParent(task: subtask.id)])
        },
        TestCase("given records that break no rule, then reconcile changes nothing") {
            let fixture = try Fixture()
            let parent = try fixture.task("Parent")
            try fixture.task("Sub task", parent: parent)
            let before = fixture.workspace
            try expectEqual(fixture.workspace.reconcile(in: fixture.context), [])
            try expectEqual(fixture.workspace, before)
        },
    ]
}
