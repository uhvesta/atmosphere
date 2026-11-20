from lib import *

PICK = {"Ledger schema migration", "Refund retries", "Upgrade Postgres 17"}
AUTO = {"Try BRIN index"}
side = sidebar("Home", PICK, AUTO, bar=sidebar_bar(4, "3 picked · 1 fork goes along"))
home = toolbar("Home", btn("Select") + btn("New session", "pri")) + home_body()

# m1-1: the sidebar in select mode
page("m1-1-sidebar-select", side, home)

TRASH = ("The terminals end for good. The sessions go to the trash on This Mac and devbox-1 for 30 days: restore one from "
         "the machine’s Storage &gt; Trash and it comes back archived, with its work. There is no Undo.")

def picked(stale=False):
    led_t = "Ends 3 running terminals: claude (waiting for you), zsh, zsh (new)." if stale else "Ends 2 running terminals: claude (waiting for you), zsh."
    ref_w = "api: 3 commits on uhvesta/refund-retries never pushed (1 new)." if stale else "api: 2 commits on uhvesta/refund-retries never pushed."
    pay = [srow("Ledger schema migration", "Claude Code · devbox-1", "live",
                [("t", led_t), ("w", "ledger: 3 uncommitted files on uhvesta/ledger-schema, never pushed.")], tick=True, changed=stale),
           srow("Try BRIN index", "Claude Code · devbox-1", "live",
                [("q", "Goes with Ledger schema migration: this fork works in the same folder."), ("t", "Ends 1 running terminal: claude (waiting for you).")], fork=True, sub=True),
           quiet("Partial BRIN on posted_at and Review the schema stay. These forks have their own folders; they move up in the tree."),
           srow("Refund retries", "Claude Code · This Mac", "live",
                [("t", "Ends 1 running terminal: claude (working)."), ("w", ref_w)], tick=False, changed=stale)]
    inf = [srow("Upgrade Postgres 17", "Claude Code · devbox-1", "live", [("q", "Nothing running. Nothing unpushed.")])]
    return group("# payments-rewrite", "3 of 6 sessions", pay) + group("# infra", "1 of 2 sessions", inf)

SUM = [("term", ORANGE, "4 running terminals end, in 3 sessions."),
       ("branch", ORANGE, "2 sessions hold work that was never pushed."),
       ("fork", "var(--text)", "1 fork goes with its parent. 2 forks stay.")]
# m1-2: the plan for the picked set
page("m1-2-plan-picked", side, home, sheet(
    "Delete 4 sessions?", "In 2 channels, on This Mac and devbox-1.", SUM, picked(), TRASH,
    "Tick “Delete that work too” for Refund retries to continue", btn("Delete 4 sessions", "dng off"), top=56))

# m1-3: delete all, nothing typed yet
def everything(scrolled=False, ticks=(True, False, False)):
    pay = [srow("Ledger schema migration", "Claude Code · devbox-1", "live",
                [("t", "Ends 2 running terminals: claude (waiting for you), zsh."), ("w", "ledger: 3 uncommitted files on uhvesta/ledger-schema, never pushed.")], tick=ticks[0]),
           srow("Try BRIN index", "Claude Code · devbox-1", "live", [("t", "Ends 1 running terminal: claude (waiting for you).")], fork=True, sub=True),
           srow("Partial BRIN on posted_at", "Claude Code · devbox-1", "live", [("t", "Ends 1 running terminal: claude (working).")], fork=True, sub=True),
           srow("Review the schema", "Claude Code · This Mac", "live", [("t", "Ends 1 running terminal: claude (idle).")], fork=True, sub=True),
           srow("Refund retries", "Claude Code · This Mac", "live",
                [("t", "Ends 1 running terminal: claude (working)."), ("w", "api: 2 commits on uhvesta/refund-retries never pushed.")], tick=ticks[1]),
           srow("API rate limiter", "Codex · devbox-1", "live", [("t", "Ends 1 running terminal: codex (idle).")]),
           quiet("2 archived, nothing unpushed: Currency codes, Webhook retries")]
    inf = [srow("Benchmark BRIN", "Codex · devbox-1", "live",
                [("t", "Ends 1 running terminal: codex (working)."), ("w", "infra: 1 commit on uhvesta/bench-brin not pushed.")], tick=ticks[2]),
           quiet("4 more, nothing running and nothing unpushed: Upgrade Postgres 17 and 3 archived · ", "Show")]
    prr = [srow("acme/api #482", "Claude Code · This Mac", "live", [("t", "Ends 1 running terminal: claude (idle).")]),
           quiet("3 archived, nothing unpushed: acme/api #455, acme/api #461, acme/ledger #88")]
    web = [quiet("6 archived, nothing unpushed · ", "Show")]
    off = (f'<div class="gh">{st("off")}<span style="font-weight:600">gpu-box</span><span class="sec">can’t reach, left alone · 4 sessions</span></div>'
           + quiet("Not deleted: Train reranker, Eval sweep (# infra), Render farm probe, Font subsetting (# web). Delete them when gpu-box is back."))
    body = (group("# payments-rewrite", "8 sessions", pay) + group("# infra", "5 sessions", inf)
            + group("# pr-reviews", "4 sessions", prr) + group("# web", "6 sessions", web) + off)
    return body

SUMALL = [("term", ORANGE, "9 running terminals end, in 8 sessions."),
          ("branch", ORANGE, "3 sessions hold work that was never pushed."),
          ("dot", "var(--secondary)", "gpu-box can’t be reached. Its 4 sessions are left alone.")]
DESK = '<div class="ter" style="font-size:11px;padding-top:6px">Desk sessions are not part of this. The Desk has its own Clean up.</div>'
SUBALL = "Every session, live and archived: 23 in 4 channels, on This Mac and devbox-1."
home_all = toolbar("Home", btn("Select") + btn("New session", "pri")) + home_body()
page("m1-3-plan-all-empty", sidebar("Home"), home_all, sheet(
    "Delete all 23 sessions?", SUBALL, SUMALL, everything(), TRASH,
    "Tick “Delete that work too” for 2 sessions and type 23 to continue", btn("Delete all 23 sessions", "red off"),
    top=22, typed=("Type the number of sessions, 23, to confirm", ""), list_style="height:296px", thumb=(4, 118), after_list=DESK))

# m1-4: the plan went stale
STALE = ('<div class="stale"><b>Something changed since this plan. Nothing was deleted.</b>'
         '<span>Refund retries: 1 new commit on uhvesta/refund-retries.</span>'
         '<span>Ledger schema migration: a new terminal started (zsh).</span>'
         '<span>This is the new plan. Check it and delete again.</span></div>')
SUM2 = [("term", ORANGE, "5 running terminals end, in 3 sessions."), SUM[1], SUM[2]]
page("m1-4-plan-stale", side, home, sheet(
    "Delete 4 sessions?", "In 2 channels, on This Mac and devbox-1.", SUM2, picked(True), TRASH,
    "Tick “Delete that work too” for Refund retries to continue", btn("Delete 4 sessions", "dng off"), top=14, stale=STALE))
