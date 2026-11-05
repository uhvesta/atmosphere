import Foundation
import LifrCore
import LifrStoreClient

extension Review {
    /// A review for tests and previews, from "Mac": "Buy race gels" retitled
    /// and moved on both devices, and "Pack the bag" trashed on the Mac
    /// while its priority changed here.
    package static func sample(now: Date, timeZone: TimeZone) -> (items: [Item], workspace: Workspace) {
        var records = RecordsBuilder(now: now, timeZone: timeZone)
        let health = records.avenue("HEALTH", "Health")
        let home = records.avenue("HOME", "Home")
        let race = records.project("Race day", in: health)
        let gels = records.task("Buy race gels", in: health, project: race)
        let bag = records.task("Pack the bag", in: health, project: race, priority: .low)

        var mine = gels
        mine.title = "Buy race gels and salt"
        mine.projectID = nil
        var theirs = gels
        theirs.title = "Buy energy gels"
        theirs.avenueID = home.id
        theirs.projectID = nil

        var bagMine = bag
        bagMine.priority = .high
        var bagTheirs = bag
        bagTheirs.deletedAt = now

        let conflicts: [SyncConflict] = [
            RecordValue.merge(base: .task(gels), mine: .task(mine), theirs: .task(theirs)),
            RecordValue.merge(base: .task(bag), mine: .task(bagMine), theirs: .task(bagTheirs)),
        ]
        .compactMap { result in
            guard case .conflict(let conflict) = result else { return nil }
            return conflict
        }
        var workspace = records.workspace
        workspace.importSynced(
            records: [RecordID(.task, gels.id): .task(mine), RecordID(.task, bag.id): .task(bagMine)], journal: [],
            history: [])
        return (conflicts.map { Item(conflict: $0, device: "Mac") }, workspace)
    }
}
