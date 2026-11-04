import Foundation
import LifrAPI
import LifrCore

/// Hands out IDs 00000000-…-000000000001 on and task codes 7001 on.
final class Sequences {
    private var ids = 0
    private var codes = 0

    func context(_ actor: Actor, at now: Date) -> Context {
        Context(
            actor: actor, now: now,
            newID: { [self] in
                ids += 1
                return UUID(uuidString: String(format: "00000000-0000-0000-0000-%012d", ids))!
            },
            newCode: { [self] in
                codes += 1
                // A malformed code is a bug in the fixture.
                // swiftlint:disable:next force_try
                return try! TaskCode(String(format: "%04d", 7000 + codes))
            })
    }
}

/// A workspace with three Avenues (HEALTH, WORK read-only for agents, and
/// FINANCE hidden from them), a Project, a label and a few tasks. IDs, codes
/// and the clock are fixed, so every run gives the same answers.
///
/// Tasks: HEALTH-7001 "Training plan" (In Progress, untouched for two weeks)
/// with Sub tasks 7002 and 7003, HEALTH-7004 "Book a checkup", WORK-7005 and
/// FINANCE-7006.
final class Fixture {
    static let week: TimeInterval = 7 * 24 * 60 * 60
    /// 2026-11-03T00:00:00Z.
    let now = Date(timeIntervalSince1970: 1_793_664_000)
    let device = "Avesta's Mac"
    let sequences = Sequences()
    var workspace = Workspace()
    private var requests = 0

    var agent: Grant { Grant(client: "Claude Code", scopes: [.read, .write], expiresAt: now + Self.week) }
    var readOnlyAgent: Grant { Grant(client: "Claude Code", scopes: [.read], expiresAt: now + Self.week) }
    var terminal: Grant {
        Grant(
            client: "Terminal", scopes: [.read, .write], expiresAt: now + Unlock.idleTimeout,
            isTerminalSession: true)
    }

    init() throws {
        let person = sequences.context(.user(device: device, surface: .cli), at: now)
        let health = try workspace.createAvenue(key: "HEALTH", name: "Health", symbol: "heart", in: person)
        let work = try workspace.createAvenue(key: "WORK", name: "Work", symbol: "briefcase", in: person)
        let finance = try workspace.createAvenue(key: "FINANCE", name: "Finance", symbol: "banknote", in: person)
        try workspace.setAgentAccess(.readOnly, ofAvenue: work.id)
        try workspace.setAgentAccess(.hidden, ofAvenue: finance.id)
        let marathon = try workspace.createProject(
            name: "Half marathon", inAvenue: health.id, dueDate: try CalendarDate("2027-04-18"), in: person)
        let running = workspace.createLabel(name: "running", color: "green", in: person)

        var plan = Workspace.NewTask(title: "Training plan", avenueID: health.id, projectID: marathon.id)
        plan.status = .inProgress
        plan.priority = .high
        plan.dueDate = try CalendarDate("2026-11-10")
        plan.labelIDs = [running.id]
        let started = now - 2 * Self.week
        let planID = try create(plan, at: started)
        var longRun = Workspace.NewTask(title: "Long run, 12 km", parentID: planID)
        longRun.dueDate = try CalendarDate("2026-11-08")
        try create(longRun, at: started)
        try create(Workspace.NewTask(title: "Buy shoes", parentID: planID), at: started)
        var checkup = Workspace.NewTask(title: "Book a checkup", avenueID: health.id)
        checkup.details = "Ask about knee pain."
        try create(checkup)
        var review = Workspace.NewTask(title: "Quarterly review", avenueID: work.id)
        review.dueDate = try CalendarDate("2026-11-20")
        try create(review)
        try create(Workspace.NewTask(title: "File taxes", avenueID: finance.id))
        _ = try workspace.addComment(
            "Start with 3 runs a week.", to: planID,
            in: sequences.context(.user(device: device, surface: .cli), at: started))
    }

    @discardableResult
    private func create(_ new: Workspace.NewTask, at time: Date? = nil) throws -> UUID {
        let change = try workspace.createTask(
            new, in: sequences.context(.user(device: device, surface: .cli), at: time ?? now))
        return change.previous.first { $0.value == nil }!.key
    }

    /// The task with this code, as it is now.
    func task(_ code: String) throws -> LifrTask { try workspace.task(code: code) }

    /// Sends one request as `grant`, against this fixture's workspace.
    @discardableResult
    func call(
        _ method: String, _ params: JSONValue = [:], grant: Grant?,
        confirm: @escaping (Confirmation) -> Bool = { _ in true }
    ) -> (request: RPCRequest, response: RPCResponse) {
        requests += 1
        let request = RPCRequest(id: requests, method: method, params: params, surface: .cli)
        return (request, handler(grant, confirm).handle(request, in: &workspace))
    }

    func call(line: String, grant: Grant?) -> RPCResponse {
        handler(grant, { _ in true }).handle(line: Data(line.utf8), in: &workspace)
    }

    private func handler(_ grant: Grant?, _ confirm: @escaping (Confirmation) -> Bool) -> Handler {
        Handler(
            grant: grant, context: sequences.context(.user(device: device, surface: .cli), at: now), confirm: confirm)
    }
}
