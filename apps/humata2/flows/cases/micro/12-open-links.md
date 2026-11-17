# ⌘-click opens a link, a plain click does not, and a double-click copies a path

Layer: micro. Platforms: macOS, Linux.

## Starting state

- Humata 2 is open on this Mac, with a shell tab in the session `ledger`.
- A test runner has printed three things in that tab:
  - a plain URL, `https://example.com/report`, on its own row;
  - a path with a line number, `src/parse.rs:42: error`;
  - an OSC 8 link whose label reads `open the report` and whose target is `https://example.com/report/full`.
- The mouse is over the window, with no button held.
- No full-screen program is running, so mouse reporting is off.

## Steps

1. **I do:** I ⌘-click the plain URL. On Linux I Ctrl-click it.
   **I see:** The URL is underlined when the pointer is over it. The default browser opens the page.
   **State and events:** The pane raises an open-link event for the URL (`terminal/pane/src/host.rs:248-253`). The app should open the URL. Nothing is sent to the shell.
   **Status:** Likely bug. The pane raises the event, but the app's terminal subscription has no arm for it. `ui/src/terminals.rs:178-211` handles focus, split, clipboard write, failure and input-uncertain events. Everything else falls into `_ => {}` at `ui/src/terminals.rs:211`, which includes `PaneEvent::OpenLink`. So in the app the click opens nothing. Confirm: ⌘-click an https link in a session tab and see that no browser opens. Add a log in the `_ => {}` arm, or in the pane's `open_link`, and see the event fire and get dropped. The pane side is tested: `terminal/pane/tests/links.rs::modifier_click_opens_explicit_and_plain_links_without_mouse_reports` (Fakes only).

2. **I do:** I plain-click the same URL, with no modifier.
   **I see:** The click places the cursor or starts a selection. No browser opens.
   **State and events:** Without the modifier, the mouse-down goes to the normal path (`terminal/view/src/view.rs:975-985`). No open-link event is raised.
   **Status:** Code, no test. `terminal/view/src/view.rs:975-985` only opens a link when the modifier is held. `terminal/pane/tests/links.rs` checks modifier clicks only and never checks that a plain click does not open.

3. **I do:** I double-click `parse.rs` in the path `src/parse.rs:42`, then paste.
   **I see:** The paste shows `src/parse.rs`. The `:42` is not copied.
   **State and events:** The word selection stops at `:` because `:` is a boundary character (`terminal/view/src/selection.rs:10`). The selection covers only the word, and the colon and the number are separate words.
   **Status:** Likely bug. `terminal/view/src/selection.rs:10` lists `:` among the boundary characters, so a path with a line number copies without the line number. The no-colon case is covered by `terminal/pane/tests/pane.rs::a_double_click_copies_a_whole_path` (Fakes only). Confirm: double-click `parse.rs` in the path, paste, and see `src/parse.rs`. Decide whether the line number belongs in the copy. If it does, the boundary rule needs a change for `:` followed by digits.

4. **I do:** I ⌘-click the OSC 8 link labelled `open the report`.
   **I see:** The browser should open `https://example.com/report/full`, the target, not the label. In the app, nothing opens.
   **State and events:** The link's hidden target is read from the cell (`terminal/view/src/view.rs:981-983`). The pane raises an open-link event with the target. The app drops it, as in step 1.
   **Status:** Likely bug. The target choice is proven at the pane and emulator level: `terminal/pane/tests/links.rs::modifier_click_opens_explicit_and_plain_links_without_mouse_reports` (first row, `hidden` target, Fakes only) and `terminal/vt/tests/links.rs::osc8_uses_hidden_uri_and_survives_scrolling_and_resize` (real engine, no shell). The app drop at `ui/src/terminals.rs:211` is the same bug as in step 1.

## Branches

### A link to a file:// target, or a path on a remote machine

- **When:** The OSC 8 target is `file:///…`, or the link points at a path on another machine. A plain path with no OSC 8 is never a link.
- **I do:** I ⌘-click the link. Then I double-click a plain path on the remote machine.
- **I see:** The file link does nothing in the app. In the lab, the person is told: "This link cannot be opened here. Copy its address and open it in an appropriate application." The plain path is not underlined and does not open. Double-click selects it, the same as step 3.
- **State and events:** Plain-text links are recognised only for `https`, `http` and `mailto` (`terminal/grid/src/links.rs:60-63`). Open-link events are refused unless the scheme is `https`, `http` or `mailto` (`terminal/lab/src/lib.rs:379-390`, `terminal/client/src/lib.rs:453`). The app does not handle the event at all (`ui/src/terminals.rs:211`).
- **Expected outcome:** Humata does not open a file link, and tells the person why. A plain path is not treated as a link.
- **Status:** Code, no test. No test checks the refusal or the message. The lab branch at `terminal/lab/src/lib.rs:379-390` has no test, and the client's branch at `terminal/client/src/lib.rs:453` refuses silently.

### An odd scheme, such as `javascript:` or `ssh://`

- **When:** An OSC 8 link uses a scheme other than `https`, `http` or `mailto`.
- **I do:** I ⌘-click the link.
- **I see:** Expected: the link is refused, and the lab says so with the message above. Actual in the app: nothing.
- **State and events:** The check is case-insensitive and also rejects control characters in the address (`terminal/lab/src/lib.rs:380-388`, `terminal/client/src/lib.rs:453`).
- **Expected outcome:** Only `https`, `http` and `mailto` open. Anything else is refused and the person is told.
- **Status:** Code, no test. The scheme check exists in two places, and neither has a test.

### A link split across a wrap

- **When:** A URL is printed so that it wraps onto the next row.
- **I do:** I ⌘-click the URL on either row.
- **I see:** The whole URL is underlined across both rows. In the app, nothing opens. Expected: the browser opens the whole URL.
- **State and events:** The plain-link finder joins soft-wrapped rows before it reads the URL (`terminal/grid/src/links.rs:6-22`).
- **Expected outcome:** The whole URL is found from either row, and opens as one URL.
- **Status:** Fakes only. `terminal/vt/tests/links.rs::plain_urls_support_wrapping_unicode_and_balanced_punctuation` checks that both rows report the same URL, in the real engine with no shell. The open step through the pane is not tested, and the app-side drop is the bug above.

### A link in a TUI with the mouse captured

- **When:** A TUI such as `htop` has turned on mouse reporting.
- **I do:** I ⌘-click a URL the TUI printed.
- **I see:** The link opens, and the TUI gets no mouse report for the click. In the app, nothing opens (see step 1).
- **State and events:** The link check runs before the mouse report is built (`terminal/view/src/view.rs:975-985`). When the modifier is held, the click is not reported to the program.
- **Expected outcome:** The link opens, and the program receives no click.
- **Status:** Fakes only. `terminal/pane/tests/links.rs::modifier_click_opens_explicit_and_plain_links_without_mouse_reports` checks that the open event fires and that nothing is sent to the fake PTY. It does not check the app.

## Expected outcomes

- [ ] ⌘-click on a plain URL opens it in the browser.
- [ ] A plain click on a URL does not open it.
- [ ] Double-click copies the whole word, and a path with a line number is copied with its line number.
- [ ] ⌘-click on an OSC 8 link opens its target, not its label.
- [ ] A file link or an unknown scheme is refused, and the person is told why.
- [ ] A URL that wraps across rows opens as one URL.
- [ ] A ⌘-click in a TUI with the mouse on opens the link and sends no mouse report.

## Status summary

| Step or branch | Status | Evidence |
| --- | --- | --- |
| 1. ⌘-click a plain URL | Likely bug | `ui/src/terminals.rs:211` drops `PaneEvent::OpenLink`; pane side in `terminal/pane/tests/links.rs::modifier_click_opens_explicit_and_plain_links_without_mouse_reports` |
| 2. Plain click does not open | Code, no test | `terminal/view/src/view.rs:975-985` |
| 3. Double-click a path with a line number | Likely bug | `terminal/view/src/selection.rs:10` (`:` is a boundary) |
| 4. ⌘-click an OSC 8 link | Likely bug | `ui/src/terminals.rs:211`; target proven by `terminal/pane/tests/links.rs` and `terminal/vt/tests/links.rs::osc8_uses_hidden_uri_and_survives_scrolling_and_resize` |
| Branch: file:// or remote path | Code, no test | `terminal/grid/src/links.rs:60-63`, `terminal/lab/src/lib.rs:379-390` |
| Branch: odd scheme | Code, no test | `terminal/client/src/lib.rs:453`, `terminal/lab/src/lib.rs:380-388` |
| Branch: link split across a wrap | Fakes only | `terminal/vt/tests/links.rs::plain_urls_support_wrapping_unicode_and_balanced_punctuation` |
| Branch: TUI with the mouse captured | Fakes only | `terminal/pane/tests/links.rs::modifier_click_opens_explicit_and_plain_links_without_mouse_reports` |

## Gaps and likely bugs

- Likely bug: the app does not open terminal links at all. `ui/src/terminals.rs:178-211` has no arm for `PaneEvent::OpenLink`, and the catch-all at `ui/src/terminals.rs:211` drops it. The open-link code exists only in the standalone client (`terminal/client/src/lib.rs:453`) and in the lab (`terminal/lab/src/lib.rs:379-390`). `terminal/pane/tests/links.rs` proves only that the pane raises the event. `TERM.md` line 9 promises that links "work as in C1's demo", and the demo is the standalone client, so the app dropped the behaviour when it was rebuilt.
- Likely bug: the same catch-all at `ui/src/terminals.rs:211` drops bells, title changes and working-directory changes too. The pane raises those events (`terminal/pane/tests/pane.rs::titles_bells_and_clipboard_writes_are_told_to_the_app` covers title and bell at the pane level). Confirm by sending a bell (`printf '\a'`) in the app and seeing no sound or badge.
- Likely bug: `:` is a word boundary (`terminal/view/src/selection.rs:10`), so double-click copies `src/parse.rs` from `src/parse.rs:42`. See step 3.
- Not built: opening a plain file path, or a `file:` link, from the terminal. No milestone covers it. `terminal/grid/src/links.rs:1-2` says link recognition is "deliberately limited to browser/mail URLs".
- Pointer note: the brief named `TERM` milestone batch 2 `terminal_link`. That batch is about the authority screen and the `clients/terminal_link` mapper crate, not link opening. The link promise is in `workflow/milestones/TERM.md` line 9.
