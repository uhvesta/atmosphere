# Brief macro-01: Pinged on chat, I start a desk session to look
Output: `apps/humata2/flows/cases/macro/01-pinged-on-chat-start-a-desk-session.md`

Scenario: a teammate pings me in a chat app: "can you look at whether we could batch the export job?". I open Humata 2, go to the Desk, start a new desk session titled "batch export idea", optionally with acme/api cloned, paste the message as the first prompt, read what the agent finds, keep the doc it writes, copy it into the acme channel's shared folder, then close the session.

Steps: Desk page lists open and closed; New desk session form; folder named from the title (a taken name gets a suffix; untitled gets a fallback); repos cloned into the folder; the agent runs; markdown in the folder is listed; copy to a channel's shared folder never overwrites; closing stops the agent and keeps the folder seven days; reopening resumes the conversation in the same folder.

Branches: the clone fails (nothing starts, no folder left); I pick an existing folder (must exist and sit outside Humata; never deleted); another machine is refused for desk; the retried start makes one session; work not pushed holds the folder past seven days; I move the desk session to box-1 from its bar; reboot while open.

Pointers: daemon/humatad/tests/desk.rs (many; real daemon), desktop/tests/native/desk.rs (fakes), features/desk/tests/*.rs, SPEC §26 desk flow (grep `26\.` and `DK-`), workflow/milestones/DESK.md.
