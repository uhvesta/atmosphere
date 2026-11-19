from lib import *
from cards import *
import lib

# 1: the day after migration
old = [("PAY-3", "Ledger schema v2", "payments-rewrite", "Was in In progress", True), ("INF-2", "Postgres 17 upgrade", "infra", "Was in In progress", True)]
nxt = [("PAY-5", "API rate limiter", "payments-rewrite", "p", "Pinned · was VIP"), ("PAY-9", "Backfill progress bar", "payments-rewrite", "", "Was 1st in To do"), ("PAY-7", "Refund retry with backoff", "payments-rewrite", "", "Was 2nd in To do"),
       ("INF-6", "Rotate the staging database credentials", "infra", "", "Was 1st in To do on infra"), ("WEB-11", "Fix the flaky cart end-to-end test", "web", "", "Was 3rd in To do"),
       ("PAY-12", "Multi-currency ledgers", "payments-rewrite", "", "Was 1st in Backlog"), ("INF-4", "Move CI to devbox runners", "infra", "", "Was 2nd in Backlog"), ("WEB-12", "Dark mode for the invoices table", "web", "", "Was 3rd in Backlog")]
banner = ('<div style="margin:8px 8px 2px;padding:11px 14px;background:#eef4ff;border-radius:9px;display:flex;gap:12px;align-items:center"><div style="flex:1;line-height:1.5"><b>Your boards became this list yesterday.</b> All 37 items are here, in the order you left them: In progress, then To do, then Backlog. '
          'Nothing has a date, effort or impact yet; add them where they matter and the order starts to follow.</div><span class="lnk">What changed</span><span class="btn">Got it</span></div>')
rows = sec("IN PROGRESS · 6", NOTE("2 of 6 shown") + HD)
for k, t, ch, w, _ in old:
    it = I(k, t, ch, ses=[("o" if k == "PAY-3" else "gr", "Ledger entries" if k == "PAY-3" else "Upgrade Postgres 17")], ended=k != "PAY-3"); rows += row(it)
rows += sec("NEXT · 20", NOTE("in your old order, until dates and impact say otherwise"))
for i, (k, t, ch, c, w) in enumerate(nxt):
    it = I(k, t, ch); lib.WHY2[t] = (c, w); rows += row(it, i + 1, open_=True)
todo_page("b9-1-day-after-migration", "28 open across 4 channels", ADD + banner + rows, nxt="PAY-5",
          barkw=dict(counts=[("All", "28"), ("payments-rewrite", "12"), ("infra", "8"), ("web", "6"), ("No channel", "2")]), inbox="1")

# 2: the terminal after No
term = '''<span class="m">~/src/api (ledger-schema) $</span> claude
> Use the same backoff policy as Refund retries.

● humata files copy
    --from "Refund retries" api/src/retry/
    --reason "Refund retries already wrote the backoff
              policy. The ledger client should use the
              same one instead of a second copy."
  <span class="am">⎿ Asked the owner in the Agent inbox. Waiting.</span>

<span class="m">  The owner asked why. Answered.</span>

<span style="color:#c0392b">✕ The owner said no</span> <span class="m">· 2m after asking</span>
<span class="m">  Their note:</span> “Keep my client.rs. Copy the two policy
  files only and add the imports by hand.”
<span class="m">  Nothing was copied.</span>

● Understood. Asking again for backoff.rs and
  policy.rs only.
● humata files copy --from "Refund retries"
    api/src/retry/backoff.rs api/src/retry/policy.rs
  <span class="gn">⎿ Allowed: you chose “Always allow between these
    two sessions” for files that overwrite nothing.</span>
● Edit(api/src/client.rs)
  <span class="m">⎿ Added 2 imports</span>'''
hist = ('<div class="ph"><b>Agent inbox</b><span style="color:var(--muted)">this session · nothing waiting</span><span class="sp"></span><span style="color:var(--muted)">⤢ &nbsp;✕</span></div>'
        '<div style="padding:14px"><div style="color:var(--muted);font-size:11.5px;font-weight:600;margin-bottom:8px">ANSWERED TODAY</div>'
        '<div class="card" style="padding:12px 14px"><div class="hd"><span style="color:#c0392b;font-weight:700">✕</span><h3 style="font-size:14px">Copy 3 files from Refund retries into Ledger entries?</h3></div>'
        '<div style="margin-top:6px;color:var(--soft);line-height:1.5">You said <b>no</b>, 3m ago, with a note: “Keep my client.rs. Copy the two policy files only and add the imports by hand.”</div>'
        '<div style="margin-top:6px;color:var(--muted);font-size:12.5px">Nothing was copied. claude was told and carried on.</div></div>'
        '<div class="card" style="padding:12px 14px;margin-top:10px"><div class="hd"><span style="color:var(--teal);font-weight:700">✓</span><h3 style="font-size:14px">Copy backoff.rs and policy.rs from Refund retries</h3></div>'
        '<div style="margin-top:6px;color:var(--soft);line-height:1.5">Allowed without asking, 2m ago, by your rule “Always allow between these two sessions” (files that overwrite nothing).</div>'
        '<div style="margin-top:6px;font-size:12.5px"><span class="lnk">Undo the copy</span> · <span class="lnk">Change this rule</span></div></div></div>')
page("b9-2-after-no", session("Ledger entries", "payments-rewrite", ["claude", "zsh"], term, hist, "Agent inbox", dot="g"), sel="Ledger entries", inbox="", raw=True, dot_ledger="g")

# 3: a Needs attention item opened in the session's To-do panel
term3 = '''<span class="m">~/src/ledger (ledger-schema) $</span> claude
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
<span class="am">✱ Writing docs/cutover.md… (40s · esc to interrupt)</span>'''
pbody = (ask("Close Ledger schema v2?", "Migration and backfill pass on staging: 3.1M rows in 4m12s. Steps 1 to 4 are done; step 5, the production cut-over, is yours.",
             [("Evidence", '<span class="lnk">3 commits</span> · <span class="lnk">acme/ledger#92</span> · <span class="lnk">212 tests passed</span>'), ("Change", "State: In progress → <b>Closed</b>"),
              ("Also", "WEB-13 stops waiting · this session keeps running"), ("Asked by", "claude in this session · 1m ago")], '<span class="btn">Ask why</span>')
         + fr("State", seg(["Open", "In progress", "Closed"], "In progress")) + fr("Due", '<span class="inp">Fri 20 Nov</span><span class="hint">tomorrow</span>')
         + fr("Effort", seg("SML", "M") + '<span style="color:var(--muted);margin:0 14px 0 24px">Impact</span>' + seg(["Low", "Medium", "High"], "High"))
         + fr("Tags", '<span class="tag">migration</span>&nbsp;<span class="tag">db</span>' + ADDL)
         + steps(["Add ledger_entries and ledger_balances", "Dual-write from the posting path", "Backfill the old table in batches", "Verify the balances match on staging"], ["Cut production reads over"], "ticked by claude"))
panel3 = ('<div class="ph"><span class="lnk">‹ To-do</span><span style="color:var(--muted)">PAY-3 · payments-rewrite</span><span class="sp"></span><span style="color:var(--muted)">⤢ &nbsp;✕</span></div>'
          f'<div style="padding:14px 16px;display:flex;flex-direction:column;flex:1"><h2 style="font-size:18px;margin-bottom:8px">Ledger schema v2</h2>{pbody}'
          '<div class="foot" style="padding-bottom:4px"><span style="color:var(--muted)">History</span><span style="color:var(--muted);margin-left:14px">☆ Pin</span><span style="color:var(--muted);margin-left:14px">Hide until…</span></div></div>')
page("b9-3-session-todo-ask", session("Ledger entries", "payments-rewrite", ["claude", "zsh"], term3, panel3, "To-do", dot="g"), sel="Ledger entries", inbox="", raw=True, dot_ledger="g")
