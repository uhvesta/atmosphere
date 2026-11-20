#!/usr/bin/env python3
"""TODO2WIRE frames: a step's context edited in the drawer, and Settings > General > To-do.
Reuses the look of ../../boardsimple/src/lib.py; writes its HTML beside this file."""
import os, sys
MINE = os.path.dirname(os.path.abspath(__file__))
sys.dont_write_bytecode = True
sys.path.insert(0, os.path.join(MINE, "..", "..", "boardsimple", "src"))
import lib
from lib import *
lib.HERE = MINE

lib.CSS += """
.step2{padding:3px 0}
.step2 .r{display:flex;gap:9px;align-items:center}
.step2 .r .lk{color:var(--blue);margin-left:8px;white-space:nowrap}
.step2.d .tx{color:var(--muted)}
.step2 .r.hov{background:#f6f6f8;box-shadow:-6px 0 0 #f6f6f8,6px 0 0 #f6f6f8;border-radius:4px}
.ctx{padding-left:24px;font-size:11.5px;color:var(--muted);margin-top:2px;line-height:1.4}
.ctx.hov{color:var(--soft)}
.ced{margin:4px 0 4px 24px}
.ced .ta{border:1px solid var(--blue);box-shadow:0 0 0 2px #cfe0ff;border-radius:6px;background:#fff;padding:6px 9px;font-size:12px;line-height:1.5;min-height:66px;white-space:pre-wrap}
.ced .ta .ph{color:#a0a0a6}
.ced .ta .car{display:inline-block;width:1px;height:14px;background:#1c1c1e;vertical-align:-2px}
.ced .b{display:flex;align-items:center;gap:10px;margin-top:6px}
.ced .b .mini{padding:3px 11px}
.ced .b .hn{margin-left:auto;color:var(--muted);font-size:11.5px}
.cur{position:absolute;width:14px;height:20px;z-index:30}
.callout{position:absolute;background:#2a2a2e;color:#fff;border-radius:6px;padding:3px 8px;font-size:11.5px;z-index:30;white-space:nowrap}
/* settings, as the app draws it: section list, then cards of rows */
.snav{width:200px;flex:none;padding:10px;border-right:1px solid var(--line);display:flex;flex-direction:column;gap:2px}
.snav div{padding:6px 10px;border-radius:7px;display:flex;gap:9px;align-items:center}
.snav div.sel{background:var(--blue);color:#fff;font-weight:600}
.snav i{width:14px;height:13px;border:1.4px solid var(--blue);border-radius:3px;display:block;flex:none}
.snav .sel i{border-color:#fff}
.sbody{flex:1;min-width:0;padding:16px 20px;display:flex;flex-direction:column;gap:14px;overflow:hidden}
.scard{border:1px solid var(--line);border-radius:9px;background:#fff}
.set2{display:flex;align-items:center;gap:16px;padding:10px 14px;border-bottom:1px solid var(--line)}
.scard .set2:last-child{border-bottom:0}
.set2 .n{flex:1;min-width:0}
.set2 .n small,.dim{display:block;font-size:11.5px;color:var(--muted)}
.dim b{font-weight:600}
.seg2{display:inline-flex;background:var(--sunken);border-radius:7px;padding:2px;flex:none}
.seg2 i{font-style:normal;padding:2px 12px;border-radius:5px;font-size:11.5px}
.seg2 i.on{background:#fff;box-shadow:0 1px 2px rgba(0,0,0,.14)}
.tog{width:28px;height:16px;border-radius:8px;padding:2px;background:var(--sunken);display:flex;flex:none}
.tog b{width:12px;height:12px;border-radius:50%;background:#fff;display:block}
.tog.on{background:#28cd41;justify-content:flex-end}
.popb{height:24px;padding:0 5px 0 9px;border-radius:6px;background:var(--sunken);border:1px solid var(--line);font-size:12px;display:inline-flex;gap:6px;align-items:center;flex:none}
.popb span{color:var(--muted);font-size:10px}
"""

CURSOR = '<svg class="cur" style="left:{x}px;top:{y}px" viewBox="0 0 14 20"><path d="M1 1v15l4-3.6 2.6 6 2.2-1-2.6-5.8H13z" fill="#111" stroke="#fff" stroke-width="1"/></svg>'

def step(i, n, text, done=False, ctx=None, hover=False, add=False, editor=None, ctx_hover=False, anchor=None):
    links = ""
    if add: links += '<span class="lk">+ Context</span>'
    if i > 0: links += '<span class="lk">↑</span>'
    if i + 1 < n: links += '<span class="lk">↓</span>'
    links += '<span class="lk">Drop</span>'
    cb = '<span class="cb d">✓</span>' if done else '<span class="cb"></span>'
    s = (f'<div class="step2{" d" if done else ""}"><div class="r{" hov" if hover else ""}">{cb}<span class="tx">{text}</span>'
         f'<span class="sp"></span>{links}</div>')
    if editor is not None:
        s += editor
    elif ctx:
        s += f'<div class="ctx{" hov" if ctx_hover else ""}">{ctx}</div>'
    if anchor:
        s += f'<div class="ctx" style="color:var(--blue)">{anchor}</div>'
    return s + '</div>'

def editor(text, hint="⌘↩ saves · Esc cancels"):
    body = (e(text) + '<span class="car"></span>') if text else '<span class="car"></span><span class="ph">Context for this step. Markdown is fine.</span>'
    return (f'<div class="ced"><div class="ta">{body}</div>'
            f'<div class="b"><span class="mini pri">Save</span><span class="lnk">Cancel</span><span class="hn">{hint}</span></div></div>')

CTX3 = "Batches of 50k rows, ordered by id. Resume from `backfill_cursor`; see docs/ledger-backfill.md."
TEXTS = ["Add ledger_entries and ledger_balances", "Dual-write from the posting path", "Backfill the old table in batches",
         "Verify the balances match on staging", "Cut production reads over"]

def body(steps_html, by="ticked by claude in Ledger entries"):
    return (fr("State", seg(["Open", "In progress", "Closed"], "In progress")) + fr("Channel", "<span># payments-rewrite</span>")
            + fr("Sessions", '<span class="ses"><i class="dot o"></i>Ledger entries</span>' + ADDL)
            + fr("Due", '<span class="inp">Fri 20 Nov</span><span class="hint">tomorrow · or type “next week”</span>')
            + fr("Effort", seg("SML", "M") + '<span style="width:96px;color:var(--muted);margin-left:34px">Impact</span>' + seg(["Low", "Medium", "High"], "High"))
            + fr("Tags", '<span class="tag">migration</span>&nbsp;<span class="tag">db</span>' + ADDL)
            + f'<div class="h3"><span>STEPS · 4 OF 5</span><span style="font-weight:400">{by}</span></div>' + steps_html
            + '<div class="step" style="color:var(--muted)"><span class="cb" style="border-style:dashed"></span><span>Add a step…</span></div>'
            + '<div class="h3">DESCRIPTION</div><div class="desc">Split postings into ledger_entries and ledger_balances, backfill from the old table in batches, and keep both in step until cut-over.</div>')

def frame(name, steps_html, over=""):
    foot = FOOT.replace("Start a session on this", "Start another session on this")
    todo_page(name, "10 open across 4 channels", todo_list(True, sel="PAY-3", na=False),
              drawer("PAY-3 · made by you, Mon 9 Nov", "Ledger schema v2", body(steps_html), foot), over=over, narrow=True)

n = len(TEXTS)
# t2-0: at rest, with the pointer over the last step
rows = "".join(step(i, n, t, done=i < 4, ctx=CTX3 if i == 2 else None, hover=i == 4, add=i == 4) for i, t in enumerate(TEXTS))
frame("t2-0-step-hover", rows, CURSOR.format(x=1222, y=536))

# t2-1: the editor open on the last step
rows = "".join(step(i, n, t, done=i < 4, ctx=CTX3 if i == 2 else None,
                    editor=editor("Flip `ledger.reads_v2` to 100% after the 09:00 balance check.\nRoll back with the same flag; the old table stays in step until Friday.") if i == 4 else None)
               for i, t in enumerate(TEXTS))
frame("t2-1-step-context-editing", rows)

# t2-2: editing a context that is already there (clicked the grey line)
rows = "".join(step(i, n, t, done=i < 4, editor=editor(CTX3, "⌘↩ saves · Esc cancels · empty removes it") if i == 2 else None) for i, t in enumerate(TEXTS))
frame("t2-2-step-context-existing", rows)

# t7-1: Settings > General, the To-do group
def setting(title, note, control): return f'<div class="set2"><div class="n">{title}<small>{note}</small></div>{control}</div>'
def seg2(opts, on): return '<span class="seg2">' + "".join(f'<i class="{"on" if o == on else ""}">{o}</i>' for o in opts) + '</span>'
def tog(on): return f'<span class="tog{" on" if on else ""}"><b></b></span>'
nav = "".join(f'<div class="{"sel" if s == "General" else ""}"><i></i>{s}</div>' for s in
              ("General", "Agents", "Machines", "Layout", "Agent access", "Storage", "Notifications", "Keyboard"))
general = (f'<div class="scard">{setting("Confirm bulk approvals", "Ask before approving requests that cannot be undone.", tog(True))}</div>'
           f'<div class="scard">{setting("Free hidden terminals", "A terminal you have not looked at for 30 seconds gives its screen memory back and shows again when you open it. Turn off to keep every screen in memory. Terminals with unsent or unconfirmed typing are never freed.", tog(True))}</div>'
           f'<div class="scard">{setting("Fast mode", "Use less CPU and draw less often. Terminal cursors stop blinking, so a terminal you are not typing in draws nothing.", tog(False))}</div>')
todo = ('<div style="display:flex;flex-direction:column;gap:8px"><span class="dim"><b>To-do</b></span>'
        '<span class="dim">What Humata does by itself when pull requests and items change. Ask puts a one-click question on the item in To-do and never stops an agent.</span>'
        '<div class="scard">'
        + setting("When a pull request merges", "Ask: its item asks whether to close. Auto: Humata closes the item and shows Undo.", seg2(["Ask", "Auto"], "Ask"))
        + setting("When an item closes", "Ask: the item offers to archive its sessions. Auto: Humata archives them and shows Undo. A session with unsaved work or a running terminal is kept either way.", seg2(["Ask", "Auto"], "Auto"))
        + setting("Wake the agent when checks fail", "Sends the failing checks to the item’s session as a note when its turn ends. Off: the item shows Needs attention and you send it.", tog(True))
        + setting("Wake the agent on review comments", "Sends new review comments to the item’s session the same way. Off: the item offers Send comments to session.", tog(False))
        + setting("Nudge about idle items", "An In progress item with no activity for this long asks whether to carry on, take it back or close it.", '<span class="popb">3 days<span>⌃⌄</span></span>')
        + '</div></div>')
repos = '<span class="dim"><b>Repositories</b></span>'
page("t7-1-settings-todo", '<div class="top"><h1>Settings</h1></div>'
     f'<div class="body">  <div class="snav">{nav}</div><div class="sbody">{general}{todo}{repos}</div></div>', sel="Settings")
