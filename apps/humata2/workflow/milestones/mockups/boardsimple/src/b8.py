from lib import *
import lib, random

# 1: empty
empty = ('<div style="padding:70px 0 0;display:flex;flex-direction:column;align-items:center;gap:8px">'
         '<div style="font-size:18px;font-weight:700">Nothing to do yet</div>'
         '<div style="color:var(--soft);max-width:520px;text-align:center;line-height:1.5">Type a title above and press Enter. A date, an effort and an impact are optional; the more an item has, the better the list can order itself.</div>'
         '<div style="color:var(--muted);margin-top:8px;max-width:520px;text-align:center;line-height:1.5">Agents can suggest items and changes. They show here marked Needs attention, and nothing joins your list until you say yes. Pull requests waiting for your review show up by themselves.</div></div>')
page("b8-1-empty", top("no items", nxt="nothing yet").replace('class="btn pri">Do next', 'class="btn pri" style="opacity:.4">Do next')
     + bar(counts=[("All", "0")]) + f'<div class="body"><div class="list">{ADD.replace("border:1px", "border:1px")}{empty}</div></div>', inbox="")

# 2: bare items, titles only
bare = [("Fix the login redirect loop", "Added 5 days ago"), ("Ask Dana about the ledger cut-over date", "Added 4 days ago"), ("Rate limiter: decide per-key or per-account", "Added 3 days ago"),
        ("Clean up the old staging buckets", "Added 2 days ago"), ("Write up the refund retry design", "Added yesterday"), ("Try the new pgbench flags", "Added today")]
rows = ""
for i, (t, w) in enumerate(bare):
    it = I(f"TD-{4 + i}", t, "No channel"); lib.WHY2[t] = ("", w)
    rows += row(it, i + 1, open_=True)
hint = ('<div style="margin:14px 8px 0;padding:10px 14px;background:#f5f5f8;border-radius:9px;color:var(--soft)">With titles only, the oldest comes first. Give any item a due date or an impact and it moves to where it belongs. '
        '<span class="lnk">Dismiss</span></div>')
page("b8-2-bare-list", top("6 open", nxt="TD-4") + bar(counts=[("All", "6"), ("No channel", "6")]) + f'<div class="body"><div class="list">{ADD}{sec("NEXT · 6", HD)}{rows}{hint}</div></div>', inbox="")

# 3: sixty items
random.seed(7)
titles = ["Ledger balances drift check", "Refund webhook signature rotation", "Checkout totals rounding in JPY", "Staging backup restore drill", "Alert on backfill lag over 5 min", "SDK: typed errors for refunds",
          "Invoice table pagination", "Move cron jobs to the scheduler", "Rate limiter metrics dashboard", "Drop the legacy postings table", "Web: empty state for no invoices", "Vault policy for the ledger service"]
chs = ["payments-rewrite", "infra", "web", "sdk", "billing", "payments-rewrite"]
whys = ["High impact for a small job", "Open for 5 weeks", "2 items wait on it", "High impact", "Open for 3 weeks", "Small job"]
dues = [("over", "Overdue · 3 days"), ("soon", "Due today"), ("soon", "Due tomorrow"), ("", "Mon 23 Nov"), ("", "Tue 24 Nov"), ("", "Fri 27 Nov"), None, None, None, None, None, None]
rows = ""
for i, t in enumerate(titles):
    ch = chs[i % 6]; key = {"payments-rewrite": "PAY", "infra": "INF", "web": "WEB", "sdk": "SDK", "billing": "BIL"}[ch] + f"-{20 + i * 3}"
    it = I(key, t, ch, dues[i], [["ledger"], ["security"], ["checkout", "bug"], ["ops"], ["ops"], ["sdk"], ["ui"], ["ops"], ["api"], ["db"], ["ui"], ["security"]][i], None, "SML"[i % 3], ["high", "high", "medium", "high", "medium", "medium", "low", "medium", "medium", "high", "low", "medium"][i])
    lib.WHY2[t] = ("p", "Pinned") if i == 0 else ("", whys[i % 6])
    rows += row(it, i + 1, open_=True)
cnt = [("All", "60"), ("payments-rewrite", "19"), ("infra", "14"), ("web", "11"), ("sdk", "7"), ("billing", "5"), ("pr-reviews", "3"), ("No channel", "1")]
strip = ('<div style="position:absolute;left:0;right:0;bottom:0;height:40px;background:rgba(255,255,255,.96);border-top:1px solid var(--line);display:flex;align-items:center;padding:0 28px;gap:18px;color:var(--muted);font-size:12.5px;z-index:4">'
         '<span><b style="color:var(--soft);font-weight:600">1–12 of 48 ranked</b> · scroll for the rest</span><span>Needs attention 4 and In progress 6 are folded above</span><span class="sp"></span><span>Waiting 4 · Hidden until a date 2</span><span><kbd style="border:1px solid #d9d9de;border-radius:4px;padding:0 5px">/</kbd> to search</span></div>'
         '<div style="position:absolute;right:4px;top:118px;width:7px;height:150px;border-radius:4px;background:#c9c9cf;z-index:5"></div>')
lst = ('<div class="fold" style="border-top:0;margin-top:4px">›<b>Needs attention · 4</b><span>2 to close · 1 new due date · 1 suggested item</span></div>'
       '<div class="fold" style="margin-top:0">›<b>In progress · 6</b><span>5 sessions working · 1 session ended</span></div>'
       + sec("NEXT · 48", NOTE("ranked for you") + HD) + rows)
page("b8-3-sixty-items", top("60 open across 7 channels", nxt="PAY-20") + bar(counts=cnt) + f'<div class="body" style="overflow:hidden"><div class="list">{lst}</div></div>{strip}', inbox="")
