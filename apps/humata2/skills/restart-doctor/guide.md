# Restart doctor

A restart (a reboot, a crash, a power cut, an update) may leave sessions or
shell tabs not brought back, or operations stuck half done. Humata writes a
restart report each time its daemon starts; `humata doctor restarts` reads
it. Your job is to explain what happened, run the repairs that can't lose
anything, ask before any that could, and leave a diagnosis the user can file
as a bug report if it can't be fixed.

You are not here to start sessions fresh, delete folders or edit Humata's
files by hand. The command does the repairs; you read, decide and ask.

## 1. Read before you change anything

Run:

    humata doctor restarts --explain --json

It reads the newest restart report (name another report's folder after
`restarts` to read an older one) and changes nothing. It works with the
daemon down. Add `--state <folder>` if it can't find the state folder.

Read the result:

- `not_brought_back`: each session or shell tab that wasn't brought back,
  with the step that failed, the problem and the files it needed (a
  conversation file, its folder, saved terminal output) and whether each is
  there.
- `operations`, `stuck` and `owed`: work the restart found half done, each
  with its `kind` (`start`, `relaunch`, `move`, `fork`, `remote_start`,
  `archive`, `rebuild`, `forget`, `remove_machine`, `delete_channel`,
  `restore_channel`, `desk_move`, `comment_send` and others; a kind you
  don't know is still real work, so read its `step` and `detail`), its
  `state` and a `detail`. What each state means and what you do:
  - `finished`: carried on to its end. Nothing to do.
  - `undone`: rolled back on purpose, and `detail` says why. The work did
    not happen; nothing is lost. Tell the user what did not happen and, if
    they still want it, that they can ask for it again.
  - `stuck`: neither finished nor undone, and `detail` says what to do next
    (it is worded as the fix). Do what it says if the user can (for
    example end an old terminal by hand), say so to the user, and note
    that the next start tries again. Never redo the work yourself by hand.
  - `pending` (listed under `owed`): it has no end line, so the restart
    didn't get to settle it, or it waits for something (`detail` says
    what). Treat it as still owed: wait for the next start or the thing it
    waits for, then run the doctor again. If it is still pending then, put
    it in the diagnosis.
  - any other state: a newer Humata wrote it. It is also listed under
    `owed`; report it as written and put it in the diagnosis.
- `repairs`: what can be done, each with a stable id. `safe: true` repairs
  can't lose anything. `safe: false` ones can; they have ids like
  `start-fresh:<session>`.

If nothing is listed under `not_brought_back`, `stuck` or `owed`, say so and stop.

## 2. Work out the cause

For each thing not brought back, say in plain words what went wrong and
why, from the report's fields. Check the facts the report names before you
believe them: does the file at the listed path exist, is the folder there,
is the agent's program installed. Don't guess at a cause the report doesn't
support; say what you can't tell.

Fix what the user can fix first, and say so: put a deleted conversation
file back from a backup, restore the folder, install the agent. A safe
repair that failed only because of one of these will work once it is fixed.

## 3. Apply the safe repairs

Run:

    humata doctor restarts --json

This runs every safe repair (resume a session again with its saved
conversation, start a shell tab again with its saved output, check the
store and, if it is damaged, restart the daemon so it restores its last
good copy) and prints what each did under `applied`. A repair that was
refused or failed says why in its `said`. A repair that was started as an
operation (a resume) goes on after the command returns: `humata ops <id>`
shows how it goes.

If the daemon asks for the user's approval (exit 12, a pending request),
tell them it is in their Inbox and wait for the answer; don't retry.

If the store was damaged, the daemon leaves after it answers. Tell the user
to open the Humata app (or start `humatad2`) again, then run the doctor
again to see what that start brought back.

## 4. Ask before anything that could lose something

A `safe: false` repair is never run for you. Show the user the repair's
`what` text, say what would be lost (for `start-fresh`: the agent will no
longer continue its old conversation, which stays on disk), and ask. Offer
the safe alternative first when there is one. Only when they say yes, run:

    humata doctor restarts --json --allow <id>

once for each repair they agreed to, using the ids from the list. Never
`--allow` something they haven't agreed to.

## 5. Leave a diagnosis

The command writes `diagnosis.md` into the report's folder: the facts, the
repairs and what each did. Add your own account at its top by passing it
in; write it for the person who will read a bug report:

    humata doctor restarts --explain --diagnosis - <<'EOF'
    What went wrong, in one or two sentences.
    Why, as far as the report shows, and what you checked.
    What you repaired and what is still not back.
    What you propose: the fix on the user's side, or the bug in Humata that
    the restart should have handled, with the report's folder as evidence.
    EOF

Use `--explain` for this call so it only writes the diagnosis; use the same
command without it only when you are also running repairs. When something
is still not back and you can't repair it, this diagnosis is the result:
tell the user where it is and that it is ready to file as a bug report.

## Don't

- Don't run `--allow` on your own, or start a session fresh because a
  resume failed once.
- Don't delete or edit anything under Humata's state folder, the restart
  report or the store.
- Don't claim something is repaired until `applied` says `ok` and the
  session or tab is back.
