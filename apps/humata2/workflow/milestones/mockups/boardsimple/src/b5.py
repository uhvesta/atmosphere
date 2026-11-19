from lib import *
import lib

# 1: Home, one agent with a question and a to-do ask
def hrow(dot, name, meta, sub, tm, btn="", cls="", sub2=""):
    return (f'<div class="hrow">{DOTS[dot]}<div style="flex:1"><div class="t">{name}<small>{meta}</small></div><div class="s {cls}">{sub}</div>'
            + (f'<div class="s">{sub2}</div>' if sub2 else "") + f'</div><span class="tm">{tm}</span>{btn}</div>')
home = ('<div class="top"><h1>Home</h1><span class="sp"></span><span class="btn pri">New session</span></div><div class="home">'
        '<div class="hsec" style="margin-top:0">Stopped until you answer · 1</div><div class="hbox">'
        + hrow("o", "Refund retries", "# payments-rewrite · devbox-1 · copilot", "Asks: Which currency should a retried refund use?", "6m", '<span class="btn pri">Answer</span>', "a",
               "Also suggested a to-do, “Idempotency keys for refund retries”. It waits in To-do and isn’t holding the agent up.")
        + '</div><div class="hsec">Ended · 1</div><div class="hbox">'
        + hrow("x", "Upgrade Postgres 17", "# infra · devbox-1 · claude", "Exited with code 1 after the dump step", "40m", '<span class="btn">Resume</span>')
        + '</div><div class="hsec">Working · 4</div><div class="hbox">'
        + hrow("g", "Ledger entries", "# payments-rewrite · devbox-1 · claude", "Writing the cut-over runbook", "now")
        + hrow("g", "Benchmark BRIN", "# infra · devbox-1 · codex", "pgbench run 2 of 4", "1m")
        + hrow("g", "Ledger totals", "# web · This Mac · claude", "Running the checkout tests", "now")
        + hrow("g", "Totals, another way", "# web · This Mac · copilot", "Comparing two table layouts", "3m")
        + '</div><div class="hsec">To-do</div><div class="hbox">'
        + '<div class="hrow"><div style="flex:1"><div class="t">Next: Rotate the staging database credentials<small>INF-6 · pinned · due today</small></div>'
          '<div class="s">10 open · 3 need attention when you get to them · 1 overdue</div></div><span class="btn">Open To-do</span><span class="btn">Do next</span></div>'
        + '</div></div>')
page("b5-1-home-across", home, sel="Home", inbox="1", dot_ledger="g")

# 2: quick add
add = ('<div class="add" style="border-color:var(--blue);box-shadow:0 0 0 3px #dbe8ff;color:#1c1c1e;border-radius:9px 9px 0 0;margin-bottom:0"><span>fix login redirect loop fri #bug web s high<span style="color:var(--blue)">▏</span></span><kbd>↵</kbd></div>'
       '<div style="margin:0 8px 6px;border:1px solid var(--line);border-top:0;border-radius:0 0 9px 9px;padding:9px 12px;background:#fafafb;display:flex;gap:8px;align-items:center;white-space:nowrap">'
       '<span class="ti">Fix login redirect loop</span><span class="due soon">Due tomorrow · Fri 20 Nov</span><span class="tag">bug</span><span class="tag"># web</span><span class="c-sz" style="width:auto"><b>S</b>high</span>'
       '<span class="sp"></span><span style="color:var(--muted);font-size:12.5px">It would be 2nd in your list · <b style="font-weight:600;color:var(--soft)">Enter</b> adds it · everything but the title is optional</span></div>')
todo_page("b5-2-quick-add", "10 open across 4 channels", todo_list(add=add))

# 3: one channel with Waiting, Hidden and Closed shown
PAY9 = I("PAY-9", "Backfill progress bar", "payments-rewrite", ("", "Wed 2 Dec"), ["ledger"], None, "M", "medium"); lib.WHY2[PAY9["title"]] = ("", "Waits on PAY-7 Refund retry with backoff")
H1 = I("PAY-14", "Check the Stripe fee change", "payments-rewrite", None, ["fees"], None, "S", "medium"); lib.WHY2[H1["title"]] = ("", "Comes back Mon 23 Nov")
H2 = I("PAY-11", "Re-run the ledger reconciliation after cut-over", "payments-rewrite", None, ["ledger"], None, "S", "high"); lib.WHY2[H2["title"]] = ("", "Comes back Tue 1 Dec")
REOPEN = '<span class="mini">Reopen</span>'
def closed(key, title, how, when):
    return ('<div class="row"><span class="sb" style="display:flex;align-items:center;justify-content:center;border-radius:50%;background:var(--teal);color:#fff;font-size:12px">✓</span><span class="rk"></span>'
            f'<div class="mid"><div class="l1"><span class="ti" style="color:var(--soft)">{title}</span></div><div class="l2"><span>{how}</span></div></div>'
            + (REOPEN if key else "") + f'<span class="c-ch" style="text-align:right;width:110px">{when}</span><span class="c-k">{key}</span></div>')
lst = (ADD.replace("Add an item…", "Add an item to payments-rewrite…")
       + sec("NEEDS ATTENTION · 2", HD) + row(PAY3, na="close?") + row(NEWI, na="add?")
       + sec("NEXT · 2") + row(PAY7, 2) + row(PAY5, 4)
       + sec("⌄ WAITING ON ANOTHER ITEM · 1", NOTE("joins the list when what it waits on is closed")) + row(PAY9, open_=True)
       + sec("⌄ HIDDEN UNTIL A DATE · 2", NOTE("each comes back on its date, without a notification")) + row(H1, open_=True) + row(H2, open_=True)
       + sec("CLOSED · 3 THIS WEEK", NOTE("reopen within a week; older ones are under History"))
       + closed("PAY-2", "Currency codes table", "Closed by you", "Mon") + closed("PAY-6", "Drop the legacy postings index", "You said yes to claude in Ledger entries", "Tue")
       + closed("", "Review: Currency rounding fix", "Closed itself: you submitted your review of acme/api#471", "Tue"))
todo_page("b5-3-waiting-hidden-closed", "4 open in payments-rewrite", lst, barkw=dict(on="payments-rewrite", closed=True), nxt="PAY-7")
