from lib import *

# 1: the terminal after Yes, with the session's To-do panel
term = '''<span class="m">~/src/ledger (ledger-schema) $</span> claude
> Design the ledger schema from PAY-3 and move the reads over.

● Bash(cargo test -p api ledger)
  <span class="gn">⎿ test result: ok. 212 passed; 0 failed</span>
● Bash(psql -f backfill.sql)
  <span class="m">⎿ INSERT 0 3104882 · 4m12s</span>
● humata todo close PAY-3
    --reason "Migration and backfill pass on staging;
              steps 1 to 4 are done."
    --evidence 3 commits, acme/ledger#92, 212 tests
  <span class="m">⎿ Asked the owner. Not waiting; carrying on.</span>

● Writing the cut-over runbook while that waits.
● Write(ledger/docs/cutover.md)
  <span class="m">⎿ Wrote 64 lines</span>

<span class="gn">✓ The owner said yes · PAY-3 Ledger schema v2 is closed</span>
<span class="m">  WEB-13 no longer waits on it.</span>

● PAY-3 is closed. The runbook for step 5 is in
  docs/cutover.md for when you do the cut-over.'''
import lib
closed = ('<div class="row hl"><span class="sb" style="display:flex;align-items:center;justify-content:center;border-radius:50%;background:var(--teal);color:#fff;font-size:12px">✓</span>'
          '<div class="mid"><div class="l1"><span class="ti" style="color:var(--soft);text-decoration:line-through">Ledger schema v2</span></div>'
          '<div class="l2"><span>Closed just now · you said yes to claude</span><span class="mini">Reopen</span></div></div><span class="c-k">PAY-3</span></div>')
W13 = I("WEB-13", "Write the ledger API changelog", "web", None, ["docs"], None, "S", "low")
lib.WHY2[W13["title"]] = ("", "No longer waiting", "PAY-3 closed")
panel = ('<div class="ph"><b>To-do</b><span style="color:var(--muted)">payments-rewrite · this session first</span><span class="sp"></span><span style="color:var(--muted)">⤢ &nbsp;✕</span></div>'
         '<div class="list narrow" style="padding:4px 10px">' + ADD.replace("Add an item… try “fix login redirect fri #bug”", "Add an item to payments-rewrite…")
         + sec("THIS SESSION") + closed
         + sec("NEEDS ATTENTION · 1") + row(NEWI, na="add?")
         + sec("NEXT IN PAYMENTS-REWRITE · 2") + row(PAY7, 1) + row(PAY5, 2)
         + '<div class="fold">›<b>Waiting on another item · 1</b><span>PAY-9 waits on PAY-7</span></div>'
         + '<div style="color:var(--muted);font-size:12.5px;padding:10px 8px">Everything in every channel is in <span class="lnk">To-do</span>.</div></div>')
page("b4-1-after-yes", session("Ledger entries", "payments-rewrite", ["claude", "zsh"], term, panel, "To-do", dot="g"), sel="Ledger entries", inbox="", raw=True, dot_ledger="g")

# 2: a field edit, asked by a session that has ended
body = (ask("Move the due date to Tue 24 Nov?",
            "The pgvector extension has to be rebuilt for Postgres 17 before the upgrade can run, and its 17 build isn't out until Monday. Steps 2 to 4 wait on it.",
            [("Change", "Due: Tue 17 Nov → <b>Tue 24 Nov</b>"), ("Also", "It stops being overdue · nothing waits on this item"), ("Asked by", "claude in Upgrade Postgres 17 · 1h ago · <b>session ended</b> after asking")],
            '<span class="btn">Resume the session to ask why</span>')
        + fr("State", seg(["Open", "In progress", "Closed"], "In progress")) + fr("Channel", "<span># infra</span>")
        + fr("Sessions", '<span class="ses"><i class="dot gr"></i>Upgrade Postgres 17</span><span class="hint">session ended</span><span class="mini" style="margin-left:8px">Resume</span>')
        + fr("Due", '<span class="inp" style="color:var(--red)">Tue 17 Nov</span><span class="hint">overdue by 2 days</span>')
        + fr("Effort", seg("SML", "L") + '<span style="width:96px;color:var(--muted);margin-left:34px">Impact</span>' + seg(["Low", "Medium", "High"], "High"))
        + fr("Tags", '<span class="tag">db</span>' + ADDL) + fr("Waits on", '<span class="hint" style="margin:0">nothing</span>' + ADDL)
        + steps(["Dump and restore on a scratch box"], ["Rebuild pgvector for 17", "Upgrade staging", "Upgrade production"], "ticked by claude")
        + '<div class="h3">DESCRIPTION</div><div class="desc">Move the staging and production clusters from Postgres 15 to 17 with pg_upgrade, extensions included.</div>')
todo_page("b4-2-field-edit-ask", "10 open across 4 channels", todo_list(True, sel="INF-2"), drawer("INF-2 · made by you, Mon 2 Nov", "Postgres 17 upgrade", body, FOOT), narrow=True)

# 3: Do next
sheet = ('<div class="scrim"></div><div class="sheet"><h2>New session for “Rotate the staging database credentials”</h2>'
         + fr("To-do", '<span class="tag">INF-6</span><span class="hint">top of your list · moves to In progress when the session starts</span>')
         + fr("Channel", "<span># infra</span>") + fr("Agent", seg(["claude", "copilot", "codex", "Terminal only"], "claude"))
         + fr("Machine", seg(["This Mac", "devbox-1", "gpu-box"], "devbox-1")) + fr("Repositories", '<span class="tag">acme/infra</span>' + ADDL)
         + '<div class="h3">FIRST MESSAGE</div><div class="area">Rotate the staging database credentials (INF-6).<br><br>'
           'The staging password is 90 days old today. Issue new credentials in Vault, roll the three services that read them, and check each one reconnects.<br><br>'
           'Tick the to-do’s steps as you finish them, and ask me before closing it.</div>'
         + '<div style="display:flex;gap:8px;align-items:center;margin-top:16px"><span class="hint" style="margin:0">Not this one? <span class="lnk">Skip to PAY-7 Refund retry with backoff</span></span><span class="sp"></span><span class="btn">Cancel</span><span class="btn pri">Start session</span></div></div>')
todo_page("b4-3-do-next", "10 open across 4 channels", todo_list(), over=sheet)
