from lib import *

def ft(rows): return '<div class="ft">' + "".join(f'<span>{a}</span><span>{b}</span>' for a, b in rows) + '</div>'
def why(text, item): return cl("WHY", "the agent's reason") + f'<div class="say">“{text}”<br><i>Working on {item}</i></div>'

FILE_MOVE = dict(
    title="Move the folder ledger-2026 from This Mac to devbox-1?",
    who="claude in Ledger entries · payments-rewrite · 2m ago",
    left=ft([("From", "<b>This Mac</b> · Ledger entries · api/fixtures/ledger-2026/"), ("To", "<b>devbox-1</b> · Ledger entries · api/fixtures/ledger-2026/"), ("Size", "412 files · 1.8 GB")])
         + why("The backfill runs on devbox-1 and reads these fixture rows. Nothing on This Mac uses them after today's run.", "PAY-3 Ledger schema v2"),
    right=cl("WHAT ELSE IT AFFECTS", "worked out by Humata")
         + li("!", "<b>2 files on devbox-1 are replaced:</b> seed.sql (the one there is 3 days newer) and accounts.csv (same content)", "w")
         + li("!", "The folder leaves This Mac: this is a move, not a copy", "w")
         + li("✓", "410 files are new on devbox-1", "ok")
         + li("✓", "No running terminal is inside this folder", "ok")
         + cl("CAN IT BE UNDONE?") + '<div>Yes, for 7 days. Undo moves the folder back and restores the 2 replaced files.</div>',
    rule=rule("Move files to another machine"), always="Always allow for this session")

SESSION_MOVE = dict(
    title="Move the session Benchmark BRIN from devbox-1 to gpu-box?",
    who="codex in Benchmark BRIN · infra · 4m ago",
    left=ft([("From", "<b>devbox-1</b> · infra · Benchmark BRIN"), ("To", "<b>gpu-box</b> · infra · Benchmark BRIN"), ("Carries", "2 repos (infra, ledger) · 2.1 GB · 3 uncommitted files, as they are")])
         + why("pgbench at 30M rows needs more memory than devbox-1 has; it started swapping at 22M. gpu-box has 64 GB free.", "INF-9 Benchmark BRIN on real data"),
    right=cl("WHAT ELSE IT AFFECTS", "worked out by Humata")
         + li("✓", "<b>2 terminals move along and keep running:</b> codex, and zsh running pgbench (14 min in)", "ok")
         + li("✓", "INF-9 stays in progress with this session", "ok")
         + li("!", "gpu-box has no clone of acme/ledger yet; Humata clones it there first (about 2 min)", "w")
         + li("!", "Its fork, Partial BRIN, stays on devbox-1", "w")
         + cl("CAN IT BE UNDONE?") + '<div>Yes. Move it back at any time; nothing is removed from devbox-1 until the move has finished.</div>',
    rule=rule("Move a session to another machine"), always="Always allow for this session")

FILE_COPY = dict(
    title="Copy 3 files from Refund retries into Ledger entries?",
    who="claude in Ledger entries · payments-rewrite · 1m ago",
    left=ft([("From", "<b>Refund retries</b> (copilot's session) · devbox-1 · api/src/retry/"), ("To", "<b>Ledger entries</b> · devbox-1 · api/src/retry/"), ("Files", "backoff.rs (4 KB) · policy.rs (2 KB) · client.rs (11 KB)")])
         + why("Refund retries already wrote the backoff policy. The ledger client should use the same one instead of a second copy that drifts.", "PAY-3 Ledger schema v2"),
    right=cl("WHAT ELSE IT AFFECTS", "worked out by Humata")
         + li("!", "<b>client.rs is overwritten</b>, and yours has 2 uncommitted changes that would be lost (+62 −21)", "w")
         + li("✓", "backoff.rs and policy.rs are new in Ledger entries", "ok")
         + li("✓", "Nothing changes in Refund retries; copilot isn't interrupted", "ok")
         + li("!", "The copies are uncommitted work from another session's branch, refund-retries", "w")
         + cl("CAN IT BE UNDONE?") + '<div>Yes, for 7 days. Undo removes the 2 new files and restores your client.rs with its changes.</div>',
    rule=rule("Copy files from another session"), always="Always allow between these two sessions")

def C(d, **kw): return card(d["title"], d["who"], d["left"], d["right"], d["rule"], d["always"], **kw)
