import Foundation

extension InMemoryDaemon {
    /// Has a client other than the app ask for an admin action, as `humata
    /// mcp` would; it waits in the inbox, as nothing runs it here.
    package func askApproval(_ approval: AdminApproval) {
        state.withValue { $0.approvals.append(approval) }
    }

    /// Grants `scopes` as the person would, until `expires`.
    package func grant(_ scopes: [ApprovalScope], until expires: Date) -> ApprovalGrant {
        state.withValue { state in
            defer { state.nextGrant += 1 }
            let grant = ApprovalGrant(
                id: "g\(state.nextGrant)", scopes: scopes, granted: start, expires: expires)
            state.grants.append(grant)
            return grant
        }
    }

    /// The approvals waiting, oldest first.
    func pendingApprovals() -> [AdminApproval] {
        state.value.approvals.filter { $0.state == .pending }
    }

    func approvals() -> Approvals {
        let state = state.value
        return Approvals(approvals: state.approvals.reversed(), grants: state.grants)
    }

    func decideApprovals(
        _ ids: [String], approve: Bool, scopes: [ApprovalScope], seconds: UInt32
    ) throws -> [AdminApproval] {
        let known = Set(state.value.approvals.map(\.id))
        if let unknown = ids.first(where: { !known.contains($0) }) {
            throw DaemonError.refused(
                .unknownApproval, message: "There's no approval \(unknown); it may have been dropped as an old one.")
        }
        if approve, !scopes.isEmpty {
            _ = grant(scopes, until: start.addingTimeInterval(TimeInterval(seconds)))
        }
        return state.withValue { state in
            var decided: [AdminApproval] = []
            for index in state.approvals.indices
            where ids.contains(state.approvals[index].id) && state.approvals[index].state == .pending {
                state.approvals[index].state = approve ? .approved : .denied
                state.approvals[index].decided = start
                decided.append(state.approvals[index])
            }
            return decided
        }
    }

    func revokeGrant(_ id: String) throws {
        try state.withValue { state in
            guard let index = state.grants.firstIndex(where: { $0.id == id }) else {
                throw DaemonError.refused(
                    .unknownApproval, message: "There's no grant \(id) in force; it was revoked or ran out.")
            }
            state.grants.remove(at: index)
        }
    }
}
