from lib import *

# 1: the pull request review item, opened
def prline(repo, title, stat, checks):
    return (f'<div style="display:flex;gap:10px;align-items:baseline;padding:5px 0"><span class="tag">{repo}</span><span style="flex:1">{title}</span>'
            f'<span style="color:var(--muted);font-size:12.5px;white-space:nowrap">{stat} · {checks}</span></div>')
body = ('<div class="askc" style="background:#f1effd"><div class="t" style="font-size:14.5px">dana-k asked for your review 2 days ago</div>'
        '<div class="q" style="color:#2c2650;margin:5px 0 9px">“Reads go to the new ledger tables behind a flag, off by default. The web change only swaps the totals query. Second pair of eyes on the flag fallback, please.”</div>'
        '<div class="b"><span class="btn pri">Start the review</span><span class="hint" style="margin:0 0 0 6px">opens a session in pr-reviews with both pull requests laid out</span></div></div>'
        '<div class="h3">PULL REQUESTS · A STACK OF 2, REVIEWED TOGETHER</div>'
        + prline("acme/api#483", "Ledger reads behind a flag", "+205 −45 · 6 files", "checks pass")
        + prline("acme/web#210", "Read totals from the ledger", "+71 −30 · 2 files", "checks pass")
        + '<div style="height:6px"></div>'
        + fr("Author", "<span>dana-k</span>") + fr("Also asked", '<span>sam-r <span class="hint" style="margin-left:4px">approved yesterday</span></span>')
        + fr("Waiting", "<span>2 days · asked Tue 17 Nov</span>") + fr("Channel", "<span># pr-reviews</span>")
        + fr("Rank", '<span>3rd<span class="hint">it climbs the longer it waits on you</span></span>')
        + '<div class="h3">THIS ITEM LOOKS AFTER ITSELF</div>'
          '<div class="desc">It closes when you submit your review, when dana-k withdraws the request, or when both pull requests are merged or closed. '
          'Its title, author and pull requests come from GitHub, so there is nothing to edit here. You can pin it, hide it until a date, or hide it for good.</div>')
foot = '<span class="btn">Open on GitHub ↗</span><span class="sp"></span><span style="color:var(--muted)">☆ Pin</span><span style="color:var(--muted);margin-left:14px">Hide until…</span><span style="color:var(--muted);margin-left:14px">Hide</span>'
todo_page("b7-1-pr-review-item", "10 open across 4 channels", todo_list(True, sel="PR"), drawer("Pull request review · from GitHub", "Review: Ledger reads behind a flag", body, foot), narrow=True)

# 2: the review session it starts
term = '''<span class="m">~/reviews/ledger-reads $</span> claude
<span class="m">Started from your to-do “Review: Ledger reads
behind a flag”. dana-k asked 2 days ago.</span>

● This is a stack of 2 pull requests, 8 files:
    1 api   acme/api#483  reads behind a flag
    2 web   acme/web#210  totals from the ledger

● I've laid out layer 1 in the Diff panel, the
  flag and its fallback first, since that's what
  dana-k asked about.

  I don't review. Ask me what anything does, or
  to show something another way.

<span class="m">The to-do closes itself when you submit.</span>'''
def f(name, path, a, d): return f'<div style="display:flex;gap:8px;padding:5px 14px 5px 38px"><span class="cb" style="border-radius:50%"></span><span>{name}</span><span style="color:var(--muted)">{path}</span><span class="sp"></span><span style="color:#0f8a3c">+{a}</span><span style="color:#c0392b">−{d}</span></div>'
def lay(n, name, sub, on=False): return f'<div style="display:flex;gap:10px;align-items:center;padding:7px 14px;{"background:#eef0f4;border-radius:8px;margin:0 6px" if on else "margin:0 6px"}"><span style="width:18px;height:18px;border-radius:50%;border:1.4px solid #8a8a90;font-size:11px;display:flex;align-items:center;justify-content:center;{"background:var(--blue);color:#fff;border-color:var(--blue)" if on else ""}">{n}</span><b>{name}</b><span style="color:var(--muted)">{sub}</span></div>'
panel = ('<div class="ph"><b>Diff</b><span style="color:var(--muted)">2 pull requests · 8 files</span><span class="sp"></span><span style="color:var(--muted)">⤢ &nbsp;✕</span></div>'
         '<div class="sec" style="padding:12px 14px 6px">STACK · 2 LAYERS</div>' + lay(1, "api", "acme/api#483 · 6 files", True) + lay(2, "web", "acme/web#210 · 2 files")
         + '<div style="border-top:1px solid var(--line);margin-top:10px;padding-top:8px"><div style="padding:5px 14px;display:flex;gap:8px"><b>api</b><span style="color:var(--muted)">against main</span><span class="sp"></span><span style="color:#0f8a3c">+205</span><span style="color:#c0392b">−45</span></div>'
         + f("flags.rs", "src/", 18, 0) + f("reads.rs", "src/ledger/", 81, 21) + f("client.rs", "src/", 62, 21) + f("entry.rs", "src/", 30, 3) + f("reads_test.rs", "tests/", 14, 0) + f("CHANGELOG.md", "", 0, 0).replace("+0", "+2").replace("−0", "−0")
         + '</div><div class="sp"></div><div style="border-top:1px solid var(--line);padding:12px 14px;display:flex;align-items:center;gap:10px"><span style="color:var(--muted)">No comments yet</span><span class="sp"></span><span class="btn pri">Submit review…</span></div>')
page("b7-2-pr-review-session", session("Ledger reads behind a flag", "pr-reviews", ["claude"], term, panel, "Diff"), sel="Ledger reads behind a flag", inbox="", raw=True, dot_ledger="g")

# 3: a suggested item, opened
body = (ask("Add this item to your list?",
            "While adding the retry I saw that a refund retried after a timeout can be sent twice: the provider may have accepted the first try. An idempotency key per refund stops that. It is separate from PAY-7, so I didn’t do it there.",
            [("Evidence", '<span class="lnk">refunds/retry.rs:88</span> · <span class="lnk">a failing test that shows the double refund</span>'),
             ("Change", "A new item in payments-rewrite · it would be <b>3rd</b> in your list"),
             ("Asked by", "copilot in Refund retries · 35m ago")])
        + fr("State", seg(["Open", "In progress", "Closed"], "Open") + '<span class="hint">once added</span>') + fr("Channel", "<span># payments-rewrite</span>")
        + fr("Due", '<span class="inp" style="color:var(--muted)">No date</span>')
        + fr("Effort", seg("SML", "S") + '<span style="width:96px;color:var(--muted);margin-left:34px">Impact</span>' + seg(["Low", "Medium", "High"], "High"))
        + fr("Tags", '<span class="tag">refunds</span>' + ADDL)
        + '<div class="h3">DESCRIPTION · AS THE AGENT WROTE IT</div><div class="desc">Send an idempotency key with every refund request, derived from the refund id, so that a retry after a timeout can’t pay out twice.</div>'
        + '<div style="margin-top:14px;color:var(--muted);font-size:12.5px;line-height:1.5">You can change any field before saying yes. Until then it has no key and no place in the ranking, and Do next never picks it.</div>')
foot2 = '<span style="color:var(--muted)">Say yes to add it, or no to drop it. The agent is told either way and isn’t waiting.</span>'
todo_page("b7-3-suggested-item", "10 open across 4 channels", todo_list(True, sel="NEW"), drawer("Suggested · not in your list yet", "Idempotency keys for refund retries", body, foot2), narrow=True)
