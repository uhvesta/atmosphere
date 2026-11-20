# Frames: TODO2WIRE, a step's context and the To-do settings

HTML mockups at 1600x950, not renders of the app. Sources are in `src/` (`sh src/shoot.sh` rebuilds and screenshots all four; it reuses the look in `../boardsimple/src/lib.py`). "Today" in the frames is Thu 19 Nov.

## A step's context, edited in the item drawer
- `t2-0-step-hover.png`: I open "Ledger schema v2" and move the pointer over its last step. I see "+ Context" appear before the step's ↑ and Drop; the third step already has a context and shows it as the grey line under it, as before. Shows: nothing new is drawn until I point at a step; a step that has a context has no "+ Context", because its grey line is the way in.
- `t2-1-step-context-editing.png`: I press "+ Context". I see a text box under the step, indented to the step's text, with Save, Cancel and "⌘↩ saves · Esc cancels". Return starts a new line. Shows: a context can run over several lines; the step's own row doesn't move.
- `t2-2-step-context-existing.png`: I click the third step's grey line instead. I see the same box in its place, holding what was there, and the hint adds "empty removes it". Shows: editing and removing are the same control; there is no separate Remove.

## Settings › General › To-do
- `t7-1-settings-todo.png`: In Settings › General, under Fast mode, I see a To-do group: what happens when a pull request merges and when an item closes, each Ask or Auto; whether the agent is woken on failing checks and on review comments; and after how many days an idle item nudges me. Shows: Ask is the default and Auto says it comes with Undo; a session with unsaved work or a running terminal is never archived; the section list on the left is unchanged.
