# Humata 2 user flows

These are the ways one person uses Humata 2 in a working day, written as test cases in plain words. Each flow starts from a sentence the owner might say, then walks through it step by step: what I do, what I see, and what the daemon should hold afterwards. Every step says how much of it is real today.

There's no code here. Tests come later, built from the events each step names.

## The flows

| # | Flow | Starts from |
|---|------|-------------|
| 1 | [A ping on Slack: look into it on the Desk](01-slack-ping-desk.md) | "I got pinged on Slack, let me make a desk and see what this could be." |
| 2 | [A bug to fix: find the repos, make a channel](02-bug-fix-channel.md) | "I got pinged to fix this bug, let's find the related repos and maybe make a channel." |
| 3 | [On call: open the channel, paste the alert, start a session](03-oncall.md) | "I'm on call, let me open the oncall channel and paste something into the desk." |
| 4 | [Writing a proposal doc with an agent](04-proposal-doc.md) | "I'm writing a doc for a proposal." |
| 5 | [A long-running project: spec, milestones, sessions, machines](05-long-running-project.md) | "I want a spec, then milestones, then sessions on them, then to move across machines." |
| 6 | [Reviewing someone's pull request with an agent](06-review-a-pr.md) | "A teammate asked me to review their PR." |
| 7 | [After a reboot, a power cut or an upgrade](07-after-a-reboot-or-upgrade.md) | "My Mac just rebooted. Did my sessions survive?" |
| 8 | [Stopping a machine for the night, and working away from the Mac](08-stop-a-machine-for-the-night.md) | "devbox-1 is burning money overnight." |
| 9 | [A plain terminal on a remote machine](09-ssh-terminal-session.md) | "I just want a shell on devbox-1." |

Flows 6–9 aren't from the owner's list. They cover parts of the spec the first five don't reach: PR review (26.17) and prepared views (26.12); reboots, power cuts and upgrades (26.8, 26.9, UPGRADE); stopping a machine and working away (STOP, 26.10); and plain shells.

A finer-grained set of cases, from single keystrokes up to whole days, is being written separately in [cases/](cases/).

## How each flow reads

- **Starting state:** what exists before step 1.
- **Steps**, each with:
  - **Do:** what I do.
  - **See:** what the screen shows.
  - **Expect:** what must be true afterwards, including the events the daemon puts on its feed.
  - **Status:** how much is real today, with the tests that show it.
- **Expected outcome:** where the whole flow should leave things.
- **Gaps this flow shows:** what stops the flow from working end to end.

Some steps have a **Do (other case)**, a branch off the main path, such as a machine that's offline.

### Events

Events are named as the feed names them: `<Entity> created`, `updated` or `deleted`, with the state in brackets when it matters. For example, "Session updated (Needs input)" or "InboxRequest created (Pending)". The entities are Session, Channel, Machine, BoardItem, InboxRequest, Grant, Comment, PreparedView, Operation, Transfer (file copies), ActivityEvent, Marks, Settings and ChannelRules. Plus the feed's own notes: JournalReplayed, PullRequestHeadsChanged, HostLinkChanged and Resync.

Desk sessions have no event yet. The Desk page reloads its list instead. Flow 1 lists this as a gap.

## Status tags

- **Works**: a test drives this against a real humatad2 (and holder or shell), and passes. The test is named as `path::test_name`, with paths relative to `apps/humata2/`.
- **Fakes only**: a test proves the screen or feature, but against the fake clients. What's unproven live is said.
- **Not built**: there's no code for it. The tag names the milestone and requirement that will build it, such as "ROAM (CD-2)", or says **no milestone** when nothing plans it.

How they're applied:

- A step often has more than one tag. For example, the daemon **Works** while the screen is **Fakes only**. Read "Works in the daemon" as "the daemon side is proven, the app side isn't".
- **Works** with a real daemon still doesn't mean a real agent CLI. The live app tests (`desktop/tests/live_flows*`, `live_boards`, `live_inbox`) run a real humatad2 and holder with a scripted test agent. Most drive the app through its actions; `live_inbox` and `live_boards` click.
- Real agent CLIs, and real SSH to a VM, are proven only in the standalone terminal experiment (`terminal/`), which isn't wired to humatad2 yet.
- Remote machines in daemon tests sit behind a fake `ssh` on the same computer. A real Linux machine is the LINUX milestone, which is blocked.
- A test that exists but whose milestone lane says it hasn't compiled is called **unverified**.

## Choices made while writing these

- Three tags only. A step that's half built gets two tags, one for each half, instead of a fourth tag.
- Examples use `acme/…` repos, `devbox-1` for a remote machine, and `you/` as the branch prefix. Labels in **bold** are the app's own labels, or the frame's where the app has none yet.
- Each flow is written as one person on one day. Where Humata deliberately has no team features (sharing a doc, handing on call to someone), the gap says "no milestone, by design".
- Steps follow the spec's flows where one exists (26.x) and the design frames for labels. Where the spec and the code disagree, the step follows the spec and the status says what the code does.
