from lib import *
from m1 import everything, SUMALL, SUBALL, TRASH, DESK

KEYS = '<span class="ter" style="font-size:11px">⌘A all · ⇧-click a range · space ticks · esc cancels</span>'
def pagebar(n, note, menu_open=False):
    pop = f'<span class="btn{" lit" if menu_open else ""}" style="padding:0 5px 0 9px;gap:6px">Select{ic("updown", 12)}</span>'
    return (f'<div class="sbar"><span style="font-weight:600">{n} selected</span><span class="sec">{note}</span>{pop}'
            f'<span class="sp"></span>{KEYS}{btn("Cancel")}{btn(f"Delete {n} sessions…", "dng")}</div>')

# m2-1: Home in select mode, a whole channel picked from the Select menu
PAY = {"Ledger schema migration", "Try BRIN index", "Partial BRIN on posted_at", "Refund retries", "API rate limiter"}
def item(text, n="", on=False, dim=False):
    mark = ic("check", 12, "var(--text)", 2.6) if on else '<span style="width:12px"></span>'
    return f'<div style="{"color:var(--tertiary)" if dim else ""}">{mark}<span style="flex:1">{text}</span><span class="sec" style="margin-left:18px">{n}</span></div>'
menu = ('<div class="menu" style="left:440px;bottom:42px;width:236px">'
        '<div class="hd">Every session in a channel</div>' + item("# payments-rewrite", "6", True) + item("# infra", "2") + item("# pr-reviews", "1")
        + '<hr><div class="hd">Every session on a machine</div>' + item("This Mac", "3") + item("devbox-1", "6") + item("gpu-box", "offline", dim=True)
        + '<hr>' + item("Everything on Home", "⌘A") + item("Nothing") + '</div>')
page("m2-1-home-select-channel", sidebar("Home"),
     toolbar("Home", btn("Done", "lit") + '<span style="opacity:.45">' + btn("New session", "pri") + '</span>') + home_body(PAY)
     + pagebar(6, "All of # payments-rewrite · Review the schema is not on Home", True) + menu)

# m2-2: the machine page in select mode, every session on it picked
ROWS = [("o", "Ledger schema migration", "# payments-rewrite", "Claude Code"), ("o", "Try BRIN index", "# payments-rewrite", "Claude Code"),
        ("i", "API rate limiter", "# payments-rewrite", "Codex"), ("g", "Partial BRIN on posted_at", "# payments-rewrite", "Claude Code"),
        ("g", "Benchmark BRIN", "# infra", "Codex"), ("x", "Upgrade Postgres 17", "# infra", "Claude Code")]
def machine(select):
    h = (f'<div style="padding:0 20px;flex:none;border-bottom:1px solid var(--line)"><div style="height:48px;display:flex;align-items:center;gap:9px">'
         f'<span style="font-size:15px;color:var(--secondary)">Hosts</span>{ic("right", 12, "var(--tertiary)")}{st("g")}<span style="font-size:15px;font-weight:600">devbox-1</span>'
         f'<span class="sp"></span>{btn("Open a shell")}{btn("New session on devbox-1", "pri")}{ic("dots")}</div>'
         f'<div class="sec" style="margin-top:-1px">Ubuntu 24.04 · x86_64 · devbox-1.lan (ssh) · humatad 2.0.3</div>'
         f'<div style="display:flex;gap:22px;margin-top:16px;font-size:12.5px"><span style="padding:0 2px 7px;border-bottom:2px solid var(--accent)">Overview</span><span style="padding:0 2px 7px;color:var(--secondary)">Storage</span></div></div>')
    h += '<div style="display:flex;gap:16px;padding:20px;font-size:12.5px"><div style="flex:1;min-width:0;display:flex;flex-direction:column;gap:16px">'
    if select:
        head = f'<div style="display:flex;align-items:center;gap:10px;padding-left:13px">{cb(1)}<span style="font-size:12px;font-weight:500">Select all on devbox-1</span><span class="sec">6 of 6</span><span class="sp"></span><span class="lnk" style="font-size:11.5px">Done</span></div>'
    else:
        head = '<div style="display:flex"><span class="lbl sp">Sessions on it · 6</span><span class="lnk" style="font-size:11.5px">Select</span></div>'
    h += f'<div style="display:flex;flex-direction:column;gap:6px">{head}<div class="card">'
    for s, t, ch, ag in ROWS:
        h += f'<div class="mrow{" tk" if select else ""}">{cb(1) if select else ""}{st(s)}<span style="width:{166 if select else 190}px" class="nw">{t}</span><span class="sec" style="width:130px">{ch}</span><span class="sec" style="font-size:12.5px">{ag}</span></div>'
    h += '</div></div><div style="display:flex;flex-direction:column;gap:6px"><span class="lbl">Humata folders by channel</span><div class="card">'
    for ch, what in (("# payments-rewrite", "4 sessions · 3.1 GB"), ("# infra", "2 sessions · 0.8 GB"), ("# pr-reviews", "no sessions · 0.2 GB")):
        h += f'<div class="mrow">{ic("folder", 12)}<span style="width:220px">{ch}</span><span class="sec sp">{what}</span><span class="lnk">Open in Files</span></div>'
    h += '</div></div></div>'
    chip = lambda t: f'<span style="padding:1px 6px;border-radius:4px;border:1px solid var(--line);font-size:11px;color:var(--secondary)">{t}</span>'
    kv = lambda k, v: f'<div style="display:flex;align-items:center;gap:8px;min-height:22px"><span style="width:84px;color:var(--secondary);flex:none">{k}</span>{v}</div>'
    h += ('<div style="width:400px;flex:none;display:flex;flex-direction:column;gap:14px"><div class="card" style="padding:14px 12px;gap:8px"><span style="font-weight:600">Health</span>'
          + kv("GitHub", ic("check", 12, GREEN) + "signed in as uhvesta " + chip("repo") + chip("read:org"))
          + kv("Agents", chip("claude 2.4") + chip("codex 0.98"))
          + kv("Disk", f'<div style="flex:1"><div style="height:4px;border-radius:2px;background:var(--fill)"><div style="width:97%;height:4px;border-radius:2px;background:{RED}"></div></div><div style="margin-top:5px">11 GB free of 500 GB</div><div class="or" style="font-size:11.5px;margin-top:3px">New clones stop at 5 GB.</div></div>')
          + kv("Variables", chip("DATABASE_URL") + chip("STRIPE_KEY"))
          + kv("Humata", "2.0.3") + '</div><span class="lbl">What its agents can reach through the host</span><div class="card" style="margin-top:-8px">')
    for t, v, col in (("Read files on other machines", "Allow", GREEN), ("Push its branch or open a pull request", "Ask me", ORANGE),
                      ("Read the global board or other channels", "Allow", GREEN), ("Change the global board or other channels", "Ask me", ORANGE)):
        h += f'<div class="mrow" style="height:32px"><span class="sp">{t}</span><span style="color:{col};width:52px">{v}</span></div>'
    h += '<div class="mrow" style="height:32px"><span class="lnk">Change in Agent access</span></div></div></div></div>'
    return h
page("m2-2-machine-select-all", sidebar("devbox-1"), machine(True) + pagebar(6, "Every session on devbox-1"))

# m2-3: the palette
home = toolbar("Home", btn("Select") + btn("New session", "pri")) + home_body()
pal = (f'<div class="scrim"></div><div class="over" style="top:110px"><div class="pal"><div class="q">{ic("search")}<span>delete</span><span class="caret" style="height:18px;margin-left:-8px"></span></div>'
       '<div class="g lbl">Commands</div>'
       f'<div class="res on">{ic("listchecks", 15, "#fff")}<span>Select sessions…</span><span class="nt">Tick sessions, channels or machines, then delete them</span></div>'
       f'<div class="res">{ic("trash", 15, RED)}<span style="color:{RED}">Delete all sessions…</span><span class="nt">23 sessions · shows the plan first</span></div>'
       '</div></div>')
page("m2-3-palette", sidebar("Home"), home, pal)

# m2-4: Settings > Storage
def settings():
    nav = lambda g, t, on=False: f'<div style="display:flex;align-items:center;gap:8px;height:28px;padding:0 10px;border-radius:6px;{"background:#0a6cff;color:#fff" if on else ""}">{ic(g, 15, "#fff" if on else "var(--accent)")}{t}</div>'
    h = '<div style="display:flex;flex:1;font-size:12.5px"><div style="width:200px;padding:10px;display:flex;flex-direction:column;gap:1px;border-right:1px solid var(--line)">'
    h += nav("gear", "General") + nav("bot", "Agents") + nav("server", "Machines") + nav("cols", "Layout") + nav("shield", "Agent access") + nav("drive", "Storage", True) + nav("bell", "Notifications") + nav("keyboard", "Keyboard")
    h += '</div><div style="flex:1;padding:20px;display:flex;flex-direction:column;gap:8px"><span class="lbl">Storage by machine</span><div class="card">'
    for s, n, what, link in (("g", "This Mac", "148 GB free of 1 TB · trash 0.4 GB", True), ("g", "devbox-1", "212 GB free of 500 GB · trash 3.1 GB", True), ("off", "gpu-box", "offline", False)):
        h += f'<div class="set" style="gap:10px">{st(s)}<span style="width:120px">{n}</span><span class="sec sp">{what}</span>' + ('<span class="lnk">Open Storage</span>' if link else "") + '</div>'
    h += '</div><span class="lbl" style="margin-top:14px">Sessions</span><div class="card">'
    h += ('<div class="set"><div class="sp"><div>Delete all sessions</div><div class="sec" style="margin-top:1px">Every live and archived session on every machine that can be reached: 23 now. '
          'You see the full plan first and type the count to confirm. Desk sessions are not part of this.</div></div>' + btn("Delete all sessions…", "dng") + '</div></div>')
    h += '<div class="sec" style="margin-top:2px">To delete only some, tick them in the sidebar, on Home or on a machine’s page: ⌘-click a session, or use Select.</div>'
    return h + '</div></div>'
page("m2-4-settings-storage", sidebar("Settings"), toolbar("Settings") + settings(), look=DARK)

# m2-5: delete all, the count typed, in dark mode, the list scrolled to its end
page("m2-5-plan-all-typed-dark", sidebar("Settings"), toolbar("Settings") + settings(), sheet(
    "Delete all 23 sessions?", SUBALL, SUMALL, everything(ticks=(True, True, True)), TRASH,
    "Return deletes", btn("Delete all 23 sessions", "red"),
    top=22, typed=("Type the number of sessions, 23, to confirm", "23"), list_style="height:296px", thumb=(172, 118), after_list=DESK, at_end=True), look=DARK)

# m2-6: after the delete of the 4 picked sessions
GONE = {"Ledger schema migration", "Try BRIN index", "Refund retries", "Upgrade Postgres 17"}
toast = f'<div class="over" style="bottom:24px;left:118px"><div class="toast">{ic("trash", 12, "#fff")}<span>Deleted 4 sessions. They are in the trash on This Mac and devbox-1 for 30 days.</span></div></div>'
page("m2-6-after-delete", sidebar("Home", gone=GONE, inbox="4"), home.replace(home_body(), home_body(gone=GONE)), toast)
