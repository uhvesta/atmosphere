# REPOS: A repo I add once is known everywhere, and I find repos by fuzzy search scoped to my usual orgs unless I widen it

**Owner (2026-11-16), verbatim:** "same for repos. If I add a repo to be part of my state it should save it as a reference and be able to recall it. I shouldnt need to readd org + repo names ever again per machine. I add it and it knows of it. Also it should let me have a fuzzy search mode or whatever where I can put in strings and it will search for repos for me. I should also be able to restrict searches to some repos globally via configs and have to manually opt out of that when I am searching (say I tend to work in `bazelbuild`'s org a lot, usually it'd pull it from there but occasionally I might want to search in the `buildfarm` org or the `buildbarn` org or in the `bazel-contrib` org.)"

**Exit:**
- A host-held repo library, synced to every machine and recalled in every picker.
- One fuzzy search: the library first, then GitHub via gh.
- A default org scope, set in Settings and ~/.humata2/config/repos.toml, shown as chips. Widening is explicit and per search.
- Spec, flows, D470–D479, frames for the owner, then code and tests.

## Lane
**Manager:** Opus (REPOS). D470–D479. Rules RL-1 to RL-12 and ST-21 (SPEC §3, "The repo library and repo search").

**Design:**
- New crate `daemon/repo_library` (`humata2_repo_library`) does one job. It holds the library in its own SQLite file (WAL, a transaction per write, a revision counter like `daemon/marks`), offering add_or_touch, list, remove and forget_all. It parses and writes `Scope` from `config/repos.toml` (owners, include_me; comments kept), and does the pure merge: library group (D274 tiers, last used), then the GitHub group minus repos already shown.
- humatad: a `RepoService` with SearchRepos(query, scope override, page) → {library, github, problem}, ListLibrary, RemoveFromLibrary, ForgetLibrary, GetRepoScope, UpdateRepoScope (user only). GitHub search via `gh api search/repositories` with a per-(query, scope) 60 s cache. Your repos reuses `gh_repos.rs`. Every successful add calls add_or_touch. `repos.toml` is pushed to machines like `stack.toml`.
- clients: `Daemon` trait methods with fakes (library, scope, scripted GitHub groups and problems) and live calls.
- features: one `RepoSearch` state and reducer (chips, widen, All of GitHub, Save as default, two groups, problem line), used by New channel and the Desk's Add repo. Settings › Repositories.
- ui: a single-job `repo_search_field.rs` (chips + field + groups), used by `overlay_new_channel.rs` and `overlay_desk_add_repo.rs`, plus a Settings section.
- CLI/MCP: `humata repos search|list|remove`, the MCP tool `repo_search`.

**Batches:** B1 the library crate (tests first). B2 daemon service + gh search + add hooks + sync (SEAMs: proto, serve.rs, BUILD, Cargo). B3 clients fakes + live. B4 features + ui. B5 CLI + MCP + api docs.

**Not mine:** CH-18 local path and frame 20's local path (NOPATH). DK-2, DK-3 and DK-14 text (DOCV).

**Batch, 2026-11-16 (PLACES script 73):** RL-12's box side. A machine's daemon used to answer RepoService "not built", so a box agent's `repo_search` failed. Now `repo_service/on_machine.rs` (`OnMachine`) forwards an agent's SearchRepos and ListRepoLibrary up the Uplink. They are two new `away::Method` rows, journal none, and the host's `run_forwarded` runs them as that agent through `BoardSvc::with_repos`. Away they are `host_unreachable`, and no copy is kept. Writes, scope, owners, and the user's own calls on a machine are refused, with a fix that points at the host (D568). Tests: `away_forward.rs` (3) and `board_service/replay.rs` (2). Flows 02 and 05 and cases agents/06 and macro/02 are retagged. Still open: the user's own `humata repos search` over SSH on a box (MachineUser forwarding, cut in AWAY, no milestone); the CH-26 card's "not in your library yet" mark (ROAM); and the per-machine add-repo cells, untested with a one-machine live rig.
