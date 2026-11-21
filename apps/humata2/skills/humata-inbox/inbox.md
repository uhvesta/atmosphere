# The Inbox and the owner's approval

## The rules

For each kind of action the owner sets one of three rules, per channel:

- **Allow**: it runs.
- **Ask me**: it becomes a request in the owner's Inbox, and runs when they
  approve.
- **Never**: it is refused at once (`denied`), and nothing reaches the
  Inbox.

Some actions are the owner's alone (`user_only`). No rule lets an agent do
them and there is no request for them.

    humata access --json                  # your channel's rules

## Check before you act

    humata access why <your-session> <capability> [<target>] --json

says the decision (allow, ask or never), where it comes from (a rule or a
grant), and whether the Mac is away. `<your-session>` is the session from
`humata whoami`. A target is `session:<session>` or `network:<host>:<port>`.
Capabilities are written in lower case:

| Capability | Covers |
| --- | --- |
| `own_board_write`, `channel_board_add`, `channel_board_edit_own`, `channel_board_edit_others`, `global_board_write` | board changes |
| `channel_read`, `global_read` | reading your channel; reading the Global board and other channels |
| `shared_write`, `files_fetch_local` | writing `shared/`; fetching into your clone on your machine |
| `files_read_other_session`, `files_read_other_machine`, `files_copy_in` | other sessions' and machines' files |
| `session_note`, `session_type` | talking to, or typing into, another session |
| `session_start_same_machine`, `session_start_other_machine`, `session_move` | starting, forking and moving sessions |
| `own_session_manage`, `own_session_end`, `session_stop_other`, `session_delete_other` | changing or ending sessions |
| `git_push`, `pr_review_start`, `review_draft` | pushing, pull requests, review drafts |
| `machine_provision`, `machine_destroy`, `machine_script_run`, `network_reach` | machines and the network |
| `channel_create`, `channel_repo_add`, `channel_change` | channels |
| `settings_change` | changing global, channel or agent settings (`Ask me` or `Never`, never `Allow`) |
| `trash_restore`, `views_prepare`, `inbox_ask` | trash, prepared views, asking the owner |

Check when the answer changes your plan, for example before you promise the
owner a fork on another machine. Don't check before every call: the call
itself tells you.

## A pending request

A call under Ask me prints a request id (`rq_...`). With `--wait <seconds>`
it waits that long; then it exits 12 (`pending`).

    humata inbox --json                   # your requests that are waiting (--all: answered too)
    humata inbox show <request> --json    # one request: its state, the answer, the owner's note

What to do while it waits:

1. Don't repeat the call. Approval runs the original call, even when you
   are no longer waiting. If you have to re-run the command, pass the same
   `--key` so it stays one request.
2. Do other useful work that doesn't depend on it, or tell the owner
   plainly: what you asked for, why, and that it is in their Inbox. Then
   end your turn.
3. When it is answered, Humata types a line into your terminal at the end
   of your turn. Read the result with `humata inbox show <request> --json`.
4. Approved: the action already ran. Don't run it again; read its result.
   Denied: it didn't run. Read the note, change the plan, and don't ask for
   the same thing again unless the owner says so.

Make requests easy to answer: one request for what you need, not five
small ones, and say why. The owner sees the action, the target and the
reason on the card.

If you no longer need a pending request, there is no CLI command to take
it back. The MCP tool `inbox_withdraw` (`request_id`) does it. A request is
also withdrawn when your session stops.

## Ask ahead

When you know you will need something later (a read in another session,
a network address), ask now so the owner can answer while you work:

    humata access ask-ahead <capability> <target> --reason "<why, in one sentence>" \
        --json --wait 45 --key <key>

`<target>` is what the access is for, written one of two ways:

- `session:<session>`: another session, for reading its files, copying
  from it, or sending it notes;
- `network:<host>:<port>`: a network address (add `--machine` for the
  machine it is reached from).

An approval covers exactly that target. A capability under Never is denied
at once, and one that is the owner's alone fails with `user_only`. Other
targets (a machine as a whole, a board item) can't be asked ahead: make the
call itself when you need it.

## Ask the owner a question

To get a decision when the owner may not be watching this terminal, put a
question in their Inbox. There is no CLI command for it. Use the MCP tool:

    inbox_ask { "question": "...", "choices": ["...", "..."], "wait_seconds": 45 }

- At most 4 choices. Leave `choices` out for a free answer.
- Write the whole question so it can be answered without your terminal:
  what you are deciding, the options, what each costs, and what you will do
  by default.
- It returns the reply, or `pending` with a `request_id`. Wait again with
  the MCP tool `inbox_wait` (`request_id`), or go on with your default and
  say so. `inbox_mine` lists your requests.

If the MCP tools are not there, ask in your reply as you normally would,
and say that you are waiting.

Ask when the choice is the owner's: scope, anything that could lose work,
spending, anything outside your session. Don't ask about things you can
find out or decide yourself.

## Only the owner

You can never do these, and must not try another way:

- approve, deny or reply to a request (`humata inbox approve|deny|reply`,
  `approve-all`, `deny-all`);
- change rules or grants (`humata access set`, `access away`,
  `access revoke`, `access passphrase`);
- `humata login`, `humata token ...`.

Settings changes (`humata settings set`, channel settings, agent switches)
are **not** `user_only`: calling them puts an approval card in the owner's
Inbox showing the exact `old -> new` diff (never "Always allow").

When something is `user_only`, tell the owner the exact command to run in
their own terminal (outside Humata's sessions), or where it is in the app.

## While the owner's Mac is away

The channel's away rule decides what a call that would ask does: wait
(the default), fail at once with `user_away`, or run low-risk actions and
keep the rest waiting. `humata access why` shows the mode. Under wait, the
request stays until the Mac is back or the owner answers on the machine.
Don't loop; say what is waiting.

## Failures

| What you see | What to do |
| --- | --- |
| exit 12 `pending` | Not a failure. See "A pending request". |
| exit 4 `denied` | The rule is Never, or the owner said no. The error names the rule. Tell the owner; only they can change it in Agent access. Don't retry. |
| exit 5 `user_only` | The owner's alone. Give them the command. |
| exit 7 `user_away` | The Mac is away and the away rule is to skip. Do what you can locally; say what was skipped. |
| exit 7 `request_withdrawn` | The request was taken back (your session stopped, or you withdrew it). Ask again only if you still need it. |
| exit 6 `already_answered` | It was answered. Read it with `humata inbox show`. |
| exit 3 on `inbox show` | Not your request, or a wrong id. `humata inbox --all` lists yours. |

## Not in the CLI

- Asking the owner a question (MCP: `inbox_ask`).
- Waiting again on a request (MCP: `inbox_wait`); the CLI can only
  `inbox show` it.
- Withdrawing a request (MCP: `inbox_withdraw`).
