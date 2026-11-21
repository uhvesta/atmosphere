# QUICKWINS (ATM-24..26)

Milestone: Lima and dev-install quick wins.

## Deliverables

### ATM-24: `apps/humata2/tools/release/dev-install.sh`
- Added executable POSIX shell script `apps/humata2/tools/release/dev-install.sh`.
- Incrementally fastbuilds `//apps/humata2/app:app` through `tools/bq/bq --build --fast //apps/humata2/app:app`.
- Developer-ID signs locally inside-out (`Contents/Helpers/*`, `Contents/Extensions/*.appex` with entitlements, updates and verifies `Contents/Resources/manifest.json` sha256s, signs the `.app`, verifies with `codesign --verify --deep --strict` and the daemon's team requirement `codesign --verify -R=...`), installs to `/Applications/Humata 2.app` (configurable via `--dest`), and relaunches (unless `--no-launch`). No notarization and no GitHub upload.
- Supports optional `--watch` mode (`fswatch -1 -r apps/humata2` if available, else POSIX polling loop with `DEV_INSTALL_POLL_SECS` and `DEV_INSTALL_WATCH_MAX`).
- Fails fast before building when no `Developer ID Application:` certificate is found in the Keychain (supports `--identity`, `HUMATA2_SIGNING_IDENTITY`, and `HUMATA2_SECURITY_BIN` for deterministic boundary testing).

### ATM-25: Lima provider blank VM auto-naming & accurate UI copy
- `apps/humata2/daemon/svc_machines/src/machine_service/mod.rs`:
  - Added `pub fn resolve_provider_machine_name(provider_id: &str, requested_name: &str, existing_names: &[String]) -> String` and `Work::resolve_provider_name(&self, provider_id: &str, requested_name: &str) -> Result<String, ManageError>`.
  - Generates lowest unused `lima-1`, `lima-2`, … (case-insensitive collision check against all `Ready`, `Adding`, and `Removing` entries in `hosts.json`) when `provider_id == "lima"` and Name is blank. Preserves explicit names (trimmed) and leaves non-Lima blank names empty.
  - Replaced misleading `ManageError::BadName` fix copy (`"use letters, digits and dashes, or leave the name empty to take it from the host"`) with `"use letters, digits and dashes, or leave the name empty to take an SSH host's name or auto-name a Lima VM"`.
- `apps/humata2/features/hosts/src/add.rs`:
  - Updated `AddMachineState::name` doc comment and added `AddMachineState::name_placeholder(&self) -> &'static str` returning `"Auto-named (lima-1, lima-2, …) if blank"` for Lima, `"Name for the machine"` for other providers, and `"Leave empty to use the host name"` for SSH hosts.

### ATM-26: Focused validation & smoke tests
- `dev-install.sh` verified:
  - `sh -n apps/humata2/tools/release/dev-install.sh` -> exit 0
  - `apps/humata2/tools/release/dev-install.sh --help` -> exit 0
  - Missing-signing-identity boundary (`HUMATA2_SECURITY_BIN` returning `0 valid identities found`) -> exit 1 before build with actionable message:
    `dev-install: no Developer ID Application certificate found in the Keychain. Create one in Xcode > Settings > Accounts > Manage Certificates, or pass --identity / set HUMATA2_SIGNING_IDENTITY. An Apple Development certificate won't do: the daemon accepts only a Developer ID app.`
  - Unknown flag (`--bogus-flag`) -> exit 1 with usage hint.
- Unit tests added:
  - `//apps/humata2/daemon/svc_machines:svc_machines_unit_test`:
    - `resolve_provider_machine_name_auto_names_lima_and_fills_gaps`
    - `work_resolve_provider_name_skips_adding_and_removing_entries`
    - `bad_name_error_fix_mentions_ssh_and_lima_auto_naming`
  - `//apps/humata2/features/hosts:add_test`:
    - `name_placeholder_matches_selected_target_kind`
    - `lima_provider_add_with_blank_name_sends_none_and_closes_with_auto_name`

## Shared Seams Needed (outside QUICKWINS claims)
1. `apps/humata2/daemon/humatad/src/provider_service.rs` (`ProviderSvc::create`, lines ~179-199):
   Resolve the provider ID and auto-assign blank Lima names via `self.work.resolve_provider_name(&provider_id, &request.name)` before `self.gate(...)` and `self.work.machines.plan_create(&provider, &resolved_name, ...)`.
2. `apps/humata2/ui/overlays/src/overlay_add_machine.rs` (`AddMachineDialog::main`, lines ~528-545):
   Update the `Name` input placeholder (or add a subtle hint below the Name row) from `state.name_placeholder()` so selecting `Local VM (Lima)` displays `"Auto-named (lima-1, lima-2, …) if blank"`.
