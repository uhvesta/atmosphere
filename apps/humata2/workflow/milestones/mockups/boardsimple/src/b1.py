from lib import *

# 1: the main list
todo_page("b1-1-list", "10 open across 4 channels", todo_list())

# 2: a Needs attention item, opened
body = (ask("Close Ledger schema v2?", "Migration and backfill pass on staging: 3.1M rows in 4m12s. Steps 1 to 4 are done; step 5, the production cut-over, is yours.",
            [("Evidence", '<span class="lnk">3 commits</span> · <span class="lnk">acme/ledger#92</span> · <span class="lnk">212 tests passed</span>'), ("Change", "State: In progress → <b>Closed</b>"),
             ("Also", "WEB-13 stops waiting · the session keeps running"), ("Asked by", "claude in Ledger entries · 20m ago")])
        + fr("State", seg(["Open", "In progress", "Closed"], "In progress")) + fr("Channel", "<span># payments-rewrite</span>")
        + fr("Sessions", '<span class="ses"><i class="dot o"></i>Ledger entries</span>' + ADDL)
        + fr("Due", '<span class="inp">Fri 20 Nov</span><span class="hint">tomorrow · or type “next week”</span>')
        + fr("Effort", seg("SML", "M") + '<span style="width:96px;color:var(--muted);margin-left:34px">Impact</span>' + seg(["Low", "Medium", "High"], "High"))
        + fr("Tags", '<span class="tag">migration</span>&nbsp;<span class="tag">db</span>' + ADDL)
        + fr("Waits on", '<span class="tag" style="color:var(--teal);background:var(--teal-soft)">PAY-2 Currency codes table ✓</span>' + ADDL)
        + steps(["Add ledger_entries and ledger_balances", "Dual-write from the posting path", "Backfill the old table in batches", "Verify the balances match on staging"], ["Cut production reads over"], "ticked by claude in Ledger entries")
        + '<div class="h3">DESCRIPTION</div><div class="desc">Split postings into ledger_entries and ledger_balances, backfill from the old table in batches, and keep both in step until cut-over.</div>')
todo_page("b1-2-item-open", "10 open across 4 channels", todo_list(True, sel="PAY-3"),
          drawer("PAY-3 · made by you, Mon 9 Nov", "Ledger schema v2", body, FOOT.replace("Start a session on this", "Start another session on this")), narrow=True)

# 3: one channel, due soon, Filters open
def pr(label, chips, on=()):
    return f'<div class="pr"><span>{label}</span><div class="cs">' + "".join(f'<span class="chip{" on" if c in on else ""}">{c}</span>' for c in chips) + '</div></div>'
pop = ('<div class="pop"><div class="h3">DUE</div>' + pr("", ["Overdue", "This week", "After this week", "No date"], ("Overdue", "This week")).replace("<span></span>", "")
       + '<div class="h3">TAGS</div>' + pr("", ["api", "bug", "db", "ledger", "migration", "refunds"]).replace("<span></span>", "")
       + '<div class="h3">MORE</div>' + pr("Session", ["Ledger entries", "Refund retries", "API rate limiter", "None yet"])
       + pr("Effort", ["S", "M", "L"]) + pr("Impact", ["Low", "Medium", "High"]) + pr("Only", ["Needs attention", "Pull request reviews"])
       + '<div class="pf"><span>3 of 4 items in payments-rewrite</span><span><span style="color:var(--blue);margin-right:14px">Reset</span><span class="btn">Done</span></span></div></div>')
extra = '<span class="chip" style="box-shadow:inset 0 0 0 1px var(--blue);background:#fff;color:var(--blue);margin-left:8px">Due: overdue or this week ✕</span>'
lst3 = (ADD.replace("Add an item…", "Add an item to payments-rewrite…") + sec("NEEDS ATTENTION · 1", HD) + row(PAY3, na="close?")
        + sec("NEXT · 2", NOTE("their places in the whole list are 2 and 4")) + row(PAY7, 2) + row(PAY5, 4)
        + '<div class="fold">›<b>Left out by the due filter · 2</b><span>PAY-9 Backfill progress bar · a suggested item with no date, “Idempotency keys for refund retries”</span></div>')
page("b1-3-channel-due-soon", top("4 open in payments-rewrite", 1, "PAY-7") + bar("payments-rewrite", extra) + f'<div class="body"><div class="list">{lst3}</div></div>' + pop)
