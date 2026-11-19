from cards import *

# 1: Agent access
def srow(name, sub, on, where, opts=("Allow", "Ask", "Never")):
    return f'<div class="srow"><div class="n">{name}<small>{sub}</small></div>{seg(opts, on)}<span class="wh">{where}</span></div>'
TD = "Asks on the item in To-do; the agent carries on."
AI = "Asks in the Agent inbox; the agent stops and waits."
tabs = '<div class="bar" style="padding-top:10px">' + "".join(f'<span class="chip{" on" if t == "Agent access" else ""}">{t}</span>' for t in ("General", "Agents", "Agent access", "Machines", "Notifications")) + '</div>'
body = ('<div class="set">'
        '<div class="sgh">TO-DO <small>never stops an agent and never notifies you</small></div>'
        + srow("Tick steps and edit items it made", "", "Allow", "")
        + srow("Add an item", "It appears under Needs attention, marked Suggested", "Ask", TD)
        + srow("Close or edit an item it didn’t make", "State, due date, effort, impact, tags, what it waits on", "Ask", TD)
        + srow("Delete an item", "", "Never", "The agent is told no, with this rule named.")
        + '<div class="sgh">FILES AND SESSIONS <small>risky: the agent stops until you answer, and you get a notification</small></div>'
        + srow("Move or copy files inside its own session", "", "Allow", "")
        + srow("Copy files from another session", "", "Ask", AI)
        + srow("Move files or its session to another machine", "Running terminals move along and keep running", "Ask", AI)
        + srow("Start or fork a session", "", "Ask", AI)
        + srow("End a session or a terminal", "Only you end them", "Never", "The agent is told no, with this rule named.")
        + '<div class="sgh">ACCESS AND QUESTIONS</div>'
        + srow("Read other channels’ to-dos and files", "", "Allow", "")
        + srow("Ask for access it doesn’t have", "", "Ask", AI, ("Ask", "Never"))
        + srow("Ask you a question", "", "Ask", AI, ("Ask",))
        + '<div style="margin-top:14px;padding:11px 14px;background:#f5f5f8;border-radius:9px;color:var(--soft);line-height:1.5">Shell commands, edits and pushes are asked by each agent in its own terminal, as that agent is set up. Humata doesn’t repeat those asks here. '
          'Rules you made with “Always allow” are listed under their row. A channel can override these in its Agent access tab.</div></div>')
page("b6-1-agent-access", '<div class="top"><h1>Settings</h1></div>' + tabs + body, sel="Settings", inbox="")

# 2: dark, with the one kind of notification
import lib
note = ('<div class="note"><div class="ic">H</div><div style="flex:1"><div class="h"><span>HUMATA</span><span>now</span></div>'
        '<div class="t">codex is stopped: move Benchmark BRIN to gpu-box?</div>'
        '<div class="b">Its 2 terminals would move along and keep running. Open the Agent inbox to say yes or no.</div></div></div>')
todo_page("b6-2-dark-notification", "10 open across 4 channels", todo_list(), over=note, dark=True, inbox="1")

# 3: the migration message
mig = card("Move your boards into the one To-do list?", "Humata 0.4 · This Mac and devbox-1",
           cl("WHAT CHANGES")
           + ft([("Boards", "1 global, 4 channel and 6 session boards → <b>one list</b>, filtered by channel and session"),
                 ("Items", "37 items keep their keys, titles, descriptions, sessions and history"),
                 ("Columns", "Backlog 9 and To do 11 → <b>Open</b> · In progress 6 and In review 2 → <b>In progress</b> · Done 9 → <b>Closed</b>"),
                 ("Order", "Your order in each column is kept as the starting rank"),
                 ("Marks", "3 VIP items become <b>pinned</b>")])
           + cl("ALSO MOVES") + '<div>The Inbox becomes the Agent inbox. 2 waiting board asks become Needs attention on their items; the 1 waiting file ask stays.</div>',
           cl("WHAT ELSE IT AFFECTS", "worked out by Humata")
           + li("✓", "<b>5 running sessions and their terminals keep running</b>", "ok")
           + li("✓", "Both machines migrate together; neither is left on the old boards", "ok")
           + li("!", "8 items on session boards move to their channel, with that session attached", "w")
           + li("!", "The Board, Table and Dashboard views and the Scope picker go away", "w")
           + li("!", "Agents’ board tools are replaced; running agents get the new ones on their next call", "w")
           + cl("CAN IT BE UNDONE?") + '<div>Yes, for 30 days: Settings › General › Restore boards brings back yesterday’s boards. Changes made after migrating are not carried back.</div>',
           "Humata can’t open To-do or the old boards until this has run", "", buttons='<span class="btn pri">Migrate</span><span class="btn">What’s new in To-do</span>', dot="o", open_link=False)
page("b6-3-migrate-message", inbox_top(1, chans=(("All", "1"),)).replace("Agents that are stopped until you answer. Changes to your to-dos wait in To-do instead.", "An update needs you once. No agent is stopped.")
     + f'<div class="ibody">{mig}</div>', sel="Agent inbox", inbox="1", dot_ledger="g")
