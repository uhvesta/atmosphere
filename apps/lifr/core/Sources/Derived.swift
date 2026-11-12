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
            tasks.values
                .filter { $0.deletedAt == nil && avenues[$0.avenueID]?.archivedAt == nil }
                .sorted(by: LifrTask.listOrder)
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
