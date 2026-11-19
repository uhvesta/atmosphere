#!/usr/bin/env python3
"""Shared parts of the To-do and Agent inbox mockups."""
import os, html
HERE = os.path.dirname(os.path.abspath(__file__))

CSS = """
*{box-sizing:border-box;margin:0;padding:0}
html,body{width:1600px;height:950px;overflow:hidden}
body{font:13px/1.35 -apple-system,BlinkMacSystemFont,"SF Pro Text","Helvetica Neue",sans-serif;color:#1c1c1e;background:#fff;display:flex;-webkit-font-smoothing:antialiased}
:root{--line:#e6e6ea;--muted:#7a7a80;--soft:#55555b;--sunken:#f3f3f5;--blue:#0a66f5;--amber:#c76a00;--amber-soft:#fff4e0;--red:#a51d1d;--red-soft:#fde4e4;--teal:#0f766e;--teal-soft:#dff5f1}
.side{width:280px;background:#f3f3f6;border-right:1px solid var(--line);padding:14px 12px;display:flex;flex-direction:column;flex:none}
.lights{display:flex;gap:8px;padding:6px 8px 18px}.lights i{width:12px;height:12px;border-radius:50%}
.srch{background:#e6e6ea;border-radius:7px;padding:6px 10px;color:var(--muted);display:flex;justify-content:space-between;margin-bottom:14px}
.nav{padding:6px 10px;border-radius:7px;display:flex;justify-content:space-between;align-items:center;margin-bottom:2px}
.nav.sel{background:var(--blue);color:#fff;font-weight:600}
.nav .n{color:var(--muted);font-size:12px}.nav.sel .n{color:#dbe8ff}
.sh{font-size:11.5px;font-weight:600;color:var(--muted);padding:16px 10px 6px}
.ch{padding:5px 10px;font-weight:600;display:flex;gap:8px}.ch.q{font-weight:400}.ch b{color:var(--muted);font-weight:400;width:8px}
.ss{padding:5px 10px 5px 30px;display:flex;gap:9px;align-items:center}
.ss.sel{background:var(--blue);color:#fff;border-radius:7px;font-weight:600;margin:0 0 0 20px;padding-left:10px}
.dot{width:8px;height:8px;border-radius:50%;display:inline-block;flex:none}
.g{background:#28c840}.o{background:#ff9500}.gr{background:#b0b0b6}.x{color:#e0342b;font-size:11px;font-weight:700;width:8px;text-align:center}
.main{flex:1;min-width:0;display:flex;flex-direction:column}
.top{height:60px;border-bottom:1px solid var(--line);display:flex;align-items:center;padding:0 28px;gap:12px;flex:none}
.top h1{font-size:17px;font-weight:700}.top .sub{color:var(--muted)}
.sp{flex:1}
.field{border:1px solid #d9d9de;border-radius:7px;padding:6px 10px;color:var(--muted);width:240px;display:flex;justify-content:space-between}
.btn{border:1px solid #d4d4da;background:#fff;border-radius:7px;padding:6px 12px;font-weight:500;white-space:nowrap}
.btn.pri{background:var(--blue);border-color:var(--blue);color:#fff;font-weight:600}
.btn.on{border-color:var(--blue);color:var(--blue);background:#eef4ff}
.btn small{font-weight:400;opacity:.8;font-size:12px}
.bar{display:flex;align-items:center;gap:6px;padding:12px 28px 4px;flex:none}
.chip{border-radius:6px;padding:3px 9px;background:var(--sunken);color:var(--soft);font-size:12.5px;white-space:nowrap}
.chip.on{background:#1c1c1e;color:#fff;font-weight:500}
.chip .c{opacity:.65;margin-left:4px}
.chip.off{background:transparent;border:1px dashed #cfcfd5;color:var(--muted);padding:2px 9px}
.lbl{color:var(--muted);font-size:12px;margin:0 2px 0 10px;white-space:nowrap}.bar{white-space:nowrap}
.ft .lnk{color:var(--blue)!important}
.body{flex:1;display:flex;min-height:0}
.list{flex:1;min-width:0;padding:6px 20px 0 20px}
.add{margin:8px 8px 4px;border:1px solid var(--line);border-radius:9px;padding:9px 12px;color:var(--muted);display:flex;justify-content:space-between}
.add kbd,.field kbd{font:inherit;font-size:11.5px;border:1px solid #d9d9de;border-radius:4px;padding:0 5px;color:var(--muted);background:#fff}
.sec{display:flex;align-items:baseline;gap:8px;padding:14px 8px 4px;font-size:11.5px;font-weight:600;color:var(--muted);letter-spacing:.02em}
.sec .hd{margin-left:auto;display:flex;font-weight:500}
.row{display:flex;align-items:center;gap:10px;padding:7px 8px;border-radius:9px;min-height:50px}
.row.sel{background:#eef4ff}
.row+.row{box-shadow:0 -1px 0 #f0f0f3}.row.sel+.row,.row.sel{box-shadow:none}
.sb{width:22px;height:22px;flex:none}
.v2 .row{gap:11px}
.rk{width:18px;text-align:right;color:#a0a0a6;font-size:12px;font-variant-numeric:tabular-nums;flex:none}
.mid{flex:1;min-width:0}
.l1{display:flex;align-items:center;gap:7px;white-space:nowrap;overflow:hidden}
.ti{font-size:14px;font-weight:550;overflow:hidden;text-overflow:ellipsis}
.due{border-radius:5px;padding:1px 7px;font-size:12px;background:var(--sunken);color:var(--soft)}
.due.over{background:var(--red-soft);color:var(--red);font-weight:600}
.due.soon{background:var(--amber-soft);color:#8a4b00;font-weight:500}
.tag{white-space:nowrap;border-radius:5px;padding:1px 7px;font-size:12px;background:var(--sunken);color:var(--soft)}
.prc{border-radius:5px;padding:1px 7px;font-size:12px;background:#ece9fd;color:#4a2fa8;font-weight:500}
.row.hl{background:#eaf7ee}
.tag.on{background:transparent;box-shadow:inset 0 0 0 1px var(--blue);color:var(--blue)}
.st{font-size:12px;color:var(--muted);font-variant-numeric:tabular-nums}
.l2{margin-top:3px;font-size:12px;color:var(--muted);white-space:nowrap;overflow:hidden;text-overflow:ellipsis;display:flex;gap:6px;align-items:center}
.why{border-radius:5px;padding:0 6px;font-size:11.5px;line-height:17px;background:#fff;box-shadow:inset 0 0 0 1px #dcdce2;color:var(--soft)}
.why.r{box-shadow:inset 0 0 0 1px #f0b6b6;color:var(--red)}
.why.p{box-shadow:inset 0 0 0 1px #b9d0fb;color:var(--blue)}
.ses{border-radius:6px;padding:2px 8px;background:var(--sunken);font-size:12px;display:inline-flex;gap:6px;align-items:center;white-space:nowrap}
.ask{color:var(--amber);font-size:12px;font-weight:600;white-space:nowrap}
.mini{border:1px solid #d4d4da;border-radius:6px;padding:2px 9px;font-size:12px;font-weight:500;background:#fff;white-space:nowrap}
.c-sz{width:110px;flex:none;font-size:12px;color:var(--soft);white-space:nowrap}
.c-sz b{display:inline-block;width:18px;height:18px;line-height:18px;text-align:center;border-radius:5px;background:var(--sunken);font-weight:600;font-size:11px;margin-right:6px}
.c-ch{width:128px;flex:none;font-size:12px;color:var(--muted);white-space:nowrap}
.c-k{width:52px;flex:none;font-size:12px;color:#a0a0a6;text-align:right;font-variant-numeric:tabular-nums}
.props{margin:10px 8px 0;background:#f6f6f8;border-radius:10px;padding:10px 12px 6px}
.prow{display:flex;align-items:center;gap:9px;padding:5px 0;white-space:nowrap}
.prow+.prow{border-top:1px solid #e9e9ee}
.pw i{font-style:normal;color:var(--muted);font-size:12px;margin-left:4px}.pw b{font-weight:600}
.py{flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;color:var(--muted);font-size:12.5px}
.mini.pri{background:var(--blue);border-color:var(--blue);color:#fff}
.narrow .py{display:none}.narrow .pw{flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis}.narrow .pw i{display:none}
.fold{margin:6px 8px 0;padding:9px 0;border-top:1px solid var(--line);color:var(--muted);display:flex;gap:8px;align-items:center}
.fold b{color:var(--soft);font-weight:600}
.narrow .c-ch,.narrow .c-sz,.narrow .hd{display:none}
/* drawer */
.drw{width:540px;flex:none;border-left:1px solid var(--line);padding:18px 26px 0;display:flex;flex-direction:column}
.drw .k{display:flex;justify-content:space-between;color:var(--muted);font-size:12px}
.drw h2{font-size:19px;font-weight:700;margin:4px 0 8px}
.fr{display:flex;align-items:center;min-height:31px}.fr>span:first-child{width:96px;color:var(--muted);flex:none}
.seg{display:inline-flex;background:var(--sunken);border-radius:7px;padding:2px}
.seg i{font-style:normal;padding:3px 11px;border-radius:5px;color:var(--soft);font-size:12.5px}
.seg i.on{background:#fff;box-shadow:0 0 0 1px #d9d9de,0 1px 2px rgba(0,0,0,.08);color:#1c1c1e;font-weight:600}
.inp{border:1px solid #d9d9de;border-radius:7px;padding:4px 10px;min-width:150px}
.hint{color:var(--muted);font-size:12px;margin-left:10px}
.askc{background:var(--amber-soft);border-radius:10px;padding:11px 14px;margin:8px 0 2px}
.askc .t{font-weight:650;display:flex;gap:8px;align-items:center}
.askc .q{color:#5c4a2a;margin:6px 0 11px}
.askc .b{display:flex;gap:8px}
.h3{font-size:11.5px;font-weight:600;color:var(--muted);margin:13px 0 5px;display:flex;justify-content:space-between}
.step{display:flex;gap:9px;align-items:center;padding:3.5px 0}
.cb{width:15px;height:15px;border-radius:4px;border:1.5px solid #b9b9c0;flex:none;display:inline-flex;align-items:center;justify-content:center;font-size:10px;color:#fff}
.cb.d{background:var(--teal);border-color:var(--teal)}.step.d span:last-child{color:var(--muted)}
.desc{color:var(--soft);line-height:1.5}
.hist{color:var(--muted);font-size:12.5px;line-height:1.75}
.foot{margin-top:auto;border-top:1px solid var(--line);padding:12px 0 14px;display:flex;gap:8px;align-items:center}
/* filters popover */
.pop{position:absolute;right:28px;top:54px;width:440px;background:#fff;border:1px solid #d6d6dc;border-radius:12px;box-shadow:0 14px 40px rgba(0,0,0,.16);padding:16px 18px 14px;z-index:5}
.pop .h3:first-child{margin-top:0}
.pr{display:flex;align-items:flex-start;padding:5px 0}.pr>span:first-child{width:84px;color:var(--muted);flex:none;padding-top:3px}
.pr .cs{display:flex;flex-wrap:wrap;gap:5px}
.pop .seg i{padding:2px 9px;font-size:12px}
.pf{border-top:1px solid var(--line);margin-top:12px;padding-top:12px;display:flex;align-items:center;justify-content:space-between;color:var(--muted)}
.na{border-radius:5px;padding:1px 7px;font-size:12px;background:#ffe9c2;color:#7a4300;font-weight:600}
.sugchip{border-radius:5px;padding:1px 7px;font-size:11.5px;border:1px dashed #9b9ba3;color:var(--soft)}
.row.sg{border:1.5px dashed #c4c4cc;margin:3px 0;box-shadow:none!important;background:#fcfcfd}
.row.sg+.row{box-shadow:none}
.acts{display:flex;gap:6px;flex:none;align-items:center}
.sgc{margin:0 8px 8px 100px;background:#f5f5f8;border-radius:9px;padding:8px 12px;display:flex;align-items:center;gap:14px}
.sgc .tx{flex:1;min-width:0}
.sgc .q{font-weight:650;font-size:13.5px}
.sgc .r{color:var(--soft);font-size:12.5px;margin-top:2px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.sgc .r a,.lnk{color:var(--blue);text-decoration:none}
.sgc .src{color:#a0a0a6;font-size:11.5px;margin-left:6px}
.narrow .sgc{margin-left:58px}.narrow .sgc .ev{display:none}
.toast{position:absolute;left:50%;bottom:26px;transform:translateX(-50%);background:#2a2a2e;color:#fff;border-radius:10px;padding:10px 16px;display:flex;gap:16px;box-shadow:0 8px 24px rgba(0,0,0,.25);white-space:nowrap}
.toast a{color:#8ab8ff;font-weight:600}
/* agent inbox */
.ibody{flex:1;padding:16px 28px;display:flex;flex-direction:column;gap:14px;min-height:0}
.card{border:1px solid #dcdce2;border-radius:12px;padding:16px 18px 14px;background:#fff}
.card.first{border-color:#f0b35a;box-shadow:0 0 0 1px #f0b35a}
.card .hd{display:flex;align-items:center;gap:10px}
.card .hd h3{font-size:15.5px;font-weight:700}
.card .hd .who{margin-left:auto;color:var(--muted);font-size:12.5px;white-space:nowrap}
.cols{display:flex;gap:28px;margin-top:10px}
.cols>div{flex:1;min-width:0}
.cl{font-size:11px;font-weight:650;color:var(--muted);letter-spacing:.03em;margin:10px 0 4px}
.cl:first-child{margin-top:0}
.cl small{font-weight:400;letter-spacing:0;font-size:11.5px}
.ft{display:grid;grid-template-columns:66px 1fr;row-gap:3px;background:#f5f5f8;border-radius:8px;padding:9px 12px}
.ft span:nth-child(odd){color:var(--muted)}
.mono{font:12px/1.5 "SF Mono",Menlo,monospace}
.say{color:#2c2c30;line-height:1.5}
.say i{font-style:normal;color:var(--muted)}
.li{display:flex;gap:8px;padding:1.5px 0;line-height:1.45}.li>b{width:14px;flex:none;font-weight:600;text-align:center}
.li .w{color:#b25000}.li .ok{color:var(--teal)}
.cfoot{display:flex;align-items:center;gap:8px;margin-top:14px;padding-top:12px;border-top:1px solid var(--line)}
.cfoot .rule{margin-left:auto;color:var(--muted);font-size:12.5px;white-space:nowrap}
.chk{display:flex;gap:7px;align-items:center;color:var(--soft);margin-left:10px;font-size:12.5px;white-space:nowrap}
.why2{background:#eef4ff;border-radius:9px;padding:10px 13px;margin-top:12px}
.why2 .t{font-weight:650;margin-bottom:3px}
/* session view */
.shead{height:60px;border-bottom:1px solid var(--line);display:flex;align-items:center;padding:0 18px;gap:10px;flex:none}
.shead .cr{color:var(--blue);font-size:12px}.shead .nm{font-size:15px;font-weight:700;display:flex;gap:8px;align-items:center}
.tabs{display:flex;border-bottom:1px solid var(--line);flex:none;background:#fafafb}
.tab{padding:9px 14px;border-right:1px solid var(--line);display:flex;gap:8px;align-items:center}.tab.on{background:#fff;box-shadow:inset 0 -2px 0 var(--blue)}
.term{flex:1;padding:16px 18px;font:12.5px/1.65 "SF Mono",Menlo,monospace;white-space:pre;overflow:hidden;color:#1c1c1e}
.term .m{color:#8a8a90}.term .gn{color:#0f8a3c}.term .am{color:#b86a00}
.prompt{margin:0 18px 8px;border:1px solid #cfcfd5;border-radius:6px;padding:7px 10px;font:12.5px "SF Mono",Menlo,monospace}
.pfoot{margin:0 18px 12px;font:12px "SF Mono",Menlo,monospace;color:#8a8a90}
.panel{width:520px;flex:none;border-left:1px solid var(--line);display:flex;flex-direction:column;min-width:0}
.panel .ph{height:44px;border-bottom:1px solid var(--line);display:flex;align-items:center;padding:0 14px;gap:8px;flex:none}
.panel .ph b{font-size:15px}
.rail{width:76px;flex:none;border-left:1px solid var(--line);padding-top:62px;display:flex;flex-direction:column;gap:18px;align-items:center;font-size:11px;color:var(--soft)}
.rail div{display:flex;flex-direction:column;align-items:center;gap:5px;text-align:center;position:relative}
.rail i{width:18px;height:16px;border:1.4px solid #6a6a70;border-radius:4px;display:block}
.rail .on{color:var(--blue)}.rail .on i{border-color:var(--blue)}
.rail em{position:absolute;top:-7px;right:2px;background:var(--blue);color:#fff;border-radius:8px;font-style:normal;font-size:10px;padding:0 5px;font-weight:600}
.panel .row{gap:8px}.panel .rk{display:none}.panel .sgc{margin-left:60px}
/* home */
.home{flex:1;padding:22px 0;width:900px;margin:0 auto}
.hsec{font-size:12.5px;font-weight:600;color:var(--soft);margin:18px 0 8px}
.hbox{border:1px solid #dcdce2;border-radius:10px}
.hrow{display:flex;align-items:center;gap:12px;padding:11px 16px}.hrow+.hrow{border-top:1px solid var(--line)}
.hrow .t{font-size:14px;font-weight:550}.hrow .t small{font-weight:400;color:var(--muted);font-size:12.5px;margin-left:8px}
.hrow .s{color:var(--muted);font-size:12.5px;margin-top:2px}.hrow .s.a{color:var(--amber);font-weight:500}
.hrow .tm{color:var(--muted);font-size:12.5px}
/* dark */
body.dark{background:#1e1e20;color:#e9e9ec;--line:#343437;--muted:#9a9aa1;--soft:#c4c4ca;--sunken:#2d2d30;--blue:#4a93ff;--amber:#ffb340;--amber-soft:#3b2b10;--red:#ff9088;--red-soft:#4b1f1f;--teal:#35c4b5;--teal-soft:#153a36}
.dark .side{background:#27272a}.dark .srch{background:#3a3a3d}
.dark .btn,.dark .field,.dark .mini,.dark kbd,.dark .add kbd{background:#2d2d30;border-color:#48484c;color:#e9e9ec}
.dark .btn.pri,.dark .nav.sel{background:#2f7cf6;border-color:#2f7cf6;color:#fff}
.dark .chip{background:#2d2d30;color:#c4c4ca}.dark .chip.on{background:#e9e9ec;color:#1e1e20}.dark .chip.off{background:transparent;border-color:#55555a}
.dark .row+.row{box-shadow:0 -1px 0 #2a2a2d}.dark .ti{color:#f2f2f4}
.dark .why{background:transparent;box-shadow:inset 0 0 0 1px #4c4c51;color:#c4c4ca}.dark .why.p{box-shadow:inset 0 0 0 1px #2f5ea8;color:#7fb2ff}
.dark .na{background:#5b3f0c;color:#ffd995}.dark .due.soon{color:#ffcf85}.dark .prc{background:#322c5c;color:#cdc4ff}
.dark .c-sz b{background:#38383b}.dark .rk,.dark .c-k{color:#77777e}
.note{position:absolute;right:14px;top:12px;width:372px;background:rgba(58,58,62,.96);color:#f2f2f4;border-radius:16px;padding:12px 14px;box-shadow:0 12px 36px rgba(0,0,0,.5),inset 0 0 0 1px rgba(255,255,255,.12);display:flex;gap:11px;z-index:20}
.note .ic{width:36px;height:36px;border-radius:9px;background:linear-gradient(#4a93ff,#2f6fe0);flex:none;display:flex;align-items:center;justify-content:center;font-weight:700;color:#fff;font-size:17px}
.note .h{display:flex;justify-content:space-between;font-size:12px;color:#b4b4ba}.note .t{font-weight:650;margin:1px 0 2px}.note .b{color:#d4d4d9;font-size:12.5px;line-height:1.4}
/* settings */
.set{flex:1;padding:8px 28px;max-width:1120px}
.srow{display:flex;align-items:center;padding:7px 0;border-top:1px solid #f0f0f3;gap:14px}.srow .n{flex:1}.srow .n small{display:block;color:var(--muted);font-size:12px;margin-top:1px}
.srow .wh{width:400px;color:var(--muted);font-size:12.5px}
.sgh{font-size:12px;font-weight:650;color:var(--soft);margin:16px 0 6px;display:flex;gap:10px;align-items:baseline}.sgh small{font-weight:400;color:var(--muted)}
/* modal */
.scrim{position:absolute;inset:0;background:rgba(20,20,25,.28);z-index:8}
.sheet{position:absolute;left:50%;top:110px;transform:translateX(-50%);width:640px;background:#fff;border-radius:14px;box-shadow:0 24px 70px rgba(0,0,0,.3);padding:22px 26px 20px;z-index:9}
.sheet h2{font-size:18px;margin-bottom:14px}
.area{border:1px solid #d9d9de;border-radius:8px;padding:9px 11px;line-height:1.5;color:#2c2c30}
"""

PLAY = '<svg class="sb" viewBox="0 0 24 24"><circle cx="12" cy="12" r="9.5" fill="none" stroke="#3a3a3f" stroke-width="1.6"/><path d="M10 8.2v7.6l6-3.8z" fill="#3a3a3f"/></svg>'
PAUSE = '<svg class="sb" viewBox="0 0 24 24"><circle cx="12" cy="12" r="9.5" fill="#fff0d6" stroke="#e08600" stroke-width="1.6"/><path d="M9.6 8.5v7M14.4 8.5v7" stroke="#e08600" stroke-width="2" stroke-linecap="round"/></svg>'
NOCHECK = '<span class="sb"></span>'
CHECK = '<svg class="sb" viewBox="0 0 24 24"><circle cx="12" cy="12" r="9.5" fill="none" stroke="#c9c9cf" stroke-width="1.6"/><path d="M8 12.3l2.8 2.8L16 9.5" fill="none" stroke="#c9c9cf" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/></svg>'
V2 = True   # batch 1 as signed off sets this False; later batches carry the second-opinion fixes
DOTS = {"g": '<i class="dot g"></i>', "o": '<i class="dot o"></i>', "gr": '<i class="dot gr"></i>', "x": '<i class="x">✕</i>'}

def e(s): return html.escape(s)

def I(key, title, ch, due=None, tags=(), steps=None, eff="", imp="", why=(), ses=(), ended=False, pr=False):
    return dict(key=key, title=title, ch=ch, due=due, tags=tags, steps=steps, eff=eff, imp=imp, why=why, ses=ses, ended=ended, pr=pr)

PAY3 = I("PAY-3", "Ledger schema v2", "payments-rewrite", ("soon", "Due tomorrow"), ["migration", "db"], "4/5", "M", "high", ses=[("o", "Ledger entries")])
INF2 = I("INF-2", "Postgres 17 upgrade", "infra", ("over", "Overdue · 2 days"), ["db"], "1/4", "L", "high", ses=[("gr", "Upgrade Postgres 17")], ended=True)
WEB8 = I("WEB-8", "Checkout page shows the new ledger totals", "web", ("", "Fri 27 Nov"), ["checkout"], "2/3", "M", "medium", ses=[("g", "Ledger totals"), ("g", "Totals, another way")])
PROG = [PAY3, INF2, WEB8]
INF6 = I("INF-6", "Rotate the staging database credentials", "infra", ("soon", "Due today"), ["security"], None, "S", "medium", [("p", "Pinned"), ("", "due today"), ("", "small job")])
PAY7 = I("PAY-7", "Refund retry with backoff", "payments-rewrite", ("over", "Overdue · 1 day"), ["refunds", "bug"], None, "S", "high", [("r", "Overdue by a day"), ("", "high impact for a small job"), ("", "PAY-9 waits on it")])
PRR = I("", "Review: Ledger reads behind a flag", "pr-reviews", why=[("", "Waiting on you for 2 days"), ("", "dana-k asked for your review"), ("", "a stack of 2 pull requests")], pr=True)
PAY5 = I("PAY-5", "API rate limiter", "payments-rewrite", ("", "Mon 23 Nov"), ["api"], "0/3", "M", "high", [("", "Due in 4 days"), ("", "high impact")])
TD3 = I("TD-3", "Renew the Apple developer certificate", "No channel", ("", "Mon 14 Dec"), ["admin"], None, "S", "high", [("", "Due in 25 days"), ("", "high impact for a small job")])
NEWI = I("", "Idempotency keys for refund retries", "payments-rewrite", None, ["refunds"], None, "S", "high")
WHY2 = {INF6['title']: ('p', 'Pinned'), PAY7['title']: ('', 'High impact for a small job', 'PAY-9 waits on it'), PRR['title']: ('', 'Waiting on you for 2 days', 'dana-k asked for your review'),
        PAY5['title']: ('', 'High impact'), TD3['title']: ('', 'High impact for a small job')}
NEXT = [INF6, PAY7, PRR, PAY5, TD3]

def row(it, rank=None, sel=False, hl=False, tagon=(), na=None, open_=False):
    """One item row. na: the Needs attention tag's question word, e.g. "close?". open_: draw as open (play) even with no rank."""
    sugg = na == "add?"
    prog = bool(it["ses"]) and not open_
    l1 = f'<span class="ti">{e(it["title"])}</span>'
    if na: l1 += f'<span class="na">Needs attention · {na}</span>'
    if it["pr"]: l1 += '<span class="prc">Pull request review</span><span class="tag">acme/api#483</span><span class="tag">acme/web#210</span><span class="st">by dana-k</span>'
    if it["steps"]: l1 += f'<span class="st">{it["steps"]}</span>'
    if it["due"]: l1 += f'<span class="due {it["due"][0]}">{e(it["due"][1])}</span>'
    l1 += "".join(f'<span class="tag{" on" if t in tagon else ""}">{e(t)}</span>' for t in it["tags"])
    if sugg:
        l2 = '<span class="why">Suggested</span><span>by copilot in Refund retries · not in your list until you say yes</span>'
    elif prog:
        l2 = "".join(f'<span class="ses">{DOTS[d]}{e(n)}</span>' for d, n in it["ses"])
        if it["ended"]: l2 += '<span>session ended</span><span class="mini">Resume</span>'
    else:
        w = it["why"] or [("", "")]
        l2 = f'<span class="why {w[0][0]}">{e(w[0][1])}</span>' + (("<span>" + e(" · ".join(x[1] for x in w[1:])) + "</span>") if w[1:] else "")
    b1 = NOCHECK if sugg else (PAUSE if prog else PLAY)
    if V2:
        b1 = ''
        w2 = WHY2.get(it['title'])
        if w2 and not prog and not sugg: l2 = f'<span class="why {w2[0]}">{e(w2[1])}</span>' + (f'<span>{e(w2[2])}</span>' if len(w2) > 2 else '')
    b2 = NOCHECK if (it["pr"] or sugg) else CHECK
    sz = f'<span class="c-sz"><b>{it["eff"]}</b>{it["imp"]}</span>' if it["eff"] else '<span class="c-sz"></span>'
    cls = "row" + (" sel" if sel else "") + (" hl" if hl else "")
    return (f'<div class="{cls}">{b1}{b2}<span class="rk">{rank or ""}</span>'
            f'<div class="mid"><div class="l1">{l1}</div><div class="l2">{l2}</div></div>'
            f'{sz}<span class="c-ch">{e(it["ch"])}</span><span class="c-k">{it["key"]}</span></div>')


def sidebar(sel="To-do", inbox="2", dot_ledger="o"):
    nav = ""
    for n in ("Home", "Agent inbox", "To-do", "Desk"):
        c = f'<span class="n">{inbox}</span>' if n == "Agent inbox" and inbox else ""
        nav += f'<div class="nav{" sel" if n == sel else ""}">{n}{c}</div>'
    def ss(dot, name):
        return f'<div class="ss{" sel" if name == sel else ""}">{DOTS[dot]}{name}</div>'
    return f"""<div class="side">
<div class="lights"><i style="background:#ff5f57"></i><i style="background:#febc2e"></i><i style="background:#28c840"></i></div>
<div class="srch"><span>Search</span><span>⌘K</span></div>
{nav}
<div class="sh">Channels</div>
<div class="ch"><b>⌄</b>payments-rewrite</div>
{ss(dot_ledger, "Ledger entries")}{ss("g", "Refund retries")}{ss("gr", "API rate limiter")}
<div class="ch"><b>⌄</b>infra</div>
{ss("x", "Upgrade Postgres 17")}{ss("g", "Benchmark BRIN")}
<div class="ch"><b>⌄</b>web</div>
{ss("g", "Ledger totals")}{ss("g", "Totals, another way")}
<div class="ch{"" if sel == "Ledger reads behind a flag" else " q"}"><b>{"⌄" if sel == "Ledger reads behind a flag" else "›"}</b>pr-reviews</div>
{ss("g", "Ledger reads behind a flag") if sel == "Ledger reads behind a flag" else ""}
<div class="sh">Hosts</div>
<div class="ss" style="padding-left:10px"><i class="dot g"></i>This Mac</div><div class="ss" style="padding-left:10px"><i class="dot g"></i>devbox-1</div><div class="ss" style="padding-left:10px"><i class="dot g"></i>gpu-box</div>
<div class="nav{" sel" if sel == "Settings" else ""}" style="margin-top:auto">Settings</div></div>"""

def top(sub, nfilt=0, nxt="INF-6"):
    f = f'<span class="btn on">Filters · {nfilt}</span>' if nfilt else '<span class="btn">Filters</span>'
    return (f'<div class="top"><h1>To-do</h1><span class="sub">{sub}</span><span class="sp"></span>'
            f'<span class="field"><span>Search</span><kbd>/</kbd></span><span class="btn">Sort: Ranked ⌄</span>{f}'
            f'<span class="btn pri">Do next <small>· {nxt}</small></span></div>')

def bar(on="All", extra="", nsug=0, states=(3, 5, 2, 3), show=("p", "o"), counts=None, closed=False):
    if V2:
        cs = counts or [("All", "10"), ("payments-rewrite", "4"), ("infra", "2"), ("web", "2"), ("pr-reviews", "1"), ("No channel", "1")]
        ch = "".join(f'<span class="chip{" on" if n == on else ""}">{n}<span class="c">{c}</span></span>' for n, c in cs)
        cl_ = '<span class="chip on" style="background:#dff5f1;color:#0f5a54">Closed shown ✕</span>' if closed else '<span class="chip off">Show closed</span>'
        return f'<div class="bar">{ch}{extra}<span class="sp"></span>{cl_}</div>'
    cs = counts or [("All", "10"), ("payments-rewrite", "4"), ("infra", "2"), ("web", "2"), ("pr-reviews", "1"), ("No channel", "1")]
    ch = "".join(f'<span class="chip{" on" if n == on else ""}">{n}<span class="c">{c}</span></span>' for n, c in cs)
    sg = f'<span class="chip" style="margin-left:10px;background:#fff;box-shadow:inset 0 0 0 1px #d4d4da">Suggestions<span class="c">{nsug}</span></span>' if nsug else ""
    def st(k, name, n, style):
        return f'<span class="chip on" style="{style}">{name}<span class="c">{n}</span></span>' if k in show else f'<span class="chip off">{name}<span class="c">{n}</span></span>'
    return (f'<div class="bar">{ch}{sg}{extra}<span class="sp"></span><span class="lbl">Show</span>'
            + st("p", "In progress", states[0], "background:#fff0d6;color:#8a4b00") + st("o", "Open", states[1], "background:#e9e9ee;color:#1c1c1e")
            + st("w", "Waiting", states[2], "background:#e9e9ee;color:#1c1c1e") + st("l", "Later", states[3], "background:#e9e9ee;color:#1c1c1e")
            + st("c", "Closed", "", "background:#dff5f1;color:#0f5a54") + '</div>')

HD = '<span class="hd"><span class="c-sz">Effort · Impact</span><span class="c-ch">Channel</span><span class="c-k"></span></span>'
ADD = '<div class="add"><span>Add an item… try “fix login redirect fri #bug”</span><kbd>N</kbd></div>'
FOLDS = ('<div class="fold">›<b>Waiting on another item · 2</b><span>PAY-9 Backfill progress bar waits on PAY-7 · WEB-13 waits on PAY-3</span></div>'
         '<div class="fold" style="margin-top:0">›<b>Later · 3</b><span>Next to come back: “Check the Stripe fee change” on Mon 23 Nov</span></div>')

def sec(t, extra=""): return f'<div class="sec">{t}{extra}</div>'

FOLDS2 = ('<div class="fold">›<b>Waiting on another item · 2</b><span>PAY-9 Backfill progress bar waits on PAY-7 · WEB-13 waits on PAY-3</span></div>'
          '<div class="fold" style="margin-top:0">›<b>Hidden until a date · 3</b><span>Next to come back: “Check the Stripe fee change” on Mon 23 Nov</span></div>')

def page(name, inner, sel="To-do", inbox="2", dark=False, raw=False, **kw):
    body = inner if raw else f'<div class="main" style="position:relative">{inner}</div>'
    doc = f'<!doctype html><meta charset="utf-8"><title>{name}</title><style>{CSS}</style><body class="{"dark" if dark else ""}">{sidebar(sel, inbox, **kw)}{body}'
    open(os.path.join(HERE, name + ".html"), "w").write(doc)

# ---- Agent inbox
def card(title, who, left, right, rule, always, first=True, extra="", buttons=None, dot="o", open_link=True):
    b = buttons or '<span class="btn pri" style="min-width:64px;text-align:center">Yes</span><span class="btn" style="min-width:64px;text-align:center">No</span><span class="btn" style="margin-left:8px">Ask why</span>'
    ol = ' &nbsp; <span class="lnk">Open session ↗</span>' if open_link else ''
    al = f'<span class="chk"><span class="cb"></span>{always}</span>' if always else ""
    return (f'<div class="card{" first" if first else ""}"><div class="hd">{DOTS[dot]}<h3>{title}</h3><span class="who">{who}{ol}</span></div>'
            f'<div class="cols"><div>{left}</div><div>{right}</div></div>{extra}'
            f'<div class="cfoot">{b}{al}<span class="rule">{rule}</span></div></div>')

def cl(t, small=""): return f'<div class="cl">{t}{" <small>" + small + "</small>" if small else ""}</div>'
def li(mark, text, cls=""): return f'<div class="li"><b class="{cls}">{mark}</b><span>{text}</span></div>'
def rule(text): return f'Asked because <b style="color:var(--soft);font-weight:500">{text}</b> is set to Ask me · <span class="lnk">Change this rule</span>'

def inbox_top(n, on="All", chans=(("All", ""),)):
    ch = "".join(f'<span class="chip{" on" if c == on else ""}">{c}<span class="c">{k}</span></span>' for c, k in chans)
    sub = f"{n} waiting on you" if n else "nothing is waiting on you"
    return (f'<div class="top"><h1>Agent inbox</h1><span class="sub">{sub}</span><span class="sp"></span><span class="lnk">History</span></div>'
            + (f'<div class="bar">{ch}<span class="sp"></span><span class="lbl">Agents that are stopped until you answer. Changes to your to-dos wait in To-do instead.</span></div>' if n else ""))

# ---- Session view
def session(name, channel, tabs, term, panel, rail_on, agent="Claude Code · devbox-1", dot="g", badge=None, prompt="", foot="▸▸ accept edits on (shift+tab to cycle)"):
    tb = "".join(f'<span class="tab{" on" if i == 0 else ""}">{DOTS["g"] if i == 0 else ">_"} {t} ✕</span>' for i, t in enumerate(tabs)) + '<span class="tab">+</span>'
    rl = ""
    for r in ("Agent inbox", "Diff", "Files", "To-do", "Journal", "Info"):
        bd = f'<em>{badge[1]}</em>' if badge and badge[0] == r else ""
        rl += f'<div class="{"on" if r == rail_on else ""}"><i></i>{r.replace("Agent inbox", "Agent<br>inbox")}{bd}</div>'
    return (f'<div class="main"><div class="shead"><div><div class="cr"># {channel} ›</div><div class="nm">{DOTS[dot]}{name}</div></div><span class="sp"></span><span style="color:var(--muted)">{agent} &nbsp; ⑂ &nbsp; ⤢ &nbsp; ⋯</span></div>'
            f'<div class="tabs">{tb}</div><div class="term">{term}</div><div class="prompt">&gt; {prompt}▌</div><div class="pfoot">{foot}</div></div>'
            f'<div class="panel">{panel}</div><div class="rail">{rl}</div>')

# ---- shared by batches 4 on
YESNO = '<span class="btn pri" style="min-width:64px;text-align:center">Yes</span><span class="btn" style="min-width:64px;text-align:center">No</span>'
NOTE = lambda t: f' <span style="font-weight:400">{t}</span>'

def todo_list(narrow=False, sel=None, na=True, folds=True, add=ADD, prog3=False):
    s = add
    if na:
        s += (sec("NEEDS ATTENTION · 3", "" if narrow else NOTE("an agent wants to change these; open one to say yes or no") + HD)
              + row(PAY3, sel=sel == "PAY-3", na="close?") + row(INF2, sel=sel == "INF-2", na="new due date?") + row(NEWI, sel=sel == "NEW", na="add?"))
        s += sec("IN PROGRESS · 1") + row(WEB8)
    else:
        s += sec("IN PROGRESS · 3", HD) + row(PAY3) + row(INF2) + row(WEB8)
    s += sec("NEXT · 5", "" if narrow else NOTE("ranked for you; the order holds still while you look"))
    s += "".join(row(it, i + 1, sel=(sel == "PR" and it is PRR)) for i, it in enumerate(NEXT))
    return s + (FOLDS2 if folds else "")

def ask(q, reason, rows, extra_btns='<span class="btn">Ask why</span><span class="btn">Open session</span>'):
    grid = "".join(f'<span>{a}</span><span>{b}</span>' for a, b in rows)
    return (f'<div class="askc"><div class="t" style="font-size:14.5px">{q}</div>'
            f'<div class="q" style="color:#3a2c12;margin:5px 0 7px">{reason}</div>'
            f'<div class="ft" style="background:#fff;grid-template-columns:74px 1fr;padding:8px 11px;margin-bottom:8px">{grid}</div>'
            f'<div class="b">{YESNO}<span class="sp"></span>{extra_btns}</div></div>')

def fr(label, val): return f'<div class="fr"><span>{label}</span>{val}</div>'
def seg(opts, on): return '<span class="seg">' + "".join(f'<i class="{"on" if o == on else ""}">{o}</i>' for o in opts) + '</span>'
ADDL = '<span class="hint" style="color:var(--blue)">+ Add</span>'

def drawer(meta, title, body, foot, width=540):
    return (f'<div class="drw" style="width:{width}px"><div class="k"><span>{meta}</span><span>⤢ &nbsp; ✕</span></div><h2>{title}</h2>{body}'
            f'<div class="foot">{foot}</div></div>')

FOOT = ('<span class="btn">Start a session on this</span><span class="sp"></span><span style="color:var(--muted)">History</span>'
        '<span style="color:var(--muted);margin-left:14px">☆ Pin</span><span style="color:var(--muted);margin-left:14px">Hide until…</span>')

def steps(done, todo, by=""):
    n = len(done) + len(todo)
    s = f'<div class="h3"><span>STEPS · {len(done)} OF {n}</span><span style="font-weight:400">{by}</span></div>'
    s += "".join(f'<div class="step d"><span class="cb d">✓</span><span>{x}</span></div>' for x in done)
    s += "".join(f'<div class="step"><span class="cb"></span><span>{x}</span></div>' for x in todo)
    return s + '<div class="step" style="color:var(--muted)"><span class="cb" style="border-style:dashed"></span><span>Add a step…</span></div>'

def todo_page(name, sub, lst, right="", over="", narrow=False, barkw=None, nxt="INF-6", **kw):
    page(name, top(sub, nxt=nxt) + bar(**(barkw or {})) + f'<div class="body"><div class="list{" narrow" if narrow else ""}">{lst}</div>{right}</div>{over}', **kw)
