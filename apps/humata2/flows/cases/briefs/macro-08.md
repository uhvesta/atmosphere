# Brief macro-08: First run and adding a machine
Output: `apps/humata2/flows/cases/macro/08-first-run-and-adding-a-machine.md`

Scenario: I download the app zip, open it from Downloads, it installs itself into Applications and installs the host daemon and CLIs; I make my first channel; I add box-1 with Add machine over SSH; the host key is new; the box lacks some tools.

Steps: install flow (the app opened outside Applications installs itself — check whether Humata 2 does this: grep desktop/src for Applications); daemon started at login; first-run empty Home; Add machine overlay (ui/src/overlay_add_machine.rs) and its steps; tools installed on the host (D49 "hosts get our CLIs"); first session on box-1.

Branches: SSH fails (auth, unreachable) with a fix; host key changed (TrustNewHostKey not_built); box is macOS (launchd gap); box on another release (Migrate item); I cancel halfway.

Pointers: features/hosts/tests/add.rs, health.rs, page.rs; desktop/tests/native/hosts_settings.rs, machines.rs; daemon/humatad/tests/machines.rs, forget_machine.rs, remove_machine_*.rs; daemon/humatad/src/machine_service.rs not_built; workflow/milestones/MACH.md; OVERVIEW.md.
