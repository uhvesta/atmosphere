from cards import *

YN = '<span class="btn pri" style="min-width:64px;text-align:center">Yes</span><span class="btn" style="min-width:64px;text-align:center">No</span>'
asked = ('<div class="why2"><div class="t">You asked why · the copy is on hold</div>'
         '<div class="say"><i>claude, 40s ago:</i> “I need backoff.rs and policy.rs. client.rs only came along because it imports them. If you’d rather keep your client.rs, say no and I’ll copy the two files and add the imports to yours by hand.”</div></div>')
ch = (("All", "1"), ("payments-rewrite", "1"), ("infra", ""))
q_left = (cl("WHAT IT NEEDS TO KNOW") + '<div class="say">“The charge stores two currencies: the one the customer paid in and the one you were paid in. Today a refund goes out in yours, so the customer gets less back when the rate has moved. Which should a retried refund use?”<br><i>Working on PAY-7 Refund retry with backoff</i></div>')
q_right = (cl("YOUR ANSWER") + '<div style="display:flex;flex-direction:column;gap:6px;align-items:flex-start">'
           '<span class="btn">The currency the customer paid in</span><span class="btn">The currency I was paid in</span><span class="btn">Leave it as it is and add a to-do</span>'
           '<span class="field" style="width:100%;margin-top:4px"><span>Or type an answer…</span><span>↵</span></span></div>')
QUESTION = card("Which currency should a retried refund use?", "copilot in Refund retries · payments-rewrite · 6m ago", q_left, q_right,
                "copilot is stopped until you answer", "", first=False, buttons='<span style="color:var(--muted)">A question changes nothing by itself; the answer goes to the agent as you give it.</span>')
page("b3-1-copy-ask-why", inbox_top(1, chans=ch) + '<div class="ibody">'
     + C(FILE_COPY, extra=asked, buttons=YN + '<span class="btn" style="margin-left:8px;color:var(--muted)">Asked why ✓</span>') + '</div>', sel="Agent inbox", inbox="1")
page("b3-2-question", inbox_top(1, chans=(("All", "1"), ("payments-rewrite", "1"), ("infra", ""))) + f'<div class="ibody">{QUESTION.replace("card", "card first", 1)}</div>', sel="Agent inbox", inbox="1", dot_ledger="g")

term = '''<span class="m">~/src/api (refund-retries) $</span> copilot
> Make refunds retry with backoff (PAY-7).

● Read(api/src/refunds/retry.rs)
  <span class="m">⎿ Read 148 lines</span>
● Edit(api/src/refunds/retry.rs)
  <span class="m">⎿ Added backoff with jitter, 5 attempts</span>
● Bash(cargo test -p api refunds)
  <span class="gn">⎿ test result: ok. 31 passed; 0 failed</span>

● The retry path picks a currency for the refund.
  The charge stores two, and they differ when
  the rate has moved. I need you to choose.

<span class="am">◐ Asked you in the Agent inbox · waiting 6m</span>
<span class="m">  You can also answer here.</span>'''
panel = ('<div class="ph"><b>Agent inbox</b><span style="color:var(--muted)">this session · 1 waiting on you</span><span class="sp"></span><span style="color:var(--muted)">⤢ &nbsp;✕</span></div>'
         '<div style="padding:14px"><div class="card first" style="padding:14px 15px"><div class="hd"><i class="dot o"></i><h3 style="font-size:14.5px">Which currency should a retried refund use?</h3></div>'
         + '<div style="margin-top:10px">' + q_left + '<div style="height:10px"></div>' + q_right + '</div>'
         '<div class="cfoot" style="color:var(--muted);font-size:12.5px">copilot is stopped until you answer · asked 6m ago</div></div>'
         '<div style="color:var(--muted);font-size:12.5px;margin-top:14px;line-height:1.5">Only this session and its forks are listed here. Everything waiting on you is in <span class="lnk">Agent inbox</span>.</div></div>')
page("b3-3-session-inbox-panel", session("Refund retries", "payments-rewrite", ["copilot", "zsh"], term, panel, "Agent inbox", agent="Copilot · devbox-1", dot="o", badge=("Agent inbox", "1")),
     sel="Refund retries", inbox="1", raw=True, dot_ledger="g")
