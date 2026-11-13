import Foundation

/// What every list works out from the same tasks: worked out once for a
/// workspace and kept until its tasks or Avenues change. Copies of a
/// workspace share it until one of them changes, so it's never stale.
/// It isn't part of a workspace's value: any two compare equal, it hashes
/// to nothing and it isn't saved.
struct Derived: Hashable, Sendable {
    private let box = Box()

    static func == (_: Derived, _: Derived) -> Bool { true }

    func hash(into _: inout Hasher) {}

    func listed(_ make: () -> [LifrTask]) -> [LifrTask] {
        box.lock.withLock {
            if let listed = box.listed { return listed }
            let listed = make()
            box.listed = listed
            return listed
        }
    }

    func children(_ make: () -> [UUID: [LifrTask]]) -> [UUID: [LifrTask]] {
        box.lock.withLock {
            if let children = box.children { return children }
            let children = make()
            box.children = children
            return children
        }
    }

    /// Only touched while `lock` is held.
    private final class Box: @unchecked Sendable {
        let lock = NSLock()
        var listed: [LifrTask]?
        var children: [UUID: [LifrTask]]?
    }
}

extension Workspace {
    /// Tasks that are in the trash or in an archived Avenue appear in no
    /// list; the rest, in list order.
    package var listedTasks: [LifrTask] {
        derived.listed {
            let live = tasks.values.filter { $0.deletedAt == nil && avenues[$0.avenueID]?.archivedAt == nil }
            // Sorts small keys, not whole tasks: moving a task costs a copy
            // of every field, and a sort of thousands moves each many times.
            return live.indices.map { ListKey(live[$0], at: $0) }.sorted().map { live[$0.index] }
        }
    }

    /// Each task's Sub tasks, not counting trashed ones, in sort order.
    var liveChildren: [UUID: [LifrTask]] {
        derived.children {
            var children: [UUID: [LifrTask]] = [:]
            for task in tasks.values where task.deletedAt == nil {
                if let parent = task.parentID { children[parent, default: []].append(task) }
            }
            return children.mapValues { $0.sorted(by: LifrTask.siblingOrder) }
        }
    }
}

/// Where a task falls in `LifrTask.listOrder`, without the rest of it.
private struct ListKey: Comparable {
    let priority: Int
    /// The due day as one number, undated last.
    let due: Int
    let sortOrder: String
    let id: UUID
    let index: Int

    init(_ task: LifrTask, at index: Int) {
        priority = task.priority.rank
        due = task.dueDate.map { $0.year * 10_000 + $0.month * 100 + $0.day } ?? .max
        sortOrder = task.sortOrder
        id = task.id
        self.index = index
    }

    static func < (lhs: Self, rhs: Self) -> Bool {
        if lhs.priority != rhs.priority { return lhs.priority > rhs.priority }
        if lhs.due != rhs.due { return lhs.due < rhs.due }
        if lhs.sortOrder != rhs.sortOrder { return lhs.sortOrder < rhs.sortOrder }
        return lhs.id.uuidString < rhs.id.uuidString
    }

    static func == (lhs: Self, rhs: Self) -> Bool { lhs.id == rhs.id }
}
