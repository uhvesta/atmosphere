# Brief macro-05: A long-running project goes from spec to milestones to sessions on a remote machine
Output: `apps/humata2/flows/cases/macro/05-long-running-project.md`

Scenario: over two weeks I turn a spec into board items with dependency edges, start a session per milestone, let them iterate, move two heavy sessions to box-1, spin up a new remote machine box-2 for a third, and survive a reboot in the middle.

Steps: board items created by me and by agents; depends-on edges (DAG / graph view — check what exists); starting a session from a board item; moving sessions; adding a machine; restart report after the reboot; the work continues.

Branches: a move fails mid-copy (left running on source, names the step); box-2 can't be provisioned (MachineService provisioning — check not_built: SetVmTier, ChangeRoot, host variables, TrustNewHostKey); a milestone item depends on one not done; two agents edit the same item (revision conflict); host Away while box-1 sessions keep working.

Pointers: daemon/humatad/tests/boards.rs, board_tools.rs, desktop/tests/live_boards.rs (32 real-daemon tests), features/app/tests/board_start.rs, features/board/tests/*, daemon/humatad/tests/move_session.rs, machines.rs, two_machines.rs, daemon/humatad/src/machine_service.rs not_built list, workflow/milestones/BOARD.md, MACH.md, ANYHOST.md (blocked).
