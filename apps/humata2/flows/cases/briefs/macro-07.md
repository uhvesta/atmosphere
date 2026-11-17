# Brief macro-07: Working across two machines
Output: `apps/humata2/flows/cases/macro/07-working-across-two-machines.md`

Scenario: this Mac is the host; box-1 is a Linux box over SSH. I start one session on each, move the Mac one to box-1 before leaving, open the app on a second computer's browser, copy a file from box-1 to this Mac, and box-1 goes offline for an hour.

Steps: Machines page and health; session start picks a machine; Move (⌥⌘M) with its plan; files across machines (cp/mv transfers); box-1 offline: sessions show offline, actions refused with reason; link returns: things catch up; the browser view.

Branches: a fork onto box-1; storage of another machine is not_built (machine_service.rs:650); a moved shell tab comes back on the new machine with its output; the copy left on an offline target is taken back when the link is up; launchd for macOS remotes (OVERVIEW gap); web listener (gap).

Pointers: daemon/humatad/tests/two_machines.rs, remote*.rs, move_session.rs, fork_remote.rs, remote_transfers.rs, remote_files.rs; desktop/tests/machines_flow.rs, native/machines.rs, native/fork_move.rs; features/hosts/tests/*; OVERVIEW.md gaps (peer link never over real SSH).
