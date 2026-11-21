#!/usr/bin/env python3
"""INTENTMODE frames (1600x950) — unified Intent Spec layout (Outcome -> Fences -> Steps -> Live Tracking).

1. im-1-board-open: Calm Intents Board + Open Intent (PAY-14) using the clean
   Intent Spec card layout on top and Live Tracking (Done/Left, Needs you, Sessions) at the bottom.
2. im-2-author-manual: Authoring an Intent directly on the Board without an LLM (+ New intent).
3. im-3-spawn-sessions: Opening PAY-19 and spawning tagged sessions inline from its drawer.
4. im-4-in-review-diff: Opening WEB-8 (In review) and viewing its agent-written Diff (v4 -> v5).
5. im-5-desk-coauthor-and-notify: Optional LLM co-authoring in Desk (dark) + notification toast.
6. im-6-settings-toggle: Global Settings > General switch (Intent mode replaces To-do).
"""
import html
import os

HERE = os.path.dirname(os.path.abspath(__file__))

CSS = """
*{box-sizing:border-box;margin:0;padding:0}
html,body{width:1600px;height:950px;overflow:hidden}
body{
  font:13px/1.44 -apple-system,BlinkMacSystemFont,"SF Pro Text","Helvetica Neue",sans-serif;
  color:#18181b;background:#ffffff;display:flex;-webkit-font-smoothing:antialiased;
}
:root{
  --bg:#ffffff;
  --side:#f4f4f6;
  --panel:#f9f9fb;
  --card:#ffffff;
  --sunken:#f1f1f4;
  --line:#e5e5eb;
  --line-soft:#f0f0f4;
  --ink:#18181b;
  --soft:#4b4b54;
  --muted:#7c7c86;
  --faint:#a0a0aa;
  --blue:#0a66f5;
  --blue-soft:#eff5ff;
  --blue-line:#c7dcff;
  --amber:#a85700;
  --amber-soft:#fff8eb;
  --amber-line:#f0d9ae;
  --red:#c02626;
  --red-soft:#fdecec;
  --green:#16794b;
  --green-soft:#e8f6ee;
  --purple:#6d28d9;
  --purple-soft:#f4f0ff;
  --purple-line:#dcd0fe;
}
body.dark{
  --bg:#1c1c1f;
  --side:#232327;
  --panel:#202024;
  --card:#242428;
  --sunken:#2c2c32;
  --line:#333339;
  --line-soft:#29292f;
  --ink:#f1f1f4;
  --soft:#c4c4cc;
  --muted:#8e8e98;
  --faint:#686872;
  --blue:#3b82f6;
  --blue-soft:#1a2b46;
  --blue-line:#2d4a7a;
  --amber:#f5a623;
  --amber-soft:#312514;
  --amber-line:#5e431c;
  --red:#ff7a70;
  --red-soft:#3b1e20;
  --green:#34d399;
  --green-soft:#153326;
  --purple:#c4b5fd;
  --purple-soft:#2b2144;
  --purple-line:#473672;
  color:var(--ink);background:var(--bg);
}

/* Sidebar */
.side{width:244px;background:var(--side);border-right:1px solid var(--line);padding:14px 12px;display:flex;flex-direction:column;flex:none}
.lights{display:flex;gap:8px;padding:4px 8px 18px}.lights i{width:12px;height:12px;border-radius:50%;display:block}
.srch{background:rgba(128,128,140,.14);border-radius:7px;padding:6px 10px;color:var(--muted);display:flex;justify-content:space-between;margin-bottom:14px;font-size:12.5px}
.nav{padding:6px 10px;border-radius:7px;display:flex;justify-content:space-between;align-items:center;margin-bottom:2px;font-size:13px;color:var(--ink)}
.nav.sel{background:var(--blue);color:#fff;font-weight:600}
.nav .n{color:var(--muted);font-size:12px;font-variant-numeric:tabular-nums}
.nav.sel .n{color:rgba(255,255,255,.84)}
.sh{font-size:11px;font-weight:600;color:var(--muted);padding:16px 10px 6px;letter-spacing:.02em;text-transform:uppercase}
.ch{padding:4px 10px;font-weight:600;display:flex;gap:7px;align-items:center;font-size:12.5px;color:var(--ink)}
.ch b{color:var(--muted);font-weight:400;width:8px}
.ss{padding:4px 10px 4px 26px;display:flex;gap:8px;align-items:center;font-size:12.5px;color:var(--soft)}
.ss.sel{background:var(--blue);color:#fff;border-radius:7px;font-weight:600;margin-left:16px;padding-left:10px}
.dot{width:7px;height:7px;border-radius:50%;display:inline-block;flex:none}
.dg{background:#22c55e}.do{background:#f59e0b}.db{background:#3b82f6}.dm{background:#9ca3af}

/* Main chrome */
.main{flex:1;min-width:0;display:flex;flex-direction:column;position:relative;background:var(--bg)}
.top{height:56px;border-bottom:1px solid var(--line);display:flex;align-items:center;padding:0 22px;gap:10px;flex:none}
.top h1{font-size:16.5px;font-weight:700;letter-spacing:-.015em}
.top .sub{color:var(--muted);font-size:12.5px}
.sp{flex:1}
.btn{border:1px solid var(--line);background:var(--card);border-radius:7px;padding:5.5px 11px;font-size:12.5px;font-weight:500;white-space:nowrap;display:inline-flex;align-items:center;gap:6px;color:var(--ink)}
.dark .btn{background:#2b2b31;border-color:#42424a}
.btn.pri{background:var(--blue);border-color:var(--blue);color:#fff;font-weight:600}
.btn.active-trigger{background:var(--blue-soft);border-color:var(--blue);color:var(--blue);font-weight:600}
.btn.quiet{border-color:transparent;background:transparent;color:var(--soft);padding:5.5px 8px}
.btn.sm{padding:4px 10px;font-size:12px}
.field{border:1px solid var(--line);border-radius:7px;padding:5px 10px;color:var(--muted);width:180px;display:flex;justify-content:space-between;font-size:12.5px;background:var(--card)}
.kbd{font:11px -apple-system,sans-serif;border:1px solid var(--line);border-radius:4px;padding:0 5px;color:var(--muted);background:var(--panel)}

/* Filter strip */
.bar{display:flex;align-items:center;gap:6px;padding:9px 22px;border-bottom:1px solid var(--line);flex:none;background:var(--panel)}
.chip{border-radius:6px;padding:3px 9px;background:var(--sunken);color:var(--soft);font-size:12px;white-space:nowrap}
.chip.on{background:var(--ink);color:var(--bg);font-weight:500}
.chip .c{opacity:.62;margin-left:4px;font-variant-numeric:tabular-nums}
.chip.ghost{background:transparent;color:var(--muted)}

/* Split layout */
.split{flex:1;min-height:0;display:flex}
.blist{width:596px;flex:none;border-right:1px solid var(--line);padding:6px 16px 14px;overflow:hidden;display:flex;flex-direction:column;background:var(--bg)}

/* Board rows — clean, scannable, zero rainbow pill soup */
.sec{display:flex;align-items:baseline;gap:8px;padding:14px 10px 5px;font-size:11px;font-weight:650;color:var(--muted);letter-spacing:.04em;text-transform:uppercase}
.sec:first-child{padding-top:8px}
.sec .cnt{font-weight:500;color:var(--faint)}

.row{display:flex;align-items:flex-start;gap:11px;padding:9px 10px;border-radius:8px}
.row + .row{box-shadow:0 -1px 0 var(--line-soft)}
.row.sel{background:var(--blue-soft);box-shadow:none}
.row.sel + .row{box-shadow:none}

.st-ic{width:15px;height:15px;margin-top:2px;flex:none;display:inline-flex;align-items:center;justify-content:center;border-radius:50%;font-size:10px;font-weight:700}
.ic-blk{border:1.5px solid #d97706;color:#b45309;background:#fffbeb}
.ic-rev{border:1.5px solid #7c3aed;background:radial-gradient(circle,#7c3aed 40%,transparent 44%)}
.ic-prog{border:1.5px solid #2563eb;background:linear-gradient(90deg,#2563eb 50%,transparent 50%)}
.ic-todo{border:1.5px solid #b4b4be}
.ic-draft{border:1.5px dashed #b4b4be}

.rmid{flex:1;min-width:0}
.rtitle{font-size:13.5px;font-weight:600;color:var(--ink);line-height:1.3;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.rsub{font-size:12px;color:var(--muted);margin-top:2px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.rsub .need{color:var(--amber);font-weight:600}
.rsub .rev{color:var(--purple);font-weight:600}

.rright{display:flex;align-items:center;gap:10px;flex:none;margin-top:1px;font-size:12px;font-variant-numeric:tabular-nums}
.rdue{color:var(--muted);white-space:nowrap}
.rdue.soon{color:var(--amber);font-weight:600}
.rdue.over{color:var(--red);font-weight:600}
.rkey{color:var(--faint);width:46px;text-align:right;font-size:11.5px}

/* Right Intent Spec Pane — exact clean structure of ui/changes/src/intent_tab.rs */
.ipane{flex:1;min-width:0;display:flex;flex-direction:column;background:var(--bg)}
.ihdr{height:42px;padding:0 18px;display:flex;align-items:center;gap:9px;background:var(--panel);border-bottom:1px solid var(--line);flex:none}
.ihdr .fn{font-weight:650;font-size:13px;white-space:nowrap;color:var(--ink)}
.valid{background:var(--green-soft);color:var(--green);font-size:11px;font-weight:600;padding:2px 8px;border-radius:999px;white-space:nowrap}
.seg{display:inline-flex;background:var(--sunken);border-radius:7px;padding:2px;gap:1px;flex:none;white-space:nowrap}
.seg i{font-style:normal;padding:2.5px 9px;border-radius:5px;font-size:11.5px;color:var(--soft);white-space:nowrap}
.seg i.on{background:var(--card);color:var(--ink);font-weight:600;box-shadow:0 1px 2px rgba(0,0,0,.08)}
.dark .seg i.on{background:#34343c;box-shadow:none}
.stepper{display:inline-flex;align-items:center;gap:5px;border:1px solid var(--line);background:var(--card);border-radius:6px;padding:2px 8px;font-size:11.5px;color:var(--soft);white-space:nowrap;flex:none}

.imeta{padding:8px 18px;border-bottom:1px solid var(--line);display:flex;align-items:center;gap:8px;white-space:nowrap;font-size:12px;background:var(--bg);flex:none}
.schpill{background:var(--purple-soft);color:var(--purple);padding:2px 8px;border-radius:5px;font-weight:600;font-size:11.5px}
.mpill{background:var(--sunken);color:var(--ink);padding:2px 8px;border-radius:5px;font-size:11.5px;font-weight:500;display:inline-flex;align-items:center;gap:5px}
.mpill.draft{background:transparent;border:1px dashed var(--faint);color:var(--soft)}
.mpill.amber{background:var(--amber-soft);color:var(--amber);font-weight:600}
.mpill.purple{background:var(--purple-soft);color:var(--purple);font-weight:600}

.ibody{flex:1;min-height:0;padding:10px 18px;display:flex;flex-direction:column;gap:8px;overflow:hidden}
.grid2{display:flex;gap:10px;align-items:stretch}
.grid2 > .icard{flex:1;min-width:0}

.icard{border:1px solid var(--line);border-radius:10px;padding:8.5px 13px;background:var(--card);display:flex;flex-direction:column;gap:4.5px}
.icard.dashed{border-style:dashed;background:var(--panel)}
.icard .ct{display:flex;align-items:center;justify-content:space-between;font-size:10.5px;font-weight:650;color:var(--muted);letter-spacing:.04em;text-transform:uppercase}
.icard .ct .sublbl{font-weight:400;text-transform:none;letter-spacing:0;font-size:11.5px;color:var(--muted)}
.fence{background:var(--red-soft);color:var(--red);padding:1px 6px;border-radius:4px;font-size:10px;font-weight:700;letter-spacing:.03em}

.ititle{font-size:16px;font-weight:700;letter-spacing:-.015em;color:var(--ink);line-height:1.25}
.bul{font-size:12.5px;line-height:1.45;color:var(--soft)}
.bul div + div{margin-top:3px}
.bul.main-txt{color:var(--ink)}

/* Deliverable steps — calm, quiet badges */
.dstep{display:flex;align-items:center;gap:9px;font-size:12.5px;padding:1.5px 0}
.cb{width:15px;height:15px;border-radius:4px;border:1.5px solid #b8b8c2;display:inline-flex;align-items:center;justify-content:center;font-size:10px;flex:none}
.cb.ok{background:var(--green);border-color:var(--green);color:#fff}
.cb.prog{border-color:var(--blue);background:linear-gradient(90deg,var(--blue) 50%,transparent 50%)}
.cb.hum{border-color:#d97706;background:var(--amber-soft);color:var(--amber);font-weight:700}
.dstep.done .dtxt{color:var(--muted)}
.dtxt{flex:1;min-width:0;color:var(--ink)}
.dref{font-family:"SF Mono",Menlo,monospace;font-size:11.5px;color:var(--blue);margin-left:5px}
.dstep.done .dref{color:var(--muted)}
.dtype{font-size:10.5px;font-weight:600;color:var(--muted);background:var(--sunken);padding:1px 6.5px;border-radius:4px;flex:none;letter-spacing:.02em}
.dtype.hum{color:var(--amber);background:var(--amber-soft)}

/* Divider separating the Intent Spec (top) from Live Tracking (bottom) */
.track-div{display:flex;align-items:center;gap:10px;margin:2px 0 -2px;font-size:10.5px;font-weight:650;color:var(--muted);letter-spacing:.05em;text-transform:uppercase}
.track-div::before,.track-div::after{content:"";height:1px;background:var(--line);flex:1}
.track-div::before{flex:0 0 12px}

/* Needs-you / Review callouts inside Tracking */
.callout{border-radius:10px;padding:10px 14px;display:flex;flex-direction:column;gap:5px}
.callout.amber{background:var(--amber-soft);border:1px solid var(--amber-line)}
.callout.purple{background:var(--purple-soft);border:1px solid var(--purple-line)}
.callout .chd{display:flex;align-items:center;justify-content:space-between;font-size:11.5px;font-weight:700}
.callout.amber .chd{color:var(--amber)}
.callout.purple .chd{color:var(--purple)}
.callout .cbody{font-size:12.5px;line-height:1.42;color:var(--ink)}
.callout .cacts{display:flex;align-items:center;gap:8px;margin-top:2px}

.track-foot{background:var(--panel);border:1px solid var(--line);border-radius:8px;padding:7px 12px;font-size:12px;color:var(--soft);display:flex;flex-direction:column;gap:3px}
.track-foot .tf1{display:flex;align-items:center;gap:8px;font-size:11.5px;color:var(--muted)}
.track-foot b{color:var(--ink);font-weight:600}
.ses-tag{display:inline-flex;align-items:center;gap:5px;padding:1px 7px;border-radius:5px;background:var(--card);border:1px solid var(--line);color:var(--ink);font-size:11.5px}

.ifoot{height:48px;border-top:1px solid var(--line);background:var(--panel);padding:0 18px;display:flex;align-items:center;gap:8px;flex:none}
.fmeta{font-size:12px;color:var(--muted)}
.mono{font-family:"SF Mono",Menlo,monospace;font-size:11.5px}

/* Editable inputs inside Manual Authoring (Frame 2) */
.edit-inp{border:1px solid var(--line);border-radius:7px;padding:7px 10px;font-size:13px;color:var(--ink);background:var(--bg);line-height:1.45}
.edit-inp.focus{border-color:var(--blue);box-shadow:0 0 0 3px rgba(10,102,245,.1)}
.linkbtn{color:var(--blue);font-size:12px;font-weight:500}

/* Inline Spawn Panel inside PAY-19 (Frame 3) */
.spawn-card{border:1px solid var(--blue-line);background:var(--blue-soft);border-radius:10px;padding:12px 14px;display:flex;flex-direction:column;gap:9px}
.spawn-card .shd{display:flex;align-items:center;justify-content:space-between;font-size:12.5px}
.sopt{border:1px solid var(--line);background:var(--card);border-radius:8px;padding:8px 11px;display:flex;align-items:flex-start;gap:10px}
.sopt.on{border-color:var(--blue)}
.cbx{width:15px;height:15px;border-radius:4px;background:var(--blue);color:#fff;font-size:10px;font-weight:700;display:inline-flex;align-items:center;justify-content:center;flex:none;margin-top:2px}

/* Diff view (Frame 4) */
.diffbox{border:1px solid var(--line);border-radius:9px;background:#1b1b1f;color:#e6e6eb;font:12px/1.54 "SF Mono",Menlo,monospace;padding:8px 0;overflow:hidden}
.dl{padding:0 14px;white-space:pre;display:flex;gap:12px}
.dl .ln{color:#686872;width:22px;text-align:right;flex:none}
.dl.add{background:rgba(34,197,94,.18);color:#bbf7d0}
.dl.del{background:rgba(239,68,68,.18);color:#fecaca}
.dl.hunk{color:#93c5fd;background:rgba(59,130,246,.14)}

/* Desk Terminal (Frame 5) */
.termwrap{width:636px;flex:none;border-right:1px solid var(--line);display:flex;flex-direction:column;background:#161619}
.tabs{display:flex;border-bottom:1px solid var(--line);background:var(--panel);height:40px;align-items:stretch}
.tab{padding:0 14px;border-right:1px solid var(--line);display:flex;gap:7px;align-items:center;font-size:12.5px;color:var(--muted)}
.tab.on{background:#161619;color:#fff;font-weight:600;box-shadow:inset 0 -2px 0 var(--blue)}
.term{flex:1;padding:16px 18px;font:12.5px/1.6 "SF Mono",Menlo,monospace;color:#e6e6ec;white-space:pre-wrap;overflow:hidden}
.term .u{color:#93c5fd;font-weight:600}
.term .ok{color:#34d399}
.term .dim{color:#848490}
.tinput{margin:0 14px 14px;border:1px solid #36363e;border-radius:8px;padding:9px 12px;font:12.5px "SF Mono",Menlo,monospace;color:#f1f1f4;background:#202025}

/* Notification toast */
.toast{position:absolute;right:20px;bottom:64px;width:410px;background:rgba(36,36,42,.96);color:#f4f4f6;border-radius:13px;padding:12px 14px;box-shadow:0 16px 40px rgba(0,0,0,.52),inset 0 0 0 1px rgba(255,255,255,.14);display:flex;gap:11px;z-index:20}
.toast .ic{width:34px;height:34px;border-radius:8px;background:linear-gradient(135deg,#3b82f6,#6d28d9);display:flex;align-items:center;justify-content:center;font-weight:700;font-size:15px;color:#fff;flex:none}
.toast .t1{display:flex;justify-content:space-between;font-size:11px;color:#a4a4ae}
.toast .t2{font-weight:650;font-size:12.5px;margin:2px 0}
.toast .t3{font-size:12px;color:#d4d4dc;line-height:1.38}

/* Settings (Frame 6) */
.snav{width:208px;flex:none;padding:14px 12px;border-right:1px solid var(--line);display:flex;flex-direction:column;gap:2px;background:var(--panel)}
.snav div{padding:6px 10px;border-radius:7px;font-size:13px;color:var(--soft)}
.snav div.sel{background:var(--blue);color:#fff;font-weight:600}
.sbody{flex:1;padding:24px 30px;display:flex;flex-direction:column;gap:16px;max-width:980px}
.scard{border:1px solid var(--line);border-radius:10px;background:var(--card);overflow:hidden}
.scard.hi{border-color:var(--blue)}
.srow{display:flex;align-items:center;gap:20px;padding:15px 18px;border-bottom:1px solid var(--line-soft)}
.scard .srow:last-child{border-bottom:0}
.srow .stxt{flex:1;min-width:0}
.srow .stxt > b{font-size:13.5px;font-weight:600;display:flex;align-items:center;gap:8px;color:var(--ink)}
.srow .stxt small{display:block;font-size:12.5px;color:var(--muted);margin-top:3px;line-height:1.48}
.srow .stxt small b{font-weight:600;color:var(--soft)}
.tog{width:34px;height:19px;border-radius:10px;padding:2px;background:#d2d2da;display:flex;flex:none}
.tog i{width:15px;height:15px;border-radius:50%;background:#fff;display:block;box-shadow:0 1px 2px rgba(0,0,0,.2)}
.tog.on{background:#22c55e;justify-content:flex-end}
"""


def esc(s: str) -> str:
    return html.escape(s)


def sidebar(sel="Intents"):
    items = [
        ("Home", ""),
        ("Agent inbox", "1"),
        ("Reviews", "2"),
        ("Intents", "6"),
        ("Desk", ""),
    ]
    nav_html = "".join(
        f'<div class="nav{" sel" if label == sel else ""}"><span>{label}</span>'
        f'{f"<span class=n>{badge}</span>" if badge else ""}</div>'
        for label, badge in items
    )
    return f"""<div class="side">
  <div class="lights"><i style="background:#ff5f57"></i><i style="background:#febc2e"></i><i style="background:#28c840"></i></div>
  <div class="srch"><span>Search intents, files…</span><span>⌘K</span></div>
  {nav_html}
  <div class="sh">Channels</div>
  <div class="ch"><b>⌄</b># payments-rewrite</div>
  <div class="ss{" sel" if sel == "DeskSession" else ""}"><i class="dot dg"></i>Desk · Intent author</div>
  <div class="ss"><i class="dot do"></i>API cursors</div>
  <div class="ss"><i class="dot dg"></i>Web hooks</div>
  <div class="ss"><i class="dot dg"></i>Backfill runner</div>
  <div class="ch"><b>⌄</b># web</div>
  <div class="ss"><i class="dot dg"></i>Ledger totals</div>
  <div class="ss"><i class="dot dg"></i>Checkout rollout</div>
  <div class="ch"><b>⌄</b># infra</div>
  <div class="ss"><i class="dot dg"></i>Upgrade PG17</div>
  <div class="sh">Hosts</div>
  <div class="ss" style="padding-left:10px"><i class="dot dg"></i>This Mac</div>
  <div class="ss" style="padding-left:10px"><i class="dot dg"></i>devbox-1</div>
  <div class="nav{" sel" if sel == "Settings" else ""}" style="margin-top:auto"><span>Settings</span></div>
</div>"""


def board_row(key, title, ic_cls, ic_txt, sub_html, due, due_cls="", sel=False):
    due_html = f'<span class="rdue {due_cls}">{esc(due)}</span>' if due else ""
    return f"""<div class="row{" sel" if sel else ""}">
  <span class="st-ic {ic_cls}">{ic_txt}</span>
  <div class="rmid">
    <div class="rtitle">{esc(title)}</div>
    <div class="rsub">{sub_html}</div>
  </div>
  <div class="rright">{due_html}<span class="rkey">{esc(key)}</span></div>
</div>"""


def board_list(selected="PAY-14"):
    return (
        '<div class="sec">Needs you <span class="cnt">2</span></div>'
        + board_row(
            "PAY-14",
            "Keyset pagination on GET /users",
            "ic-blk", "!",
            '<span class="need">Blocked</span> · API cursors asked about deleted-row cursors · 1/4 done',
            "Tomorrow", "soon",
            sel=(selected == "PAY-14"),
        )
        + board_row(
            "WEB-8",
            "Checkout page shows new ledger totals",
            "ic-rev", "",
            '<span class="rev">In review</span> · PR acme/web#214 ready + 1 human flag step · 2/3 done',
            "Today", "soon",
            sel=(selected == "WEB-8"),
        )
        + '<div class="sec">In progress <span class="cnt">2</span></div>'
        + board_row(
            "PAY-3",
            "Ledger schema v2 & dual-write cutover",
            "ic-prog", "",
            'Staging backfill done (3.1M rows); prod cutover left · 2 sessions',
            "Fri 27 Nov", "",
            sel=(selected == "PAY-3"),
        )
        + board_row(
            "INF-2",
            "Postgres 17 upgrade & logical replica cutover",
            "ic-prog", "",
            'Replica Terraform PR open; soak & failover left · 1 session',
            "Overdue 1d", "over",
            sel=(selected == "INF-2"),
        )
        + '<div class="sec">Todo <span class="cnt">2</span></div>'
        + board_row(
            "PAY-19",
            "Webhook HMAC key rollover (24h dual-verify)",
            "ic-todo", "",
            'Outcome & guardrails set · #payments-rewrite · ready to start',
            "Mon 30 Nov", "",
            sel=(selected == "PAY-19"),
        )
        + board_row(
            "INF-6",
            "Rotate staging database credentials",
            "ic-todo", "",
            'Outcome & guardrails set · #infra · no sessions yet',
            "Mon 23 Nov", "",
            sel=(selected == "INF-6"),
        )
        + '<div class="sec">Draft <span class="cnt">1</span></div>'
        + board_row(
            "WEB-11",
            "Saved cards vault migration",
            "ic-draft", "",
            'Drafting in #payments-rewrite Desk',
            "", "",
            sel=(selected == "WEB-11"),
        )
    )


def board_header(new_active=False):
    new_cls = "btn active-trigger" if new_active else "btn pri"
    return (
        '<div class="top"><h1>Intents</h1><span class="sub">6 active across 3 channels</span>'
        '<span class="sp"></span>'
        '<span class="field"><span>Filter intents…</span><span class="kbd">/</span></span>'
        '<span class="btn">✦ Draft with AI</span>'
        f'<span class="{new_cls}">+ New intent</span></div>'
        '<div class="bar">'
        '<span class="chip on">All<span class="c">6</span></span>'
        '<span class="chip">payments-rewrite<span class="c">3</span></span>'
        '<span class="chip">web<span class="c">1</span></span>'
        '<span class="chip">infra<span class="c">2</span></span>'
        '<span class="sp"></span>'
        '<span class="chip ghost">Show done (4)</span></div>'
    )


def write_frame(name, body_html, sel="Intents", dark=False):
    doc = (
        f'<!doctype html><meta charset="utf-8"><title>{name}</title>'
        f'<style>{CSS}</style><body class="{"dark" if dark else ""}">'
        f'{sidebar(sel=sel)}<div class="main">{body_html}</div></body>'
    )
    with open(os.path.join(HERE, f"{name}.html"), "w") as f:
        f.write(doc)


# ==============================================================================
# FRAME 1: Board + Open Intent (PAY-14)
# Exact Intent Spec card structure on top (Outcome -> Non-goals | Guardrails ->
# Deliverable Steps), then Live Tracking (Done/Left, Needs you, Sessions) at bottom.
# ==============================================================================
f1_right = """<div class="ipane">
  <div class="ihdr">
    <span class="fn">PAY-14 · intent.yaml</span>
    <span class="valid">✓ Schema valid</span>
    <span class="sp"></span>
    <span class="seg"><i class="on">Structured</i><i>YAML</i><i>Diff</i></span>
    <span class="stepper">‹ &nbsp;v4 of 4&nbsp; ›</span>
  </div>
  <div class="imeta">
    <span class="schpill">humata.intent/v2</span>
    <span class="mpill amber"><i class="dot do"></i>Blocked ⌄</span>
    <span class="mpill">#payments-rewrite</span>
    <span class="mpill amber">Due tomorrow (Sun 22 Nov) ✎</span>
    <span class="sp"></span>
    <span style="color:var(--amber);font-weight:600">1 blocking question</span>
  </div>
  <div class="ibody">
    <div class="icard">
      <div class="ct"><span>OUTCOME</span><span class="sublbl">Set at start · owner-locked</span></div>
      <div class="ititle">Keyset pagination on GET /users</div>
      <div class="bul main-txt">Replace slow deep-page offset pagination on <span class="mono">GET /users</span> with opaque keyset cursors so large directories stay O(1) with zero skipped or duplicate rows.</div>
    </div>

    <div class="grid2">
      <div class="icard">
        <div class="ct"><span>NON-GOALS</span></div>
        <div class="bul">
          <div>• Keeping legacy <span class="mono">?offset=</span> after cutover</div>
          <div>• Cursor pagination on admin export endpoints</div>
        </div>
      </div>
      <div class="icard">
        <div class="ct"><span>INVARIANTS &amp; GUARDRAILS</span><span class="fence">HARD FENCE</span></div>
        <div class="bul">
          <div>• Zero duplicate or skipped rows across deletes</div>
          <div>• No new third-party crates; ask owner on schema/flag changes</div>
        </div>
      </div>
    </div>

    <div class="icard">
      <div class="ct"><span>DELIVERABLE STEPS · 1 OF 4 DONE</span><span class="sublbl">added by sessions while cooking · PRs, human steps &amp; deploys</span></div>
      <div class="dstep done">
        <span class="cb ok">✓</span>
        <span class="dtxt">Opaque cursor codec &amp; GET /users handler<span class="dref">acme/api#489</span></span>
        <span class="dtype">PR</span>
      </div>
      <div class="dstep">
        <span class="cb prog"></span>
        <span class="dtxt">Web directory infinite cursor pagination &amp; deleted-anchor fix<span class="dref">acme/web#216</span></span>
        <span class="dtype">PR</span>
      </div>
      <div class="dstep">
        <span class="cb hum">!</span>
        <span class="dtxt">Enable <span class="mono">users.keyset_v2</span> feature flag (10% → 100% prod)</span>
        <span class="dtype hum">HUMAN</span>
      </div>
      <div class="dstep">
        <span class="cb"></span>
        <span class="dtxt">Prod rollout &amp; remove deprecated <span class="mono">?offset=</span> path after 48h soak</span>
        <span class="dtype">DEPLOY</span>
      </div>
    </div>

    <div class="track-div">LIVE TRACKING · UPDATED 3M AGO BY 2 TAGGED SESSIONS</div>

    <div class="grid2">
      <div class="icard">
        <div class="ct"><span>WHAT WE HAVE DONE</span></div>
        <div class="bul main-txt">
          <div>• Shipped base64url HMAC cursor codec + handler (<span class="mono">acme/api#489</span> merged).</div>
          <div>• Migrated React user table off <span class="mono">?offset=</span> in <span class="mono">acme/web#216</span>.</div>
        </div>
      </div>
      <div class="icard">
        <div class="ct"><span>WHAT WE BELIEVE IS LEFT</span></div>
        <div class="bul main-txt">
          <div>• Land composite <span class="mono">(created_at, id)</span> seek fix once Q1 is answered, then merge <span class="mono">acme/web#216</span>.</div>
          <div>• Flip <span class="mono">users.keyset_v2</span> flag &amp; retire legacy offset param after 48h soak.</div>
        </div>
      </div>
    </div>

    <div class="callout amber">
      <div class="chd"><span>● NEEDS YOU · Q1 (EDGE CASE FROM API CURSORS)</span><span style="font-weight:500;color:var(--muted)">3m ago</span></div>
      <div class="cbody">When an anchor user is hard-deleted between pages, a bare <span class="mono">user_id</span> lookup fails. Encoding <span class="mono">(created_at_us, user_id)</span> inside the HMAC cursor keeps seek O(1). Good to go?</div>
      <div class="cacts">
        <span class="btn pri sm">Use (created_at_us, user_id)</span>
        <span class="btn sm">Reply…</span>
        <span class="sp"></span>
        <span class="btn quiet sm">Open API cursors ↗</span>
      </div>
    </div>

    <div class="track-foot">
      <div class="tf1">
        <span><b>Sessions</b></span>
        <span class="ses-tag"><i class="dot do"></i>API cursors (claude)</span>
        <span class="ses-tag"><i class="dot dg"></i>Web hooks (codex)</span>
        <span class="sp"></span>
        <span>Runs 2 · $0.48 · 1 comment</span>
      </div>
      <div><b>You</b> (on v2): Keep cursor format version-prefixed (<span class="mono">v1_…</span>) so future tweaks never break live tabs.</div>
    </div>
  </div>
  <div class="ifoot">
    <span class="btn pri">Start session</span>
    <span class="btn">Tag session…</span>
    <span class="btn">Add comment</span>
    <span class="btn">Edit</span>
    <span class="sp"></span>
    <span class="fmeta">Logs &amp; 4 versions · ☆ Pin</span>
  </div>
</div>"""

write_frame(
    "im-1-board-open",
    board_header() + f'<div class="split"><div class="blist">{board_list("PAY-14")}</div>{f1_right}</div>',
)

# ==============================================================================
# FRAME 2: Authoring an Intent WITHOUT an LLM (+ New intent clicked in header)
# Uses the exact same Intent Spec card structure as a clean editable form.
# ==============================================================================
f2_right = """<div class="ipane">
  <div class="ihdr">
    <span class="fn">New intent · direct editor</span>
    <span class="sp"></span>
    <span class="seg"><i class="on">Structured</i><i>YAML</i></span>
  </div>
  <div class="imeta">
    <span class="schpill">humata.intent/v2</span>
    <span class="mpill">State: Todo ⌄</span>
    <span class="mpill">#payments-rewrite ⌄</span>
    <span class="mpill" style="border:1px solid var(--blue);background:var(--blue-soft);color:var(--blue)">Due: Mon 30 Nov (9d)</span>
    <span class="sp"></span>
    <span style="color:var(--muted)">No AI required</span>
  </div>
  <div class="ibody" style="gap:12px">
    <div class="icard">
      <div class="ct"><span>TITLE &amp; OUTCOME</span><span class="sublbl">High-level what &amp; why — skip impl details</span></div>
      <div class="edit-inp focus" style="font-size:15px;font-weight:650">Webhook HMAC key rollover (24h dual-verify)<span style="color:var(--blue);font-weight:300">|</span></div>
      <div class="edit-inp">Customers and ops can roll webhook HMAC signing secrets with zero dropped webhooks by accepting both primary and retiring secrets during a 24-hour window, expiring strictly on schedule.</div>
    </div>

    <div class="grid2">
      <div class="icard">
        <div class="ct"><span>NON-GOALS</span><span class="sublbl">Optional</span></div>
        <div class="edit-inp" style="min-height:76px;color:var(--soft)">• Self-serve web UI rotation screen (API-first)<br>• Multi-region KMS re-architecture</div>
      </div>
      <div class="icard">
        <div class="ct"><span>INVARIANTS &amp; GUARDRAILS</span><span class="fence">HARD FENCE</span></div>
        <div class="edit-inp" style="min-height:76px;color:var(--soft)">• Constant-time HMAC verification; hard reject past <span class="mono">retiring_until</span><br>• Never write raw or partial secrets to logs or DB audit rows</div>
      </div>
    </div>

    <div class="icard dashed">
      <div class="ct"><span>DELIVERABLE STEPS · 0 YET</span><span class="sublbl">Optional — leave empty for agents to populate as they cook</span></div>
      <div class="bul">
        You don't need to pre-plan micro-steps. Leave this empty and spawned sessions will add real PRs, manual human actions, and deployments once work begins — or <span class="linkbtn">+ add a known milestone now</span>.
      </div>
    </div>

    <div class="icard">
      <div class="ct"><span>ACCEPTANCE</span><span class="sublbl">Optional</span></div>
      <div class="edit-inp" style="color:var(--soft)">• Dual-valid window accepts both keys; requests past <span class="mono">retiring_until</span> return <span class="mono">401</span>.</div>
    </div>
  </div>
  <div class="ifoot">
    <span class="btn pri">Create intent (Todo)</span>
    <span class="btn">Create &amp; spawn session…</span>
    <span class="sp"></span>
    <span class="btn quiet">✦ Refine with AI…</span>
  </div>
</div>"""

write_frame(
    "im-2-author-manual",
    board_header(new_active=True) + f'<div class="split"><div class="blist">{board_list(selected="")}</div>{f2_right}</div>',
)

# ==============================================================================
# FRAME 3: PAY-19 Selected & Open — Starting a New Session OR Tagging an Existing Session
# ==============================================================================
f3_right = """<div class="ipane">
  <div class="ihdr">
    <span class="fn">PAY-19 · intent.yaml</span>
    <span class="valid">✓ Schema valid</span>
    <span class="sp"></span>
    <span class="seg"><i class="on">Structured</i><i>YAML</i><i>Diff</i></span>
    <span class="stepper">‹ &nbsp;v1 of 1&nbsp; ›</span>
  </div>
  <div class="imeta">
    <span class="schpill">humata.intent/v2</span>
    <span class="mpill">Todo ⌄</span>
    <span class="mpill">#payments-rewrite</span>
    <span class="mpill">Due Mon 30 Nov (9d) ✎</span>
    <span class="sp"></span>
    <span style="color:var(--green);font-weight:600">Ready · 0 blocking questions</span>
  </div>
  <div class="ibody" style="gap:10px">
    <div class="icard">
      <div class="ct"><span>OUTCOME</span></div>
      <div class="ititle">Webhook HMAC key rollover (24h dual-verify)</div>
      <div class="bul main-txt">Customers and ops can roll webhook HMAC signing secrets with zero dropped webhooks by accepting both primary and retiring secrets during a 24-hour window, expiring strictly on schedule.</div>
    </div>

    <div class="grid2">
      <div class="icard">
        <div class="ct"><span>NON-GOALS</span></div>
        <div class="bul">
          <div>• Self-serve web UI rotation screen (API-first)</div>
          <div>• Multi-region KMS re-architecture</div>
        </div>
      </div>
      <div class="icard">
        <div class="ct"><span>INVARIANTS &amp; GUARDRAILS</span><span class="fence">HARD FENCE</span></div>
        <div class="bul">
          <div>• Constant-time HMAC verification; hard reject past <span class="mono">retiring_until</span></div>
          <div>• Never write raw or partial secrets to logs or DB audit rows</div>
        </div>
      </div>
    </div>

    <div class="icard dashed">
      <div class="ct"><span>DELIVERABLE STEPS · 0 YET</span><span class="sublbl">populated as agents start cooking</span></div>
      <div class="bul">Empty upfront. Dedicated or tagged sessions will record real PRs, manual human steps, and deploys here as work takes shape.</div>
    </div>

    <div class="track-div">START A NEW SESSION OR TAG A RUNNING ONE</div>

    <div class="spawn-card">
      <div class="shd">
        <b>New session for PAY-19</b>
        <span style="color:var(--muted);font-size:12px">Dedicated to PAY-19 · moves Todo → In progress</span>
      </div>
      <div style="display:flex;gap:8px;align-items:center">
        <div class="edit-inp" style="flex:1;padding:5px 9px;font-weight:600">Webhook HMAC key rollover</div>
        <span class="btn sm">#payments-rewrite ⌄</span>
        <span class="btn sm">Claude Code ⌄</span>
        <span class="btn sm">devbox-1 ⌄</span>
      </div>
      <div style="font-size:12px;color:var(--soft)">Starts with PAY-19's outcome &amp; guardrails (<span class="mono">HUMATA_INTENT_KEY=PAY-19</span>) and instructions to keep <b>Done / Left</b> and real deliverable steps updated.</div>
      <div style="display:flex;align-items:center;gap:8px">
        <span style="font-size:12px;color:var(--muted)">Or tag an already-spawned session:</span>
        <span class="ses-tag"><i class="dot dg"></i>Desk · Intent author <span style="color:var(--blue);font-weight:600;margin-left:4px">+ Tag</span></span>
        <span class="sp"></span>
        <span class="btn sm">Cancel</span>
        <span class="btn pri sm">Start session</span>
      </div>
    </div>
  </div>
  <div class="ifoot">
    <span class="btn active-trigger">Start session</span>
    <span class="btn">Tag session…</span>
    <span class="btn">Add comment</span>
    <span class="btn">Edit</span>
    <span class="sp"></span>
    <span class="fmeta">v1 · ☆ Pin</span>
  </div>
</div>"""

write_frame(
    "im-3-spawn-sessions",
    board_header() + f'<div class="split"><div class="blist">{board_list("PAY-19")}</div>{f3_right}</div>',
)

# ==============================================================================
# FRAME 4: WEB-8 (In review) — Inspecting Agent Write-Back in Diff Tab (v4 -> v5)
# ==============================================================================
f4_right = """<div class="ipane">
  <div class="ihdr">
    <span class="fn">WEB-8 · intent.yaml</span>
    <span class="valid">✓ Schema valid</span>
    <span class="sp"></span>
    <span class="seg"><i>Structured</i><i>YAML</i><i class="on">Diff</i></span>
    <span class="stepper">‹ &nbsp;v5 of 5 (Checkout rollout)&nbsp; ›</span>
  </div>
  <div class="imeta">
    <span class="schpill">humata.intent/v2</span>
    <span class="mpill purple">In review ⌄</span>
    <span class="mpill">#web</span>
    <span class="mpill amber">Due today (Sat 21 Nov)</span>
    <span class="sp"></span>
    <span style="color:var(--purple);font-weight:600">PR ready + 1 human step</span>
  </div>
  <div class="ibody" style="gap:11px">
    <div class="callout purple">
      <div class="chd"><span>● MOVED IN PROGRESS → IN REVIEW BY CHECKOUT ROLLOUT</span><span style="font-weight:500;color:var(--muted)">4m ago</span></div>
      <div class="cbody">Opened <span class="mono">acme/web#214</span> after verifying legacy vs v2 totals match to the cent across 500 staging orders. Next step after merge is flipping the prod feature flag.</div>
      <div class="cacts">
        <span class="btn pri sm">Review acme/web#214 ↗</span>
        <span class="btn sm">Open session</span>
      </div>
    </div>

    <div class="icard">
      <div class="ct"><span>SEMANTIC CHANGES (v4 → v5)</span><span class="sublbl">DOCV #5 · written by Checkout rollout</span></div>
      <div class="bul main-txt">
        <div>• <b>State:</b> <span class="mono">in_progress → in_review</span></div>
        <div>• <b>Progress:</b> Updated <b>Done</b> &amp; <b>Left</b> with staging verification results</div>
        <div>• <b>Deliverables:</b> Added PR <span class="mono">acme/web#214</span> and <b>HUMAN</b> flag flip step</div>
      </div>
    </div>

    <div class="diffbox">
      <div class="dl hunk"><span class="ln">@@</span><span> WEB-8.intent.yaml — agent write-back (v4 → v5)</span></div>
      <div class="dl del"><span class="ln"> 4</span><span>- state: in_progress</span></div>
      <div class="dl add"><span class="ln"> 4</span><span>+ state: in_review</span></div>
      <div class="dl"><span class="ln"> 5</span><span>  due: 2026-11-21</span></div>
      <div class="dl"><span class="ln">14</span><span>progress:</span></div>
      <div class="dl add"><span class="ln">15</span><span>+ done: "Ledger v2 balance selector + JPY rounding shipped in acme/web#214."</span></div>
      <div class="dl add"><span class="ln">16</span><span>+ left: "Merge acme/web#214, then flip checkout.ledger_totals_v2 in prod."</span></div>
      <div class="dl"><span class="ln">18</span><span>steps:</span></div>
      <div class="dl add"><span class="ln">19</span><span>+ - kind: pr</span></div>
      <div class="dl add"><span class="ln">20</span><span>+   title: Checkout UI reads ledger_balances_v2</span></div>
      <div class="dl add"><span class="ln">21</span><span>+   ref: acme/web#214</span></div>
      <div class="dl add"><span class="ln">22</span><span>+   status: in_review</span></div>
      <div class="dl add"><span class="ln">23</span><span>+ - kind: human</span></div>
      <div class="dl add"><span class="ln">24</span><span>+   title: Enable checkout.ledger_totals_v2 flag in LaunchDarkly</span></div>
      <div class="dl add"><span class="ln">25</span><span>+   status: todo</span></div>
    </div>

    <div class="track-foot">
      <div class="tf1"><span><b>Checkout rollout (codex)</b> left a handoff note</span><span class="sp"></span><span>4m ago · v5</span></div>
      <div>Verified legacy &amp; v2 totals match to the cent on 500 staging orders before opening <span class="mono">acme/web#214</span>.</div>
    </div>
  </div>
  <div class="ifoot">
    <span class="btn pri">Mark Intent Done ✓</span>
    <span class="btn">Request changes (→ In progress)</span>
    <span class="sp"></span>
    <span class="fmeta">Runs 2 · $0.62</span>
  </div>
</div>"""

write_frame(
    "im-4-in-review-diff",
    board_header() + f'<div class="split"><div class="blist">{board_list("WEB-8")}</div>{f4_right}</div>',
)

# ==============================================================================
# FRAME 5: Optional LLM Co-Authoring in Desk (Dark) + Notification Toast
# ==============================================================================
f5_term = """<div class="termwrap">
  <div class="tabs">
    <div class="tab on"><i class="dot dg"></i>Claude Code · Intent author</div>
    <div class="tab"><i class="dot do"></i>API cursors (PAY-14)</div>
    <div class="tab">+</div>
  </div>
  <div class="term"><span class="u">You ›</span> Let's create an intent for webhook HMAC key rollover in #payments-rewrite, due Nov 30. Zero-downtime 24h dual-key window; never log secrets. Keep it high-level — let the coding sessions figure out exact impl &amp; PR splits once they start cooking.

<span class="ok">● Claude Code</span> <span class="dim">(humata-intent-spec)</span>
Drafted <span class="ok">PAY-19.intent.yaml</span> (v1) beside us — kept deliberately outcome-level:

  • <b>Due:</b> Mon 30 Nov 2026 (9 days)
  • <b>Outcome:</b> Zero-downtime webhook secret rotation with a 24h dual-verify window
  • <b>Non-goals (to comfortably hit Nov 30):</b> Self-serve UI dashboard; Ed25519 switch
  • <b>Hard fences:</b> Constant-time HMAC compare; zero secret material in logs/traces
  • <b>Steps:</b> Left empty for now — sessions spawned on PAY-19 will add real PRs, manual flag flips, and deployments as they cook.

Ready to save to the Board as <b>TODO</b> or spawn sessions right away.</div>
  <div class="tinput">&gt; Love it — let's put it on the board and spawn 2 sessions▌</div>
</div>"""

f5_right = """<div class="ipane">
  <div class="ihdr">
    <span class="fn">PAY-19 · intent.yaml</span>
    <span class="valid">✓ Schema valid</span>
    <span class="sp"></span>
    <span class="seg"><i class="on">Structured</i><i>YAML</i><i>Diff</i></span>
    <span class="stepper">‹ &nbsp;v1 of 1&nbsp; ›</span>
  </div>
  <div class="imeta">
    <span class="schpill">humata.intent/v2</span>
    <span class="mpill draft">Draft · #payments-rewrite</span>
    <span class="mpill">Due Mon 30 Nov (9d) ✎</span>
    <span class="sp"></span>
    <span style="color:var(--green);font-weight:600">Ready · 0 blocking questions</span>
  </div>
  <div class="ibody" style="gap:12px">
    <div class="icard">
      <div class="ct"><span>OUTCOME (WHAT &amp; WHY — NO PREMATURE IMPL DETAILS)</span></div>
      <div class="ititle">Webhook HMAC key rollover (24h dual-verify)</div>
      <div class="bul main-txt">Customers and ops can roll webhook HMAC signing secrets with zero dropped webhooks by accepting both primary and retiring secrets during a 24-hour window, expiring strictly on schedule.</div>
    </div>

    <div class="grid2">
      <div class="icard">
        <div class="ct"><span>NON-GOALS (SCOPED FOR NOV 30)</span></div>
        <div class="bul">
          <div>• Self-serve web UI rotation screen (API-first)</div>
          <div>• Multi-region KMS re-architecture</div>
        </div>
      </div>
      <div class="icard">
        <div class="ct"><span>INVARIANTS &amp; GUARDRAILS</span><span class="fence">HARD FENCE</span></div>
        <div class="bul">
          <div>• Constant-time HMAC verification; hard reject past <span class="mono">retiring_until</span></div>
          <div>• Never write raw or partial secrets to logs or DB audit rows</div>
        </div>
      </div>
    </div>

    <div class="icard dashed">
      <div class="ct"><span>DELIVERABLE STEPS · 0 YET</span><span class="sublbl">populated as agents start cooking</span></div>
      <div class="bul">Intentionally empty before work starts. Once spawned, tagged sessions add real trackable outcomes here — <b>PRs</b>, <b>manual human steps</b> (e.g. feature flags / secret cutover), and <b>deployments</b> — never micromanaged chores.</div>
    </div>

    <div class="icard">
      <div class="ct"><span>ACCEPTANCE</span></div>
      <div class="bul">
        <div>• Dual-valid window accepts both keys; requests after <span class="mono">retiring_until</span> return <span class="mono">401</span>.</div>
      </div>
    </div>
  </div>
  <div class="ifoot">
    <span class="btn">Add inline comment</span>
    <span class="sp"></span>
    <span class="btn">Save to Board (TODO)</span>
    <span class="btn pri">Approve &amp; Spawn Sessions ⌄</span>
  </div>
</div>"""

f5_toast = """<div class="toast">
  <div class="ic">H</div>
  <div style="flex:1;min-width:0">
    <div class="t1"><span><b>Humata · Intent needs you</b></span><span>now</span></div>
    <div class="t2">PAY-14 · Edge case question (Due tomorrow)</div>
    <div class="t3">Claude in <b>API cursors</b> hit a deleted-row cursor edge case (Q1). Click to answer on PAY-14.</div>
  </div>
</div>"""

write_frame(
    "im-5-desk-coauthor-and-notify",
    '<div class="top"><h1># payments-rewrite · Desk</h1><span class="sub">Co-authoring PAY-19 with Claude Code</span>'
    '<span class="sp"></span><span class="btn">Open Intents Board (6)</span></div>'
    f'<div class="split">{f5_term}{f5_right}</div>{f5_toast}',
    sel="DeskSession",
    dark=True,
)

# ==============================================================================
# FRAME 6: Settings > General — Global Intent Mode Toggle (Replaces To-do)
# ==============================================================================
snav = "".join(
    f'<div class="{"sel" if s == "General" else ""}">{s}</div>'
    for s in ("General", "Agents", "Sync", "Machines", "Browser", "Agent access", "Notifications", "Keyboard")
)

f6_body = f"""<div class="top"><h1>Settings</h1><span class="sub">General</span></div>
<div class="split">
  <div class="snav">{snav}</div>
  <div class="sbody">
    <div style="font-size:11.5px;font-weight:650;color:var(--muted);letter-spacing:.04em;text-transform:uppercase">Planning &amp; Board (Global)</div>
    <div class="scard hi">
      <div class="srow">
        <div class="stxt">
          <b>Intent mode <span class="schpill">Global · Replaces To-do</span></b>
          <small>Replaces To-dos everywhere in the app with high-level <b>Intent Packages</b> (Draft · Todo · In progress · In review · Blocked · Done + due dates). Author intents directly by hand or co-author them with an LLM; tagged sessions automatically update Done, Left, and real deliverables (PRs, manual human steps, deployments).</small>
        </div>
        <span class="tog on"><i></i></span>
      </div>
      <div class="srow">
        <div class="stxt">
          <b>Tagged sessions update their intent automatically</b>
          <small>Sessions spawned from an intent carry <span class="mono">HUMATA_INTENT_KEY</span> and record <b>What's done</b>, <b>What's left</b>, and real deliverable milestones — never micro-chores like “review PR” or “iterate until green”.</small>
        </div>
        <span class="tog on"><i></i></span>
      </div>
      <div class="srow">
        <div class="stxt">
          <b>Notify me when an intent needs me</b>
          <small>Alerts only when an intent moves to <b>In review</b>, hits a blocking question or edge case, has an actionable <b>Human</b> step, or is due within 24 hours while blocked or unstarted.</small>
        </div>
        <span class="tog on"><i></i></span>
      </div>
    </div>

    <div style="font-size:11.5px;font-weight:650;color:var(--muted);letter-spacing:.04em;text-transform:uppercase;margin-top:6px">Appearance &amp; Data</div>
    <div class="scard">
      <div class="srow">
        <div class="stxt"><b>Theme</b><small>Follow macOS appearance across Desk, Intents, and Reviews.</small></div>
        <span class="seg"><i class="on">System</i><i>Light</i><i>Dark</i></span>
      </div>
      <div class="srow">
        <div class="stxt"><b>Switching back to To-do mode</b><small>Non-destructive — existing Intents and legacy To-dos stay preserved in your workspace if you toggle back later.</small></div>
        <span class="btn sm">View data</span>
      </div>
    </div>
  </div>
</div>"""

write_frame("im-6-settings-toggle", f6_body, sel="Settings")
print("Wrote 6 frames.")
