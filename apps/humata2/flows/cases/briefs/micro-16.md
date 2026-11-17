# Brief micro-16: Typing over a slow or dropped SSH link
Output: `apps/humata2/flows/cases/micro/16-typing-over-a-slow-or-dropped-ssh-link.md`

Scenario: I work in a shell on box-1 (a remote machine reached over SSH). The link lags, then drops while I am mid-word, then comes back.

Steps: typed keys are sent in order; echo arrives late but in order; when the link drops the pane says disconnected and input is dropped (not queued and replayed later); Reconnect or automatic reconnect; on reattach the screen catches up from its cursor and typing resumes only once synced.

Branches: the box reboots (holder gone); the link is up but the box's daemon is stopped; the box is Away from the host; a keyed input sent twice types once.

Pointers: terminal/lab/tests/*ssh*.rs (real OpenSSH VM), terminal/pane/tests/pane.rs `while_disconnected_input_is_dropped_and_the_pane_says_so`, daemon/holder/tests/keyed_input.rs, daemon/humatad/tests/terminal.rs (reattach from cursor), daemon/humatad/tests/remote*.rs, ui/src (grep "Reconnect"). OVERVIEW notes the peer link has never run over real SSH — check and say which parts are proven over SSH (lab, terminal only) versus peer link (fakes).
