# Brief agents-06: An agent proposes a repo for the channel
Output: `apps/humata2/flows/cases/agents/06-proposing-a-repo.md`

Scenario: while fixing a bug, the agent realises it needs acme/shared-lib. It runs `humata channel repo add eu-checkout acme/shared-lib`. I approve in the Inbox; the repo is cloned.

Steps: the add becomes an Inbox request under the repo-add rule; approving adds and clones; if I had already added it, the approval succeeds without a second clone.

Branches: asking for another channel is denied and adds nothing to the Inbox; a desk agent's proposal for any channel is asked; a local path is refused; other channel changes by an agent are never allowed; RemoveRepo not_built; AddRepoToSession not_built (session_service.rs).

Pointers: daemon/humatad/tests/channel_repo_propose.rs (7), channel_add.rs, cli/src/commands (channel repo), daemon/humatad/src/session_service.rs (add_repo_to_session not_built).
