#!/usr/bin/env python3
"""Shared parts of the mass-delete mockups. Pages are laid out at 1280x760
(the size the app's design frames use) and shot at 1.25x, so a PNG is
1600x950."""
import os
HERE = os.path.dirname(os.path.abspath(__file__))

LIGHT = dict(sidebar="#e9e7ea", content="#ffffff", panel="#f6f5f7", line="#dcdadd", text="#1d1d1f",
             secondary="#6e6e73", tertiary="#a1a1a6", accent="#0a6cff", fill="#eceaee", card="#ffffff",
             attention="#fff4e0", scrim="rgba(0,0,0,.19)", toast="#2c2c2e", tint="rgba(10,108,255,.11)")
DARK = dict(sidebar="#2a2a2d", content="#1e1e1f", panel="#252527", line="#3a3a3d", text="#e8e8ea",
            secondary="#98989e", tertiary="#6a6a70", accent="#3b8cff", fill="#323235", card="#2a2a2c",
            attention="#3a2c12", scrim="rgba(0,0,0,.5)", toast="#3a3a3d", tint="rgba(59,140,255,.18)")

CSS = """
*{box-sizing:border-box;margin:0;padding:0}
html,body{width:1280px;height:760px;overflow:hidden}
body{font:13px/1.35 -apple-system,BlinkMacSystemFont,"SF Pro Text","Helvetica Neue",sans-serif;color:var(--text);background:var(--content);display:flex;-webkit-font-smoothing:antialiased;position:relative}
svg{flex:none;display:block}
.sp{flex:1}
.sec{color:var(--secondary);font-size:11.5px}
.ter{color:var(--tertiary)}
.or{color:#ff9f0a}
.lbl{font-size:11px;font-weight:600;color:var(--secondary)}
.nw{white-space:nowrap}
/* sidebar */
.side{width:236px;height:760px;background:var(--sidebar);flex:none;display:flex;flex-direction:column;border-right:1px solid var(--line)}
.shead{height:52px;display:flex;align-items:center;padding:0 14px 0 20px;gap:12px;flex:none}
.lights{display:flex;gap:8px}.lights i{width:12px;height:12px;border-radius:50%}
.srch{margin:0 10px 6px;height:26px;display:flex;align-items:center;gap:6px;padding:0 8px;border-radius:6px;background:var(--fill);color:var(--tertiary);flex:none}
.r{height:26px;margin:0 10px;padding:0 8px;display:flex;align-items:center;gap:7px;border-radius:6px;flex:none;position:relative}
.r.on{background:var(--accent-sel);color:#fff}
.r.tk{background:var(--tint)}
.r .t{flex:1;min-width:0;overflow:hidden;white-space:nowrap}
.r .n{font-size:11px;color:var(--secondary)}.r.on .n{color:#fff}
.shd{display:flex;align-items:center;padding:12px 18px 3px;flex:none}
.st{width:10px;height:10px;display:flex;align-items:center;justify-content:center;flex:none}
.st i{width:8px;height:8px;border-radius:50%}
.lead{display:flex;align-items:center;gap:5px}
/* checkboxes */
.cb{width:14px;height:14px;border-radius:4px;flex:none;display:inline-flex;align-items:center;justify-content:center;border:1px solid var(--tertiary);background:var(--card)}
.cb.y{background:var(--accent);border-color:var(--accent)}
.cb.auto{background:var(--accent);border-color:var(--accent);opacity:.5}
.cb.dis{opacity:.4;border-style:dashed;background:transparent}
.cb.s{width:13px;height:13px;border-radius:3.5px}
.r .cb{position:absolute;left:3px}
/* buttons */
.btn{display:inline-flex;align-items:center;gap:5px;height:24px;padding:0 11px;border-radius:6px;font-size:12px;font-weight:500;flex:none;white-space:nowrap;background:var(--fill);border:1px solid var(--line);color:var(--text)}
.btn.pri{background:var(--accent);border-color:var(--accent);color:#fff}
.btn.dng{border-color:#ff453a;color:#ff453a}
.btn.dng.off,.btn.red.off{border-color:var(--line);color:var(--tertiary);background:var(--fill)}
.btn.red{background:#e5352b;border-color:#e5352b;color:#fff}
.btn.lit{border-color:var(--accent);color:var(--accent)}
.kbd{padding:0 5px;border-radius:4px;background:var(--fill);font-size:11px;color:var(--secondary);flex:none}
.lnk{color:var(--accent)}
/* page */
.main{flex:1;min-width:0;display:flex;flex-direction:column;background:var(--content);position:relative;height:760px}
.tb{height:52px;display:flex;align-items:center;gap:10px;padding:0 20px;border-bottom:1px solid var(--line);flex:none}
.tb h1{font-size:15px;font-weight:600}
.card{display:flex;flex-direction:column;border-radius:9px;background:var(--card);border:1px solid var(--line);overflow:hidden}
.hrow{display:flex;align-items:center;gap:10px;padding:9px 12px;border-bottom:1px solid var(--line)}
.hrow:last-child,.mrow:last-child,.set:last-child{border-bottom:0}
.hrow.tk,.mrow.tk{background:var(--tint)}
.mrow{display:flex;align-items:center;gap:10px;padding:0 12px;height:36px;border-bottom:1px solid var(--line);font-size:12.5px}
.set{display:flex;align-items:center;gap:16px;padding:10px 14px;border-bottom:1px solid var(--line)}
/* select bar on a page */
.sbar{position:absolute;left:0;right:0;bottom:0;height:48px;border-top:1px solid var(--line);background:var(--panel);display:flex;align-items:center;gap:10px;padding:0 20px;font-size:12.5px}
.sbox{margin:0 10px 2px;padding:7px 10px 9px;border-radius:9px;background:var(--card);border:1px solid var(--line);box-shadow:0 2px 8px rgba(0,0,0,.08);display:flex;flex-direction:column;gap:2px;flex:none}
.menu{position:absolute;background:var(--card);border:1px solid var(--line);border-radius:8px;box-shadow:0 10px 30px rgba(0,0,0,.22);padding:4px;font-size:12.5px;display:flex;flex-direction:column}
.menu div{padding:4px 8px;border-radius:5px;white-space:nowrap;display:flex;gap:8px;align-items:center}
.menu .hd{font-size:11px;font-weight:600;color:var(--secondary);padding-top:6px}
.menu .sel{background:var(--accent);color:#fff}
.menu hr{border:0;border-top:1px solid var(--line);margin:4px 0}
/* overlays */
.scrim{position:absolute;inset:0;background:var(--scrim)}
.over{position:absolute;left:0;width:100%;display:flex;justify-content:center}
.dlg{width:700px;display:flex;flex-direction:column;gap:6px;padding:20px;border-radius:12px;background:var(--card);border:1px solid var(--line);box-shadow:0 20px 50px rgba(0,0,0,.28);font-size:12.5px}
.dlg h2{font-size:15px;font-weight:600}
.sum{display:flex;flex-direction:column;gap:4px;margin:4px 0 2px}
.sum div{display:flex;gap:7px;align-items:center}
.list{display:flex;flex-direction:column;border-top:1px solid var(--line);border-bottom:1px solid var(--line);margin-top:4px;padding:2px 0 8px;position:relative}
.list.scroll{overflow:hidden;padding-right:14px;justify-content:flex-start}.list.end{justify-content:flex-end}
.thumb{position:absolute;right:1px;width:5px;border-radius:3px;background:var(--tertiary);opacity:.7}
.gh{padding:9px 0 3px;display:flex;gap:8px;align-items:baseline}
.srow{display:flex;flex-direction:column;gap:2px;padding:4px 0}
.srow .l1{display:flex;align-items:center;gap:7px;white-space:nowrap}
.srow .who{width:170px;flex:none}.srow .state{width:56px;flex:none;text-align:right}
.note{padding-left:19px;font-size:11.5px;display:flex;gap:6px;align-items:center}
.note.o{color:#ff9f0a}.note.q{color:var(--secondary)}
.tick{padding-left:19px;display:flex;gap:7px;align-items:center;padding-top:1px;padding-bottom:1px}
.sub{padding-left:19px}
.chg{font-size:10.5px;font-weight:600;color:#ff9f0a;border:1px solid #ff9f0a;border-radius:4px;padding:0 5px;line-height:15px}
.box{margin-top:6px;padding:10px;border-radius:7px;background:var(--fill);color:var(--secondary);line-height:1.45}
.stale{padding:10px 12px;border-radius:7px;background:var(--attention);color:#ff9f0a;display:flex;flex-direction:column;gap:3px;margin:2px 0 2px}
.stale b{font-weight:600}
.foot{display:flex;align-items:center;gap:8px;padding-top:8px}
.field{display:flex;align-items:center;height:26px;padding:0 8px;border-radius:6px;background:var(--card);border:1px solid var(--line);font-size:12.5px}
.field.focus{border-color:var(--accent);box-shadow:0 0 0 2.5px color-mix(in srgb,var(--accent) 30%,transparent)}
.caret{width:1.5px;height:15px;background:var(--accent);margin-left:1px}
.pal{width:600px;display:flex;flex-direction:column;padding-bottom:6px;border-radius:12px;background:var(--card);border:1px solid var(--line);box-shadow:0 20px 50px rgba(0,0,0,.28);font-size:13px}
.pal .q{display:flex;align-items:center;gap:10px;height:48px;padding:0 16px;border-bottom:1px solid var(--line);font-size:16px}
.pal .g{padding:8px 16px 3px}
.pal .res{display:flex;align-items:center;gap:9px;height:32px;margin:0 6px;padding:0 10px;border-radius:6px}
.pal .res.on{background:var(--accent-sel);color:#fff}
.pal .res .nt{flex:1;font-size:11.5px;color:var(--secondary)}.pal .res.on .nt{color:#fff}
.toast{display:flex;align-items:center;gap:14px;padding:9px 14px;border-radius:10px;background:var(--toast);color:#fff;font-size:12.5px;box-shadow:0 10px 30px rgba(0,0,0,.3)}
"""

P = {
 "house": '<path d="M15 21v-8a1 1 0 0 0-1-1h-4a1 1 0 0 0-1 1v8"/><path d="M3 10a2 2 0 0 1 .709-1.528l7-5.999a2 2 0 0 1 2.582 0l7 5.999A2 2 0 0 1 21 10v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>',
 "inbox": '<polyline points="22 12 16 12 14 15 10 15 8 12 2 12"/><path d="M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z"/>',
 "kanban": '<rect width="18" height="18" x="3" y="3" rx="2"/><path d="M8 7v7"/><path d="M12 7v4"/><path d="M16 7v9"/>',
 "term": '<path d="m7 11 2-2-2-2"/><path d="M11 13h4"/><rect width="18" height="18" x="3" y="3" rx="2"/>',
 "gear": '<path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"/><circle cx="12" cy="12" r="3"/>',
 "search": '<circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/>',
 "panel": '<rect width="18" height="18" x="3" y="3" rx="2"/><path d="M9 3v18"/>',
 "pen": '<path d="M12 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/><path d="M18.375 2.625a2.121 2.121 0 1 1 3 3L12 15l-4 1 1-4Z"/>',
 "fork": '<circle cx="12" cy="18" r="3"/><circle cx="6" cy="6" r="3"/><circle cx="18" cy="6" r="3"/><path d="M18 9v2c0 .6-.4 1-1 1H7c-.6 0-1-.4-1-1V9"/><path d="M12 12v3"/>',
 "branch": '<line x1="6" x2="6" y1="3" y2="15"/><circle cx="18" cy="6" r="3"/><circle cx="6" cy="18" r="3"/><path d="M18 9a9 9 0 0 1-9 9"/>',
 "down": '<path d="m6 9 6 6 6-6"/>', "right": '<path d="m9 18 6-6-6-6"/>',
 "updown": '<path d="m7 15 5 5 5-5"/><path d="m7 9 5-5 5 5"/>',
 "plus": '<path d="M5 12h14"/><path d="M12 5v14"/>', "minus": '<path d="M5 12h14"/>',
 "check": '<path d="M20 6 9 17l-5-5"/>', "x": '<path d="M18 6 6 18"/><path d="m6 6 12 12"/>',
 "trash": '<path d="M3 6h18"/><path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6"/><path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2"/>',
 "bot": '<path d="M12 8V4H8"/><rect width="16" height="12" x="4" y="8" rx="2"/><path d="M2 14h2"/><path d="M20 14h2"/><path d="M15 13v2"/><path d="M9 13v2"/>',
 "server": '<rect width="20" height="8" x="2" y="2" rx="2"/><rect width="20" height="8" x="2" y="14" rx="2"/><path d="M6 6h.01"/><path d="M6 18h.01"/>',
 "shield": '<path d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z"/><path d="m9 12 2 2 4-4"/>',
 "drive": '<line x1="22" x2="2" y1="12" y2="12"/><path d="M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z"/><path d="M6 16h.01"/><path d="M10 16h.01"/>',
 "bell": '<path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9"/><path d="M10.3 21a1.94 1.94 0 0 0 3.4 0"/>',
 "keyboard": '<rect width="20" height="16" x="2" y="4" rx="2"/><path d="M6 8h.01M10 8h.01M14 8h.01M18 8h.01M8 12h.01M12 12h.01M16 12h.01M7 16h10"/>',
 "cols": '<rect width="18" height="18" x="3" y="3" rx="2"/><path d="M12 3v18"/>',
 "folder": '<path d="m6 14 1.5-2.9A2 2 0 0 1 9.24 10H20a2 2 0 0 1 1.94 2.5l-1.54 6a2 2 0 0 1-1.95 1.5H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.69.9l.81 1.2a2 2 0 0 0 1.67.9H18a2 2 0 0 1 2 2v2"/>',
 "dots": '<circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/><circle cx="5" cy="12" r="1"/>',
 "listchecks": '<path d="m3 17 2 2 4-4"/><path d="m3 7 2 2 4-4"/><path d="M13 6h8"/><path d="M13 12h8"/><path d="M13 18h8"/>',
 "unplug": '<path d="m19 5 3-3"/><path d="m2 22 3-3"/><path d="M6.3 20.3a2.4 2.4 0 0 0 3.4 0L12 18l-6-6-2.3 2.3a2.4 2.4 0 0 0 0 3.4Z"/><path d="M7.5 13.5 10 11"/><path d="M10.5 16.5 13 14"/><path d="m12 6 6 6 2.3-2.3a2.4 2.4 0 0 0 0-3.4l-2.6-2.6a2.4 2.4 0 0 0-3.4 0Z"/>',
}
def ic(name, size=15, color="var(--secondary)", sw=2):
    return f'<svg width="{size}" height="{size}" viewBox="0 0 24 24" fill="none" stroke="{color}" stroke-width="{sw}" stroke-linecap="round" stroke-linejoin="round">{P[name]}</svg>'

GREEN, ORANGE, RED, GREY = "#30d158", "#ff9f0a", "#ff453a", "#8e8e93"
def st(s):
    if s == "x": return f'<span class="st">{ic("x", 10, RED, 2.4)}</span>'
    style = {"g": f"background:{GREEN}", "o": f"background:{ORANGE}", "i": f"border:1px solid {GREY}", "off": f"background:{GREY}"}[s]
    return f'<span class="st"><i style="{style}"></i></span>'

def cb(state, small=False):
    """state: 0 empty, 1 ticked, 2 some, 'auto' ticked for you, 'dis' can't be ticked."""
    k = " s" if small else ""
    z = 9 if small else 10
    if state == 1: return f'<span class="cb y{k}">{ic("check", z, "#fff", 3)}</span>'
    if state == 2: return f'<span class="cb y{k}">{ic("minus", z, "#fff", 3)}</span>'
    if state == "auto": return f'<span class="cb auto{k}">{ic("check", z, "#fff", 3)}</span>'
    if state == "dis": return f'<span class="cb dis{k}"></span>'
    return f'<span class="cb{k}"></span>'

def btn(text, kind="", key=None):
    k = f'<span style="opacity:.7;font-weight:400;font-size:11px">{key}</span>' if key else ""
    return f'<span class="btn {kind}">{text}{k}</span>'

# the fixture: (depth, state, title, fork, bold, inbox, machine)
PAYMENTS = [(0, "o", "Ledger schema migration", False, True, "2", "devbox-1"),
            (1, "o", "Try BRIN index", True, False, None, "devbox-1"),
            (2, "g", "Partial BRIN on posted_at", True, False, None, "devbox-1"),
            (1, "i", "Review the schema", True, False, None, "This Mac"),
            (0, "g", "Refund retries", False, True, None, "This Mac"),
            (0, "i", "API rate limiter", False, False, None, "devbox-1")]
INFRA = [(0, "g", "Benchmark BRIN", False, False, None, "devbox-1"),
         (0, "x", "Upgrade Postgres 17", False, True, None, "devbox-1")]

def sidebar(sel="Home", pick=None, auto=(), gone=(), bar=None, inbox="7"):
    """pick: None for the plain sidebar, else the set of ticked titles (select mode)."""
    mode = pick is not None
    pick = set(pick or ()); auto = set(auto)
    def row(lead, text, trail="", indent=0, on=False, cls="", style="", box=None):
        b = cb(box, True) if box is not None else ""
        return f'<div class="r{" on" if on else ""} {cls}" style="padding-left:{8+indent}px;{style}">{b}{lead}<span class="t">{text}</span>{trail}</div>'
    def nav(icon, text, count=None):
        on = sel == text
        return row(ic(icon, 15, "#fff" if on else "var(--accent)"), text, f'<span class="n">{count}</span>' if count else "", on=on)
    def node(n):
        d, s, title, fork, bold, ib, _m = n
        if title in gone: return ""
        if "Ledger schema migration" in gone: d = 0
        on = sel == title and not mode
        lead = '<span class="lead">' + (ic("fork", 12, "var(--tertiary)") if fork else "") + st(s) + '</span>'
        indent = max(0, 10 + 16 * d - (17 if fork and d else 0))
        trail = f'<span style="display:flex;align-items:center;gap:2px;font-size:11px;color:{RED}">{ic("inbox", 12, RED)}{ib}</span>' if ib and not mode else ""
        box = None; cls = ""
        if mode:
            box = 1 if title in pick else "auto" if title in auto else 0
            cls = "tk" if box else ""
        return row(lead, title, trail, indent + (6 if mode else 0), on, cls, "font-weight:600;" if bold else "", box)
    def tri(nodes):
        live = [n for n in nodes if n[2] not in gone]
        k = sum(1 for n in live if n[2] in pick or n[2] in auto)
        return 0 if k == 0 else 1 if k == len(live) else 2
    def channel(text, open_, nodes=(), roll=None, bold=False, none=False):
        on = sel == text
        if mode:
            lead = ""; box = "dis" if none else tri(nodes) if nodes else 0
            ind = 14
        else:
            lead = ic("down" if open_ else "right", 12, "var(--tertiary)"); box = None; ind = -4
        trail = f'<span style="display:flex;align-items:center;gap:5px">{st(roll[0])}<span class="n">{roll[1]}</span></span>' if roll and not open_ else ""
        return row(lead, text, trail, ind, on, "", f"font-weight:{700 if bold else 500};", box)
    def host(s, text, note="", boxstate=None):
        on = sel == text
        return row(st(s), text, f'<span class="sec" style="{"color:#fff" if on else ""}">{note}</span>' if note else "", 14 if mode else 0, on, "", "", boxstate if mode else None)
    allnodes = PAYMENTS + INFRA + [(0, "i", "acme/api #482", False, False, None, "This Mac")]
    def hosttri(m):
        live = [n for n in allnodes if n[6] == m and n[2] not in gone]
        k = sum(1 for n in live if n[2] in pick or n[2] in auto)
        return 0 if k == 0 else 1 if k == len(live) else 2
    h = '<div class="side">'
    h += f'<div class="shead"><span class="lights"><i style="background:#ff5f57"></i><i style="background:#febc2e"></i><i style="background:#28c840"></i></span><span class="sp"></span>{ic("panel")}{ic("pen")}</div>'
    h += f'<div class="srch">{ic("search", 12, "var(--tertiary)")}<span class="sp">Search</span><span style="font-size:11px">⌘K</span></div><div style="height:4px;flex:none"></div>'
    h += nav("house", "Home") + nav("inbox", "Agent inbox", inbox) + nav("kanban", "To-do") + nav("term", "Desk")
    h += f'<div class="shd"><span class="lbl sp">Channels</span>{ic("plus", 12)}</div>'
    pay = [n for n in PAYMENTS if n[2] not in gone]; inf = [n for n in INFRA if n[2] not in gone]
    h += channel("payments-rewrite", True, pay, bold=True) + "".join(node(n) for n in pay)
    h += channel("infra", True, inf, bold=True) + "".join(node(n) for n in inf)
    h += channel("pr-reviews", False, [allnodes[-1]] if mode else (), ("i", "1")) + channel("web", False, none=True)
    h += '<div class="shd"><span class="lbl sp">Hosts</span></div>'
    h += host("g", "This Mac", boxstate=hosttri("This Mac")) + host("g", "devbox-1", boxstate=hosttri("devbox-1")) + host("off", "gpu-box", "offline", "dis")
    h += '<div class="sp"></div>' + (bar or "")
    h += '<div style="padding-bottom:10px">' + nav("gear", "Settings") + '</div></div>'
    return h

def sidebar_bar(n, note):
    return (f'<div class="sbox"><div style="font-weight:600;font-size:12.5px">{n} selected</div>'
            f'<div class="sec" style="line-height:1.3">{note}</div>'
            f'<div style="display:flex;gap:6px;margin-top:6px">{btn("Cancel")}<span class="btn dng" style="flex:1;justify-content:center;padding:0 6px">Delete {n} sessions…</span></div></div>')

def toolbar(title, actions=""):
    return f'<div class="tb"><h1>{title}</h1><span class="sp"></span>{actions}</div>'

HOME = [("Needs you · 3", [("o", "Ledger schema migration", "# payments-rewrite · devbox-1", "Asks to move PAY-3 to Done", "2m", ("pri", "Review")),
                           ("o", "Try BRIN index", "fork of Ledger schema", "Asks to push try-brin to origin", "9m", ("pri", "Review")),
                           ("x", "Upgrade Postgres 17", "# infra · devbox-1", "Exited with code 1 after the dump step", "40m", ("", "Resume"))]),
        ("Working · 3", [("g", "Partial BRIN on posted_at", "fork of Try BRIN index", "Comparing partial BRIN on posted_at", "now", None),
                         ("g", "Refund retries", "# payments-rewrite", "Running cargo test -p refunds", "now", None),
                         ("g", "Benchmark BRIN", "# infra · devbox-1", "pgbench run 2 of 4", "1m", None)]),
        ("Recent", [("i", "API rate limiter", "# payments-rewrite · devbox-1", "Finished: pushed and opened PR #491", "1h", ("", "Open PR")),
                    ("i", "acme/api #482", "# pr-reviews", "3 new commits since your review", "2h", ("", "Review"))])]

def home_body(pick=None, gone=(), top=24):
    mode = pick is not None
    h = f'<div style="display:flex;justify-content:center;padding:{top}px 24px"><div style="width:720px;display:flex;flex-direction:column;gap:20px">'
    for title, rows in HOME:
        rows = [r for r in rows if r[1] not in gone]
        if not rows: continue
        if gone: title = title.split(" · ")[0] + (f" · {len(rows)}" if "·" in title else "")
        h += f'<div style="display:flex;flex-direction:column;gap:6px"><span class="lbl">{title}</span><div class="card">'
        for s, t, ch, what, when, act in rows:
            if gone and ch == "fork of Try BRIN index": ch = "# payments-rewrite · devbox-1"
            on = mode and t in pick
            h += f'<div class="hrow{" tk" if on else ""}">' + (cb(1 if on else 0) if mode else "") + st(s)
            h += f'<div style="flex:1;min-width:0"><div style="display:flex;gap:8px;align-items:baseline"><span style="font-weight:500">{t}</span><span class="sec">{ch}</span></div>'
            h += f'<div style="font-size:12px;color:{ORANGE if s == "o" else "var(--secondary)"}">{what}</div></div><span class="sec">{when}</span>'
            if act and not mode: h += btn(act[1], act[0])
            h += '</div>'
        h += '</div></div>'
    return h + '</div></div>'

def page(name, side, main, over="", look=LIGHT):
    v = ";".join(f"--{k}:{val}" for k, val in look.items()) + ";--accent-sel:#0a6cff"
    doc = f'<!doctype html><meta charset="utf-8"><title>{name}</title><style>:root{{{v}}}{CSS}</style><body>{side}<div class="main">{main}</div>{over}</body>'
    open(os.path.join(HERE, name + ".html"), "w").write(doc)

# ---- the confirmation sheet ----
def srow(title, who, state, notes=(), tick=None, fork=False, changed=False, icon=None, sub=False):
    """notes: (kind, text) with kind 't' terminals end, 'w' unpushed work, 'q' quiet. tick: None or True/False."""
    g = icon or ("fork" if fork else "term")
    h = f'<div class="srow{" sub" if sub else ""}"><div class="l1">{ic(g, 12)}<span>{title}</span>'
    if changed: h += '<span class="chg">changed</span>'
    h += f'<span class="sp"></span><span class="sec who">{who}</span><span class="sec state">{state}</span></div>'
    for kind, text in notes:
        if kind == "t": h += f'<div class="note o">{ic("term", 11, ORANGE)}<span>{text}</span></div>'
        elif kind == "w": h += f'<div class="note o">{ic("branch", 11, ORANGE)}<span>{text}</span></div>'
        else: h += f'<div class="note q">{text}</div>'
    if tick is not None: h += f'<div class="tick">{cb(1 if tick else 0)}<span>Delete that work too</span></div>'
    return h + '</div>'

def group(title, count, rows):
    return f'<div class="gh"><span style="font-weight:600">{title}</span><span class="sec">{count}</span></div>' + "".join(rows)

def quiet(text, link=None):
    return f'<div class="note q" style="padding:3px 0 3px 19px">{text}' + (f'<span class="lnk">{link}</span>' if link else "") + '</div>'

def sheet(title, sub, summary, body, box, hint, button, top=40, stale=None, typed=None, list_style="", thumb=None, after_list="", at_end=False):
    """summary: (icon, colour, text) lines. typed: None, or (label, value) for the count field."""
    h = f'<div class="scrim"></div><div class="over" style="top:{top}px"><div class="dlg"><h2>{title}</h2>'
    if stale: h += stale
    h += f'<div style="color:var(--secondary)">{sub}</div><div class="sum">'
    for g, col, text in summary:
        h += f'<div style="color:{col}">{ic(g, 12, col) if g != "dot" else st("off")}<span>{text}</span></div>'
    h += f'</div><div class="list{" scroll" if list_style else ""}{" end" if at_end else ""}" style="{list_style}">{body}'
    if thumb: h += f'<span class="thumb" style="top:{thumb[0]}px;height:{thumb[1]}px"></span>'
    h += f'</div>{after_list}<div class="box">{box}</div>'
    if typed:
        label, value = typed
        val = f'{value}<span class="caret"></span>' if value else '<span class="caret"></span>'
        h += f'<div style="display:flex;align-items:center;gap:10px;padding-top:8px"><span>{label}</span><span class="field focus" style="width:86px">{val}</span></div>'
    h += f'<div class="foot"><span class="ter" style="font-size:11px">{hint}</span><span class="sp"></span>{btn("Cancel")}{button}</div></div></div>'
    return h
