# Brief agents-14: Waiting from the CLI
Output: `apps/humata2/flows/cases/agents/14-waiting-from-the-cli.md`

Scenario: I (in a plain terminal) and an agent both run commands that need an answer: a change needing presence, a request that goes to the Inbox, and a destructive plan with --dry-run, --yes, --json and --wait.

Steps: presence is asked in the app and the call is sent again; when the host is away a passphrase is typed at the terminal (never from a pipe); presence is never offered from a session's terminal; dry-run prints the plan and changes nothing; `--plan` does what the printed plan said; --yes skips the question; answering no changes nothing; --json doesn't wait and exits 12 with the request; --wait holds for that long; a terminal waits until the answer comes; a wait that ends unanswered is an error; denied is an error with its kind; ⌃C stops waiting with 130.

Branches: a change with no one to confirm it is refused; presence asked twice is reported; the agent uses `humata inbox wait` after an approved request (mcp.rs `an_approved_request_is_waited_for_with_inbox_wait`); MCP InboxWait / InboxAnswer in the dispatch? (check).

Pointers: cli/tests/waiting.rs (16), cli/tests/exit_codes.rs, cli/src/exit.rs, api/CLI.md §6–7, daemon/humatad/tests/mcp.rs.
