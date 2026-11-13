import Foundation

extension InMemoryDaemon {
    /// Shows a cleanup as going on, as the real daemon does while it works.
    package func startCleanup(_ job: Storage.Job) {
        state.withValue { $0.storage.job = job }
    }

    /// Marks the sessions last used at `date`, as the daemon tells.
    package func setLastActive(_ date: Date, of sessions: [String]) {
        state.withValue { state in
            for index in state.sessions.indices where sessions.contains(state.sessions[index].id) {
                state.sessions[index].lastActive = date
            }
        }
    }

    /// Cleans up at once: deletes the archived sessions unused for
    /// `deleteAfterDays`, keepers and sessions forked from left with why.
    func cleanUp(_ cleanup: Storage.Cleanup) -> Storage {
        let (deleteAfterDays, compact) = (cleanup.deleteAfterDays, cleanup.compact)
        return state.withValue { state in
            let before = start.addingTimeInterval(-Double(deleteAfterDays) * 86_400)
            var skipped: [Storage.Skip] = []
            var deleted = 0
            if deleteAfterDays > 0 {
                for session in state.sessions where session.archived && (session.lastActive ?? start) < before {
                    if session.keeper {
                        skipped.append(
                            .init(sessionID: session.id, name: session.name, reason: "It's its task's keeper."))
                    } else if state.sessions.contains(where: { $0.parentID == session.id }) {
                        skipped.append(
                            .init(sessionID: session.id, name: session.name, reason: "A fork of it is kept."))
                    } else {
                        state.sessions.removeAll { $0.id == session.id }
                        deleted += 1
                    }
                }
            }
            func free(_ kind: Storage.Kind, _ bytes: UInt64) {
                for index in state.storage.uses.indices where state.storage.uses[index].kind == kind {
                    state.storage.uses[index].bytes -= min(bytes, state.storage.uses[index].bytes)
                }
            }
            if cleanup.putAwayWorktrees {
                free(.worktrees, state.storage.archivedWorktreesBytes)
                state.storage.archivedWorktreesBytes = 0
            }
            if cleanup.removeUnusedClones {
                free(.clones, state.storage.unusedClonesBytes)
                state.storage.unusedClonesBytes = 0
            }
            if compact {
                for index in state.storage.uses.indices where state.storage.uses[index].kind != .worktrees {
                    state.storage.uses[index].bytes = state.storage.uses[index].bytes / 10 * 7
                }
            }
            state.storage.job = nil
            state.storage.skipped = skipped
            state.storage.deletedSessions = deleted
            state.storage.cleaned = start
            state.storage.failure = nil
            return state.storage
        }
    }
}

extension InMemoryDaemon {
    /// The skills the Mac and hosts have, as the daemon would list them.
    package func setSkills(_ skills: SkillList) {
        state.withValue { $0.skills = skills }
    }

    func pickSkill(_ name: String, _ candidates: [String], _ winner: String) throws {
        try state.withValue { state in
            guard
                let index = state.skills.clashes.firstIndex(where: {
                    $0.name == name && Set($0.candidates.map(\.hash)) == Set(candidates)
                }), candidates.contains(winner)
            else {
                throw DaemonError.refused(
                    .invalidRequest, message: "There's no such clash of skills named \(name); list the skills again.")
            }
            state.skills.clashes[index].winner = winner
        }
    }
}
