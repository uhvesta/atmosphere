# Brief agents-03: A permission prompt goes to the Inbox
Output: `apps/humata2/flows/cases/agents/03-permission-prompt-through-the-inbox.md`

Scenario: a Claude agent wants to run `rm -rf build/`. Its hook (written by the daemon, `humata hook`) holds and sends the ask to the Inbox. I see the card on my Mac and press Allow; later a Codex agent's ask I Deny with a note; a third I answer Always.

Steps: the daemon writes a blocking hook that holds for an hour; the card shows the command; Allow reaches the waiting hook; Deny reaches it with its note; each agent gets the answer in its own shape (Claude, Codex, Copilot); Always grants only the target the card offered.

Branches: the same ask held twice is one card and both get the answer; a hook that stops holding withdraws its card (can't be answered); a payload that isn't an ask gets no card; only the session's own agent may hold a hook; Answer all leaves permission cards alone; the daemon is gone (hook prints nothing, exits 0 → agent falls back to its own prompt); Inbox asks turned off; the live hook checks owed (HOOKS milestone).

Pointers: daemon/humatad/tests/hooks.rs (12 tests), desktop/tests/live_inbox.rs `a permission ask waits in the Inbox and Allow reaches its hook` (real daemon), cli/src/commands/hook.rs, features/inbox/tests/*, features/app/tests/permission_alerts.rs, workflow/milestones/HOOKS.md.
