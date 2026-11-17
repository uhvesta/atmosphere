# Brief agents-10: Roaming a conversation between Claude, Codex and Copilot
Output: `apps/humata2/flows/cases/agents/10-roaming-between-agents.md`

Scenario: a session started with Claude hits a limit. I fork it to Codex with the conversation; later fork that to Copilot; then move the Codex one to box-1. Each new agent picks up where the last left off.

Steps: fork with another agent is handed the conversation and a summary; context-only fork with another agent hands over in the parent's folder; code-only with another agent starts it with nothing to read; a handover cut after launch is typed once after a restart; freeing a context-only fork removes its handover file.

Branches: an agent Humata does not know is refused; layout discovery per CLI (ROAM milestone: conversation fork/move for three CLIs, layout discovery fixtures — workers running); Copilot's format unverified live; recover exact agent conversations (recent commit "recover exact agent conversations and preserve forks").

Pointers: daemon/humatad/tests/fork.rs (another-agent tests), context_fork.rs (`key_conversations_by_folder`), workflow/milestones/ROAM.md, daemon/humatad/src (grep "handover", "layout"), codex_trust.rs, providers.rs.
