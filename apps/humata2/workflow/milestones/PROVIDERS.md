# PROVIDERS

**Outcome:** You can add a machine from a custom provider in the app, and an agent can find, import and use providers without admin mode.

**Why:** Owner, 2026-11-18 12:05: "Do custom machine providers even work via non admin mode? Feels like it doesnt." A live repro (12:19) on a `--admin-pid` daemon, with Gemini Flash in Copilot, found these:

1. The Add machine Name input never takes focus. Typing goes to "or type a host" and drops the provider pick, so no provider machine can be added in the app.
2. The agent's `provider ls` is empty, even after an import.
3. The agent's `provider config show` is refused.
4. The agent's CLI `provider import` is refused. Only MCP files the Inbox request.
5. No docs for the TOML format.
6. An MCP import takes two approvals, and the card title reads "asks to ask you".
7. Hook cards show `{}` or raw JSON.
8. There is no Settings place for providers.
9. Host variables never reach SystemRunner (machine_service.rs:464 and :2170, login_service.rs:325).

**Exit:** Each finding is fixed, with focused tests that use the non-user caller path. A live rerun shows that an agent can list and import a provider, and the user can approve it and add a machine with it from the app.

## Settings design (Fable 5.2, 2026-11-18 12:22)

No new page. Add a **Providers** block in Settings › Machines, above Scripts, styled like `machine_scripts_block`.

- **Note:** "Commands Humata runs on this Mac to make, start, stop and delete machines. An imported provider runs with your account's access, so read its commands before you add it."
- **Rows:** one per provider.
  - Title: the provider's label.
  - Note: `id · Built in|Imported · <actions defined>`, then "· N machines" if any, then "Needs VAR" only when a variable is unset.
  - Buttons: "Commands" on every row, "Remove" on imported rows only.
- **Commands** expands the row in place. One row is open at a time. It shows:
  - a read-only mono block of each command;
  - the params, with their defaults;
  - a host-variable line for each variable the commands name: the name, "Set" or "Not set", a secure field with "Set" or "Replace", and "Clear" when set. Values live in the Keychain and never come back from the daemon.
- **Remove** asks for an in-place confirm. If machines still use the provider, the confirm is replaced by "<label> still runs X and Y. Delete those machines first.", with no Remove button.
- **"Import provider…"** under the card expands into a paste field ("Paste the provider's TOML"). It is paste only: no file chooser and no path field.
  - **Review** calls PreviewProviderImport and shows the label, id, every command verbatim, the variables it reads, and "Replaces <existing>" if the id exists. Buttons: Import and Cancel.
  - A parse error shows as a red alert line under the field.
- **Add machine:** the Host pop-up ends with a divider and "Manage providers…", which opens Settings › Machines at Providers. If the picked provider has an unset variable, the error area says "<label> needs VAR. Set it in Settings > Machines.", with an "Open Settings" button, and Add is disabled.
- **Refusal messages** that say "the user can do it themselves in the app" should point at Settings › Machines › Providers.
- **Golden frames** (go to Fable for approval):
  - A: at rest.
  - B: a row expanded, with a variable.
  - C: import review.
  - D: parse error.
  - E: remove blocked.
  - F: the Add machine pop-up with "Manage providers…".

## Lane

- 12:20: three lanes started:
  - Daemon/CLI: findings 2–6 and 9.
  - UI: finding 1 and hook cards (finding 7).
  - Fable: the design above.
- Settings block: starts after the UI lane, because the two share the overlay_add_machine pop-up.
- 13:40: the three lanes landed, and their gate is green (CLI, daemon, UI, features and clients targets).
  - Fixes made in main:
    - the payloads raw string;
    - lima.toml through rustc_env;
    - settings_providers as a crate module;
    - the "asks you …" title;
    - `provider import --reason`.
  - Native frames A–E were recorded (`native_test_record -- providers`). Fable approved them after two fix rounds:
    - C pastes the full TOML and shows connect;
    - C and D have a caption;
    - D drops the line number;
    - the review shows the row note.
  - A follow-up lane is open:
    - RemoveProvider RPC;
    - ListProviders returns the commands;
    - presence proofs;
    - "Manage providers…" with frame F;
    - the unset-variable error;
    - refusals that point to Settings.
- 15:15: the follow-up lane landed.
  - Built and tested:
    - RemoveProvider;
    - ListProviders returning the commands;
    - "Manage providers…" ending the Host pop-up;
    - the unset-variable error, with "Open Settings", blocking Add;
    - refusals pointing at Settings › Machines › Providers.
  - Frame F is recorded (`providers-f-host-popup`). The Host row overflow it exposed (pre-existing in frame 56) is fixed: the hint is now "from ~/.ssh/config" and the field flexes. Fable approved native 56, design 56 and F.
  - **Live rerun (exit), CLI against a fresh `humatad2_admin`.** An agent session's token came from the shell the daemon starts. Results:
    - `whoami` shows the agent.
    - The agent's `provider ls` lists lima, and its `provider config show` succeeds.
    - The agent's `provider import cloud.toml --reason …` makes one Inbox request ("provider live asks you to import a machine provider"). The preview shows the file and every command.
    - The user's `inbox approve` imports it. The agent's `provider ls` then shows Test Cloud with CLOUD_TOKEN unset.
    - The agent's `provider rm` is refused with "ask the user to remove it in Settings › Machines › Providers". The user's `rm` works.
  - The app side is covered by the headless native frames A–F, per the no-windows rule.

**Status: done.**
