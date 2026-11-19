from cards import *

empty = ('<div style="flex:1;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:8px;padding-bottom:80px">'
         '<div style="width:46px;height:46px;border-radius:50%;background:var(--teal-soft);color:var(--teal);font-size:22px;display:flex;align-items:center;justify-content:center">✓</div>'
         '<div style="font-size:18px;font-weight:700;margin-top:6px">Nothing is waiting on you</div>'
         '<div style="color:var(--soft);max-width:520px;text-align:center;line-height:1.5">5 agents are working. An agent stops and asks here before it moves or copies files or sessions, needs new access, or has a question for you.</div>'
         '<div style="color:var(--muted);margin-top:10px">Today: 2 answered · 31 actions followed your rules without asking · <span class="lnk">History</span></div>'
         '<div style="color:var(--muted)">3 to-dos need attention when you get to them · <span class="lnk">Open To-do</span></div></div>')
page("b2-1-inbox-empty", inbox_top(0) + empty, sel="Agent inbox", inbox="", dot_ledger="g")
ch = (("All", "1"), ("payments-rewrite", "1"), ("infra", ""))
page("b2-2-file-move", inbox_top(1, chans=ch) + f'<div class="ibody">{C(FILE_MOVE)}</div>', sel="Agent inbox", inbox="1")
ch = (("All", "1"), ("payments-rewrite", ""), ("infra", "1"))
page("b2-3-session-move", inbox_top(1, chans=ch) + f'<div class="ibody">{C(SESSION_MOVE)}</div>', sel="Agent inbox", inbox="1", dot_ledger="g")
