# AGY: Antigravity is a supported agent

- **Status:** open (owner, 2026-11-19: "we will add agy (antigravity) support")
- **Depends on:** INBOXHOOKS (its asks come only through hooks or an API, never the screen).
- **Lane:** daemon (agent kinds) + features/settings + desktop goldens

## Outcome
I can start Antigravity's agent (`agy`) in any session like claude, copilot or codex:
- it appears in the new-session and "New agent tab" menus;
- Settings › Agents can turn it on and off, and installs it only if I opt in (per-agent opt-in rule);
- it resumes after restarts and moves;
- its permission asks and questions reach the Inbox through whatever hook or API it offers, and if it has none, they stay in its terminal.

## Steps
1. Research `agy` as it is now (fetch its docs; don't install system-wide): how it launches, how it resumes a session, where its config and auth live (never modify them), what hooks, events or API it has for permissions and questions, and how it picks models (in its own TUI; Humata picks none).
2. Add it as an agent kind next to the existing ones (D673/D795 patterns), with hooks where it offers them.
3. Write flows in `flows/` and a live run on the cheapest model, in an isolated home.

## Out of scope
New screen parsing for its prompts (INBOXHOOKS: best effort only).
