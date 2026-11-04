import Foundation

/// A rule violation that two offline devices created together, and how
/// `Workspace.reconcile` fixed it.
package enum Fix: Hashable, Sendable {
    /// The Sub task's parent became a Sub task, so it moved to the top-level
    /// task. `to` is nil when the parents formed a cycle and this task became
    /// top-level.
    case reparented(task: UUID, from: UUID, to: UUID?)
    /// The Sub task's Avenue or Project differed from its parent's.
    case alignedWithParent(task: UUID)
    /// The task was closed with an open Sub task, so it's In Progress again.
    case reopenedParent(task: UUID)
    /// Two tasks had the same code; the newer one got a new code.
    case recoded(task: UUID, from: TaskCode, to: TaskCode)
    /// Two Avenues had the same key; the newer one got a numeric suffix, and
    /// the app should ask for a new key.
    case suffixedAvenueKey(avenue: UUID, from: AvenueKey, to: AvenueKey)
}

extension Workspace {
    /// Writes records that sync brought in from another device, as they are:
    /// per record, the last writer wins. They may break a rule in combination
    /// with local changes, so call `reconcile` once the import is done.
    package mutating func applySynced(
        tasks synced: [LifrTask] = [], avenues syncedAvenues: [Avenue] = [],
        history syncedHistory: [HistoryEvent] = []
    ) {
        for task in synced { tasks[task.id] = task }
        for avenue in syncedAvenues { avenues[avenue.id] = avenue }
        let known = Set(history.map(\.id))
        history += syncedHistory.filter { !known.contains($0.id) }
    }

    /// Fixes violations after a sync import and writes a system history event
    /// for each task it changes. Every fix depends only on the records, so two
    /// devices reconciling the same records make the same changes.
    package mutating func reconcile(in context: Context) -> [Fix] {
        var fixes: [Fix] = []
        var change = Change()
        let system = context.system

        // Oldest first, so ties always break the same way.
        func byAge(_ lhs: LifrTask, _ rhs: LifrTask) -> Bool {
            (lhs.createdAt, lhs.id.uuidString) < (rhs.createdAt, rhs.id.uuidString)
        }

        // 1. Only one level of nesting: move Sub tasks to the top-level task.
        for task in tasks.values.sorted(by: byAge) {
            guard let parentID = tasks[task.id]?.parentID, tasks[parentID]?.parentID != nil else { continue }
            let root = topLevel(from: task.id)
            let newParent = root == task.id ? nil : root
            update(task.id, in: context, &change) { $0.parentID = newParent }
            record(
                .fieldChanged, on: task.id, field: .parent, oldValue: tasks[parentID]?.code.rawValue,
                newValue: newParent.flatMap { tasks[$0]?.code.rawValue },
                reason: .reconciledNestedSubtask, by: system, in: context, &change)
            fixes.append(.reparented(task: task.id, from: parentID, to: newParent))
        }

        // 2. Sub tasks take their parent's Avenue and Project.
        for task in tasks.values.sorted(by: byAge) {
            guard let parent = task.parentID.flatMap({ tasks[$0] }),
                task.avenueID != parent.avenueID || task.projectID != parent.projectID
            else { continue }
            place(
                task.id, avenue: parent.avenueID, project: parent.projectID, by: system,
                reason: .reconciledParentPlacement, in: context, &change)
            fixes.append(.alignedWithParent(task: task.id))
        }

        // 3. A closed task can't keep open Sub tasks.
        for task in tasks.values.sorted(by: byAge)
        where task.status.isClosed && task.deletedAt == nil && !openSubtasks(of: task.id).isEmpty {
            apply(.inProgress, to: task.id, by: system, reason: .reconciledClosedParent, in: context, &change)
            fixes.append(.reopenedParent(task: task.id))
        }

        // 4. Codes are unique: the newer task gets a code derived from its id.
        var used = Set<TaskCode>()
        for task in tasks.values.sorted(by: byAge) {
            guard used.contains(task.code) else {
                used.insert(task.code)
                continue
            }
            var attempt = 0
            var code = TaskCode.derived(from: task.id, attempt: attempt)
            while used.contains(code) || tasks.values.contains(where: { $0.code == code }) {
                attempt += 1
                code = TaskCode.derived(from: task.id, attempt: attempt)
            }
            used.insert(code)
            update(task.id, in: context, &change) { $0.code = code }
            record(
                .fieldChanged, on: task.id, field: .code, oldValue: task.code.rawValue,
                newValue: code.rawValue, reason: .reconciledDuplicateCode, by: system,
                in: context, &change)
            fixes.append(.recoded(task: task.id, from: task.code, to: code))
        }

        // 5. Avenue keys are unique: the newer Avenue gets a numeric suffix.
        var keys = Set<AvenueKey>()
        for avenue in avenues.values.sorted(by: { ($0.createdAt, $0.id.uuidString) < ($1.createdAt, $1.id.uuidString) })
        {
            guard keys.contains(avenue.key) else {
                keys.insert(avenue.key)
                continue
            }
            let taken = Set(avenues.values.map(\.key)).union(keys)
            var number = 2
            var key = Self.suffixed(avenue.key, number)
            while taken.contains(key) {
                number += 1
                key = Self.suffixed(avenue.key, number)
            }
            keys.insert(key)
            avenues[avenue.id]?.key = key
            fixes.append(.suffixedAvenueKey(avenue: avenue.id, from: avenue.key, to: key))
        }

        return fixes
    }

    /// The top-level task above `id`. If the parents form a cycle, the oldest
    /// task in it is treated as top-level.
    func topLevel(from id: UUID) -> UUID {
        var path = [id]
        var current = id
        while let parent = tasks[current]?.parentID, tasks[parent] != nil {
            if let start = path.firstIndex(of: parent) {
                let cycle = path[start...].compactMap { tasks[$0] }
                return cycle.min { ($0.createdAt, $0.id.uuidString) < ($1.createdAt, $1.id.uuidString) }?.id ?? id
            }
            path.append(parent)
            current = parent
        }
        return current
    }

    static func suffixed(_ key: AvenueKey, _ number: Int) -> AvenueKey {
        let digits = String(number)
        return AvenueKey(reconciled: String(key.letters.prefix(10 - digits.count)) + digits)
    }
}
