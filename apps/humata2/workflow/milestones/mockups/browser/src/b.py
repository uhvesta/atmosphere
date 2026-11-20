#!/usr/bin/env python3
"""The BROWSER frames: Settings > Browser and the two browser request cards."""
from lib import *

P["globe"] = '<circle cx="12" cy="12" r="10"/><path d="M12 2a14.5 14.5 0 0 0 0 20 14.5 14.5 0 0 0 0-20"/><path d="M2 12h20"/>'
MONO = 'font-family:Menlo,monospace'
AMBER_L, AMBER_D = "#b25000", "#ffb340"
TEAL_L, TEAL_D = "#0f766e", "#35c4b5"

# ---------------------------------------------------------------- b1: Settings > Browser
def settings():
    nav = lambda g, t, on=False: f'<div style="display:flex;align-items:center;gap:8px;height:28px;padding:0 10px;border-radius:6px;{"background:#0a6cff;color:#fff" if on else ""}">{ic(g, 15, "#fff" if on else "var(--accent)")}{t}</div>'
    h = '<div style="display:flex;flex:1;font-size:12.5px"><div style="width:200px;padding:10px;display:flex;flex-direction:column;gap:1px;border-right:1px solid var(--line)">'
    h += (nav("gear", "General") + nav("bot", "Agents") + nav("server", "Machines") + nav("cols", "Layout") + nav("shield", "Agent access")
          + nav("globe", "Browser", True) + nav("drive", "Storage") + nav("bell", "Notifications") + nav("keyboard", "Keyboard"))
    h += '</div><div style="flex:1;padding:20px;display:flex;flex-direction:column;gap:8px">'
    # heading
    h += ('<span class="lbl">Browser</span>'
          '<div class="sec" style="margin-top:-4px">Claude Code, Codex, Copilot and agy can read tabs in your own browsers and, when you say yes, click, type and open pages in them. No other agent can.</div>')
    # the table
    h += f'<div style="display:flex;align-items:center;margin-top:10px"><span class="lbl sp">Browsers on this Mac</span>{btn("Install all")}</div><div class="card">'
    h += ('<div class="set" style="padding:6px 14px;gap:0"><span class="lbl" style="width:120px">Browser</span>'
          '<span class="lbl" style="width:370px">Host</span><span class="lbl sp">Extension</span></div>')
    def brow(name, host, ext, buttons):
        return (f'<div class="set" style="gap:0;min-height:40px;padding:8px 14px"><span style="width:120px;flex:none">{name}</span>'
                f'<div style="width:370px;flex:none;padding-right:16px">{host}</div><div class="sp" style="min-width:0">{ext}</div>'
                f'<div style="display:flex;gap:6px;flex:none">{buttons}</div></div>')
    ok = lambda t: f'<div style="display:flex;align-items:center;gap:6px">{ic("check", 12, GREEN, 2.6)}<span>{t}</span></div>'
    live = lambda s, t: f'<div style="display:flex;align-items:center;gap:6px">{st(s)}<span>{t}</span></div>'
    sub = lambda t: f'<div class="sec" style="margin-top:1px">{t}</div>'
    h += brow("Chrome", ok("Set up"), live("g", "Connected"), btn("Uninstall"))
    h += brow("Brave", ok("Set up"), live("off", "Not connected") + sub("Load the extension in Brave, below."), btn("Uninstall"))
    h += brow("Edge", f'<div style="display:flex;align-items:center;gap:6px;color:{ORANGE}"><b style="width:12px;text-align:center">!</b><span>Out of date</span></div>'
              + sub("Its file points at an older Humata, so Edge can’t connect."), "", btn("Update host", "pri") + btn("Uninstall"))
    h += brow("Firefox", '<span class="sec" style="font-size:12.5px">Not set up</span>', "", btn("Set up host", "pri"))
    h += '</div>'
    h += '<div class="sec" style="margin-top:2px">Set up host writes one file into that browser’s own folder so it can find Humata. Uninstall removes that file and nothing else.</div>'
    # the extension
    h += '<span class="lbl" style="margin-top:12px">Load the extension</span><div class="card">'
    def ext(title, how, path):
        return (f'<div class="set" style="align-items:flex-start"><div class="sp" style="min-width:0"><div>{title}</div>'
                f'<div class="sec" style="margin-top:1px">{how}</div>'
                f'<div style="{MONO};font-size:11.5px;margin-top:4px">{path}</div></div>'
                f'<div style="display:flex;gap:14px;flex:none;padding-top:1px"><span class="lnk">Copy path</span><span class="lnk">Reveal in Finder</span></div></div>')
    h += ext("Chrome, Brave and Edge", "Open the browser’s Extensions page, turn on Developer mode, choose Load unpacked and pick this folder.",
             "apps/humata2/browser_extension/chromium")
    h += ext("Firefox", "Open about:debugging, choose This Firefox, then Load Temporary Add-on, and pick manifest.json in this folder. Firefox forgets it when it quits, so load it again after a restart.",
             "apps/humata2/browser_extension/firefox")
    h += '</div>'
    # asking
    h += '<span class="lbl" style="margin-top:12px">What you are asked</span><div class="card">'
    h += '<div class="set"><span class="sp">Read a tab or list your tabs</span><span class="sec" style="font-size:12.5px">Asked in the Agent inbox · you can allow reading for 30 min</span></div>'
    h += '<div class="set"><span class="sp">Click, type or open a page</span><span class="sec" style="font-size:12.5px">Asked every time · can’t be allowed ahead</span></div>'
    h += '</div>'
    return h + '</div></div>'
page("b1-settings-browser", sidebar("Settings"), toolbar("Settings") + settings(), look=DARK)

# ---------------------------------------------------------------- the Agent inbox page and its card
INBOX_CSS = """<style>
.chip{height:26px;padding:0 9px;border-radius:6px;background:var(--fill);display:flex;align-items:center;gap:5px;font-size:12.5px;flex:none}
.chip .c{color:var(--secondary)}
.chip.on{background:var(--chipon);color:var(--chipontext);font-weight:500}.chip.on .c{color:var(--chipontext);opacity:.7}
.rq{border-radius:12px;background:var(--card);border:1px solid var(--waiting);padding:16px 18px 14px;font-size:13px;display:flex;flex-direction:column}
.rq .top{display:flex;align-items:center;gap:10px}
.rq .ttl{font-size:15.5px;font-weight:700;flex:1;min-width:240px}
.rq .who{font-size:12.5px;color:var(--secondary);white-space:nowrap}
.rq .open{font-size:12.5px;color:var(--accent);white-space:nowrap}
.rq .cols{display:flex;gap:28px;align-items:flex-start;margin-top:10px}
.rq .cols>div{flex:1;min-width:0;display:flex;flex-direction:column}
.facts{display:flex;flex-direction:column;gap:3px;padding:9px 12px;border-radius:8px;background:var(--fill)}
.facts>div{display:flex}
.facts .k{width:66px;flex:none;color:var(--secondary)}
.facts .v{flex:1;min-width:0}
.sct{display:flex;align-items:baseline;gap:4px;margin:10px 0 4px;font-size:11px;font-weight:600;color:var(--secondary)}
.sct.first{margin-top:0}
.sct small{font-size:11.5px;font-weight:400}
.eff{display:flex;gap:8px;padding:1.5px 0;align-items:flex-start}
.eff i{width:14px;flex:none;text-align:center;font-style:normal;font-weight:600}
.eff.w i{color:var(--warn)}.eff.k i{color:var(--good)}
.rq .ft{display:flex;align-items:center;gap:8px;margin-top:14px;padding-top:12px;border-top:1px solid var(--line)}
.ab{height:32px;padding:0 14px;min-width:64px;border-radius:7px;font-size:13px;font-weight:500;display:inline-flex;align-items:center;justify-content:center;background:var(--card);border:1px solid var(--line);flex:none;white-space:nowrap}
.ab.pri{background:var(--accent);border-color:var(--accent);color:#fff}
.q{color:var(--secondary)}
.url{font-family:Menlo,monospace;font-size:12px;padding:1px 6px;border-radius:5px;background:var(--attention);border:1px solid var(--waiting);display:inline-block}
.tgt{display:inline-flex;align-items:center;height:22px;padding:0 9px;border-radius:6px;border:1px solid var(--tertiary);background:var(--card);font-weight:600;font-size:12.5px}
</style>"""

def inbox(waiting, chips, cards, bulk=""):
    h = INBOX_CSS + f'<div class="tb"><h1>Agent inbox</h1><span class="sec" style="font-size:12.5px">{waiting} waiting on you</span><span class="sp"></span>{bulk}<span class="lnk" style="font-size:12.5px;margin-left:6px">History</span></div>'
    h += '<div style="display:flex;align-items:center;gap:6px;padding:12px 28px 0">'
    for i, (name, n) in enumerate(chips):
        h += f'<span class="chip{" on" if i == 0 else ""}">{name} <span class="c">{n}</span></span>'
    h += '<span class="sp"></span><span class="sec" style="font-size:12px">Agents that are stopped until you answer. Changes to your to-dos wait in To-do instead.</span></div>'
    h += f'<div style="display:flex;flex-direction:column;gap:14px;padding:16px 28px">{"".join(cards)}</div>'
    return h

def card(title, who, facts, why, working, effects, undo, buttons, note):
    h = f'<div class="rq"><div class="top">{st("o")}<span class="ttl">{title}</span><span class="who">{who}</span><span class="open">Open session ↗</span></div>'
    h += '<div class="cols"><div><div class="facts">' + "".join(f'<div><span class="k">{k}</span><span class="v">{v}</span></div>' for k, v in facts) + '</div>'
    h += f'<div class="sct">WHY <small>the agent’s reason</small></div><div style="line-height:1.5">“{why}”</div><div class="q" style="margin-top:2px">Working on {working}</div></div>'
    h += '<div><div class="sct first">WHAT ELSE IT AFFECTS <small>worked out by Humata</small></div>'
    for warn, lead, rest in effects:
        h += f'<div class="eff {"w" if warn else "k"}"><i>{"!" if warn else "✓"}</i><span><b>{lead}</b>{rest}</span></div>'
    h += f'<div class="sct">CAN IT BE UNDONE?</div><div>{undo}</div></div></div>'
    h += '<div class="ft">' + "".join(f'<span class="ab{" pri" if i == 0 else ""}">{b}</span>' for i, b in enumerate(buttons))
    h += f'<span class="sp"></span><div style="display:flex;flex-direction:column;align-items:flex-end;font-size:12.5px" class="q">{note}</div></div></div>'
    return h

def look(base, dark):
    d = dict(base)
    d.update(waiting="#b9812f" if dark else "#f0b35a", warn=AMBER_D if dark else AMBER_L, good=TEAL_D if dark else TEAL_L,
             chipon="#e8e8ea" if dark else "#1d1d1f", chipontext="#1d1d1f" if dark else "#ffffff")
    return d

# ---------------------------------------------------------------- b2: read
read = card(
    "Let Claude Code on devbox-1 read a tab in your browser?",
    "Claude Code in Ledger schema migration · payments-rewrite · 2m ago",
    [("Tab", "<b>Retry refunds with backoff #482</b>"),
     ("URL", f'<span style="{MONO};font-size:12px">https://code.example.test/acme/api/pull/482</span>'),
     ("Browser", "Chrome on This Mac"),
     ("Reads", "The text and links on the page, as it shows now"),
     ("From", "<b>devbox-1</b> · Claude Code in Ledger schema migration")],
    "I need the review comments on this pull request so I can answer them in the code.",
    "PAY-3 Ledger schema v2",
    [(False, "", "Nothing is clicked, typed or opened: this only reads"),
     (True, "The page is read as you see it, signed in:", " it can hold things only your account can open"),
     (True, "Allow 30 min is wider than this tab:", " this session can read any of your tabs, and list them, until 14:32. No other session can")],
    "No. Once the agent has the text, it keeps it. Allow 30 min ends by itself at 14:32, or sooner if Humata restarts.",
    ["Allow once", "Allow 30 min", "Deny"],
    "<span>Claude Code is stopped until you answer</span>")
page("b2-inbox-browser-read", sidebar("Agent inbox", inbox="1"),
     inbox(1, [("All", 1), ("payments-rewrite", 1)], [read]), look=look(LIGHT, False))

# ---------------------------------------------------------------- b3: write
write = card(
    "Let Claude Code on devbox-1 click “Merge pull request” in your browser?",
    "Claude Code in Ledger schema migration · payments-rewrite · 1m ago",
    [("Action", "<b>Click</b> · one click, nothing typed"),
     ("URL", '<span class="url">https://code.example.test/acme/api/pull/482</span>'),
     ("Target", '<div style="display:flex;align-items:center;gap:8px"><span class="tgt">Merge pull request</span><span class="q">button</span></div>'
                f'<div class="q" style="{MONO};font-size:11.5px;margin-top:3px">button.merge-confirm[data-action="merge"]</div>'),
     ("Browser", "Chrome on This Mac · tab “Retry refunds with backoff #482”"),
     ("From", "<b>devbox-1</b> · Claude Code in Ledger schema migration")],
    "All checks pass and you approved this pull request an hour ago. Merging lets me rebase the migration branch on main.",
    "PAY-3 Ledger schema v2",
    [(True, "The click is made as you:", " the page sees your signed-in account, not the agent"),
     (True, "Humata can’t tell what the page does after the click.", " Check the URL and the target"),
     (False, "", "Only this one click: the next click, typed text or page is asked again"),
     (False, "", "If the target is no longer on the page, nothing is clicked")],
    "Not by Humata. What the click does is up to the page.",
    ["Allow once", "Deny"],
    "<span>Asked every time: clicks, typing and opening pages can’t be allowed ahead</span><span>Not part of Allow all</span>")
push = card(
    "Push try-brin to origin?",
    "Claude Code in Try BRIN index · payments-rewrite · 9m ago",
    [("Branch", "<b>try-brin</b> · 3 commits ahead of origin"), ("Remote", "origin · acme/ledger")],
    "The index experiment is ready for you to look at.", "PAY-3 Ledger schema v2",
    [(False, "", "A new branch on origin; main is not touched")], "Yes. Delete the branch on origin.",
    ["Yes", "No", "Ask why"], "")
bulk = '<span class="btn">Allow all 1</span><span class="btn">Deny all</span>'
page("b3-inbox-browser-write", sidebar("Agent inbox", inbox="2"),
     inbox(2, [("All", 2), ("payments-rewrite", 2)], [write, push], bulk), look=look(DARK, True))
