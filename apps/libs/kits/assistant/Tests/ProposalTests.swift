import AssistantKit
import Foundation
import TestingKit

func proposalTests() -> [TestCase] {
    [
        TestCase("given a new proposal, then it's pending until the person applies or declines it") {
            var proposal = Proposal<String, String>(actions: ["rename"], steps: ["Shopping → Groceries"])
            try expect(proposal.isPending)
            proposal.decision = .declined
            try expect(!proposal.isPending)
        },
        TestCase("given a decided proposal, then it reads back with its decision") {
            let proposal = Proposal<String, String>(actions: ["rename"], steps: ["step"], decision: .applied)
            let read = try JSONDecoder().decode(
                Proposal<String, String>.self, from: JSONEncoder().encode(proposal))
            try expectEqual(read, proposal)
        },
    ]
}
