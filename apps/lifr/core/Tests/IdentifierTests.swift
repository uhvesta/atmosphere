import Foundation
import LifrCore
import TestingKit

func identifierTests() -> [TestCase] {
    [
        TestCase("given a code with or without its Avenue key, then it parses to the same code") {
            try expectEqual(try TaskCode("HEALTH-7K2Q"), try TaskCode("7k2q"))
        },
        TestCase("given Crockford look-alikes, then I and L read as 1 and O as 0") {
            try expectEqual(try TaskCode("ilo1"), try TaskCode("1101"))
        },
        TestCase("given a code of the wrong length or with U, then it's rejected") {
            try expectError(.invalidTaskCode("7K2")) { try TaskCode("7K2") }
            try expectError(.invalidTaskCode("7K2U")) { try TaskCode("7K2U") }
        },
        TestCase("given random codes, then each one round-trips through parsing") {
            var generator = SystemRandomNumberGenerator()
            for _ in 0..<100 {
                let code = TaskCode.random(using: &generator)
                try expectEqual(try TaskCode(code.rawValue), code)
            }
        },
        TestCase("given an Avenue key, then it must be 2 to 10 letters") {
            try expectEqual(try AvenueKey("health").rawValue, "HEALTH")
            try expectError(.invalidAvenueKey("H")) { try AvenueKey("H") }
            try expectError(.invalidAvenueKey("ABCDEFGHIJK")) { try AvenueKey("ABCDEFGHIJK") }
            try expectError(.invalidAvenueKey("HEALTH2")) { try AvenueKey("HEALTH2") }
        },
        TestCase("given an Avenue key in use, then a second Avenue can't take it") {
            let fixture = try Fixture()
            try expectError(.avenueKeyTaken(try AvenueKey("HEALTH"))) {
                try fixture.workspace.createAvenue(key: "health", name: "Also health", in: fixture.context)
            }
        },
        TestCase("given a task, then it's found by its code with or without the key") {
            let fixture = try Fixture()
            let task = try fixture.task("Buy running shoes")
            try expectEqual(fixture.workspace.displayCode(of: task), "HEALTH-\(task.code)")
            try expectEqual(try fixture.workspace.task(code: "health-\(task.code)").id, task.id)
            try expectEqual(try fixture.workspace.task(code: task.code.rawValue).id, task.id)
        },
        TestCase("given sort keys made in sequence and between, then they sort in order") {
            var keys = [SortKey.between(nil, nil)]
            for _ in 0..<50 { keys.append(SortKey.between(keys.last, nil)) }
            for _ in 0..<50 { keys.insert(SortKey.between(nil, keys.first), at: 0) }
            for _ in 0..<50 { keys.insert(SortKey.between(keys[0], keys[1]), at: 1) }
            try expectEqual(keys.sorted(), keys)
            try expectEqual(Set(keys).count, keys.count)
        },
    ]
}
