# Humata 2: requirement trace

Every requirement ID in [SPEC.md](../SPEC.md) mapped to the gRPC calls
([GRPC.md](GRPC.md), `proto/humata/v1/`), MCP tools ([MCP.md](MCP.md)) and
`humata` commands ([CLI.md](CLI.md)) that carry it. "UI only" means the
requirement is how the app draws or behaves and needs no call of its own
beyond reading state it already has (events, `GetViewState`). "Internal"
means it is how a daemon behaves and is checked by tests, not by a call.

RPCs are written `Service.Rpc`. "—" means no MCP tool or command, on
purpose; the reason is in the last column when it is not obvious.

The access check for each call is in BOUNDARIES.md §3 and the capability
column of MCP.md §6. The matrix in §2 here sums it up per actor.

## 1. Requirements

### AR: architecture (5)

| ID | RPC | MCP | CLI | Note |
| --- | --- | --- | --- | --- |
| AR-1 | DaemonService.GetServerInfo, PeerService.Hello | — | `humata status` | one daemon per machine (PER-7) |
| AR-2 | all services; DaemonService.WhoAmI | all tools | all commands | one API, three clients |
| AR-3 | EventService.Subscribe, ViewStateService.GetViewState | — | — | app holds nothing that matters |
| AR-4 | PeerService.Uplink, TransferService.* (via the Mac) | topology_get | `humata topology` | no machine-to-machine path |
| AR-5 | ViewStateService.GetViewState/PutViewState | — | — | view state on the host |

### CH: channels (25)

| ID | RPC | MCP | CLI | Note |
| --- | --- | --- | --- | --- |
| CH-1 | ChannelService.CreateChannel | channel_create | `humata channel create` | |
| CH-2 | ChannelService.CreateChannel (folder returned) | channel_create | `humata channel create` | derived, never typed |
| CH-3 | ChannelService.SearchGitHubRepos | — | — | picker only; agents name repos |
| CH-4 | MachineService.ListMachines | topology_get | `humata host ls` | offline listed, not choosable |
| CH-5 | ChannelService.CreateChannel → Operation | operation_get | `humata ops` | |
| CH-6 | ChannelService.AddRepo | channel_add_repo | `humata channel repo add` | |
| CH-7 | ChannelService.AddMachine | channel_add_machine | `humata channel machine add` | |
| CH-8 | ChannelService.PlanRemoveMachine/RemoveMachine | channel_remove_machine | `humata channel machine rm` | plan, then act (NV-27) |
| CH-9 | ChannelService.GetGrid | topology_get | `humata channel grid` | |
| CH-10 | SessionService.StartSession (no repos) | session_start | `humata session start` | |
| CH-11..CH-14 | ChannelService.GetChannel, SessionService.ListSessions | channels_list | `humata ls`, `humata channels` | tabs, tree table: UI only |
| CH-15 | ActivityService.ListActivity | — | `humata activity` | |
| CH-16 | ChannelService.ListChannels, ActivityService.MarkSeen | channels_list | `humata channels` | |
| CH-17 | ActivityService.MarkSeen | — | — | app marks on open |
| CH-18, CH-19 | ChannelService.AddRepo | channel_add_repo | `humata channel repo add` | |
| CH-20 | Internal | — | — | existing sessions unchanged |
| CH-21 | SessionService.AddRepoToSession | session_add_repo | `humata session repo add` | |
| CH-22 | ChannelService.PlanRemoveRepo/RemoveRepo | channel_remove_repo | `humata channel repo rm` | |
| CH-23 | SessionService.PlanDropRepo/DropRepo | session_drop_repo | `humata session repo drop` | |
| CH-24 | ChannelService.PlanDeleteChannel/DeleteChannel | — | `humata channel delete` | |
| CH-25 | CHANNEL_DELETE user-only; no InboxService kind for it | — (refused user_only, no ask) | `humata channel delete` (presence) | |

### FC: folders and clones (18)

| ID | RPC | MCP | CLI | Note |
| --- | --- | --- | --- | --- |
| FC-1 | none take a path for Humata's own folders | — | — | |
| FC-2 | Internal (read-only main clones); FileService.WriteFile refuses | file_write refuses | — | |
| FC-3 | SessionService.StartSession | session_start | `humata session start` | |
| FC-4 | StartSessionRequest repos | session_start | `--repo` | |
| FC-5 | SessionService.PlanArchive/ArchiveSession | session_archive | `humata session archive` | |
| FC-6, FC-7 | PeerService.PrepareClone, ReceiveBundle, CreateBundle | — | — | internal choice of path |
| FC-8 | FileService.* on `shared/` | files_copy_in, file_write | `humata cp` | |
| FC-9 | none (no behind-main status) | — | — | |
| FC-10 | MachineService.PlanChangeRoot/ChangeRoot | — (user-only) | `humata host root` | |
| FC-11..FC-15 | SessionService.PreviewBranchNames, StartSession, PreviewFork | session_start, session_fork | `humata session start`, `fork` | |
| FC-16 | Internal (record outside clones) | session_info | — | |
| FC-17 | SessionService.RenameSession | session_rename | `humata session rename` | branch kept |
| FC-18 | PullRequestReviewService.StartReview | pr_review_start | `humata review start` | |

### SE: sessions (25)

| ID | RPC | MCP | CLI | Note |
| --- | --- | --- | --- | --- |
| SE-1..SE-3 | SessionService.StartSession, PreviewBranchNames | session_start | `humata session start` | |
| SE-4 | SessionService.StartSession → Operation | operation_get | `humata ops` | |
| SE-5 | StartSessionResponse | session_start | `humata session start` | |
| SE-6 | UI only | — | — | |
| SE-7 | SessionService.Rename/Move/Stop/PlanArchive… | session_* | `humata session …` | |
| SE-8, SE-9 | TerminalService.ListTabs, OpenShellTab, CloseTab | — | `humata attach --tab` | shell tab is user-only |
| SE-10 | TerminalService.Attach (input) | — | `humata attach` | input user-only (TERMINAL_INPUT) |
| SE-11, SE-12 | UI only | — | — | |
| SE-13 | TerminalService.Attach (snapshot + offset) | — | `humata attach` | |
| SE-14 | SessionService.GetSession, Session.status | session_info | `humata ls` | |
| SE-15 | SessionService.ResumeSession | — | `humata session resume` | |
| SE-16 | TerminalService.Attach (last screen), Session.status | session_info | `humata attach` | |
| SE-17 | UI only | — | — | |
| SE-18 | SessionService.GetSession | session_info | `humata ls` | |
| SE-19 | Session.survival | session_info | — | |
| SE-20 | AccessService.Explain | access_explain | `humata access why` | |
| SE-21 | Session.turn_end (TurnEndSource), AgentDefinition.turn_end_source | session_info | `humata agents ls` | |
| SE-22 | Session.turn_end, SESSION_STATUS_PROBABLY_IDLE | session_info | `humata ls` | |
| SE-23 | TerminalService.ListQueued, SendQueuedNow, CancelQueuedTyping | — | `humata session queue`, `send-now` | user's |
| SE-24 | TerminalService.Attach (other_writers) | — | `humata attach` | |
| SE-25 | TerminalService.Attach (read_only) | — | `humata attach --read-only` | |

### FK: forks (13)

| ID | RPC | MCP | CLI | Note |
| --- | --- | --- | --- | --- |
| FK-1..FK-4 | SessionService.PreviewFork, ForkSession, ListTurns | session_fork | `humata session fork` | |
| FK-5, FK-6 | PeerService.ExportConversation/ImportConversation | session_fork | `humata session fork` | |
| FK-7, FK-8 | FileService.* (parent's clones, read-only) | files_list, file_read | `humata files` | IN-20 |
| FK-9 | SessionService.GiveOwnCopy | session_own_copy | `humata session own-copy` | |
| FK-10 | Internal (first terminal line) | — | — | |
| FK-11 | ActivityService.ListActivity | — | `humata activity` | |
| FK-12 | PeerService.CreateBundle/ReceiveBundle | session_fork | — | |
| FK-13 | SessionService.ArchiveSession | session_archive | `humata session archive` | |

### MV: move and archive (12)

| ID | RPC | MCP | CLI | Note |
| --- | --- | --- | --- | --- |
| MV-1..MV-5 | SessionService.MoveSession → Operation | session_move | `humata session move` | |
| MV-6 | Internal | — | — | |
| MV-7..MV-9 | SessionService.PlanArchive/ArchiveSession | session_archive | `humata session archive` | |
| MV-10 | SessionService.PlanPush, PushAndOpenPullRequests, ArchiveSession | session_push, session_archive | `humata session push --pr` | |
| MV-11 | UI only | — | — | |
| MV-12 | SessionService.PlanPush/PushAndOpenPullRequests | session_push | `humata session push` | |

### LD: lifecycle and disk (22)

| ID | RPC | MCP | CLI | Note |
| --- | --- | --- | --- | --- |
| LD-1, LD-2 | SessionService.ArchiveSession | session_archive | `humata session archive` | |
| LD-3 | SessionService.ListSessions (archived) | — | `humata ls --all` | |
| LD-4 | SessionService.RestoreSession, RebuildSession | session_restore, session_rebuild | `humata session restore`, `rebuild` | |
| LD-5 | SessionService.PlanDelete/DeleteSession | session_delete | `humata session delete` | |
| LD-6, LD-7 | TrashService.ListTrash, RestoreFromTrash, PlanEmptyTrash/EmptyTrash | trash_restore | `humata trash`, `trash empty` | empty is user-only |
| LD-8, LD-9 | PeerService.Uplink (MirrorChunk) | — | — | |
| LD-10 | MachineService.PlanRemoveMachine/RemoveMachine | machine_remove | `humata host remove` | |
| LD-11, LD-12 | MachineService.PlanForgetMachine/ForgetMachine | machine_forget | `humata host forget` | |
| LD-13, LD-14 | MachineService.GetStorage | — | `humata host storage` | |
| LD-15 | MachineService.ClearBuildCache, TrimLogs (same checks) | — | `humata host storage` | |
| LD-16..LD-18 | SettingsService.UpdateSettings/UpdateChannelSettings (retention), ActivityService | — | `humata settings`, `channel settings` | |
| LD-19, LD-20 | errors: disk_full on the blocked call | all tools | all commands | |
| LD-21 | PeerService.Uplink (MirrorChunk.scrubbed_names) | — | — | |
| LD-22 | Internal | — | — | |

### RV: review (27)

| ID | RPC | MCP | CLI | Note |
| --- | --- | --- | --- | --- |
| RV-1..RV-3, RV-7 | DiffService.GetDiff, GetFileDiff | — | `humata diff` | agents use git |
| RV-4, RV-20, RV-25 | CommentService.CreateComment | — | `humata comment add` | user's comments |
| RV-5, RV-26, RV-27 | CommentService.SendComments | — | — | user-only (SEC-7) |
| RV-6 | DiffService.MarkReviewed | — | `humata diff mark-reviewed` | user-only |
| RV-8 | CommentService.ListComments | comments_list | — | |
| RV-9..RV-12 | FileService.ListDirectory, FindFiles, ReadFile | files_list, files_find, file_read | `humata files` | |
| RV-13 | FileService (remote), errors.host_unreachable | files_list | `humata files` | |
| RV-14 | UI only | — | — | |
| RV-15..RV-19, RV-21..RV-24 | DiffService.GetFileDiff | — | — | editor modes: UI only |

### PR: PR review (18)

| ID | RPC | MCP | CLI | Note |
| --- | --- | --- | --- | --- |
| PR-1 | PullRequestReviewService.ResolvePullRequest | pr_review_start | `humata review start` | |
| PR-2 | ChannelService.GetChannel (PR reviews channel) | channels_list | `humata ls` | |
| PR-3 | PullRequestReviewService.ResolvePullRequest | pr_review_start | `humata review start` | |
| PR-4..PR-6 | PullRequestReviewService.StartReview | pr_review_start | `humata review start` | |
| PR-7, PR-8 | PullRequestReviewService.SetPushAllowed | — (user-only) | `humata review allow-push` | |
| PR-9 | CommentService.CreateComment (draft, by agent) | review_draft, review_draft_edit, review_draft_drop | — | |
| PR-10, PR-11 | DiffService.GetDiff, CommentService.ListComments | comments_list | `humata review drafts` | |
| PR-12, PR-13 | PullRequestReviewService.PlanSubmit/SubmitReview | — (user-only) | — | app only |
| PR-14 | CommentService.ListComments (posted) | comments_list | `humata review drafts` | |
| PR-15 | PullRequestReviewService.CheckHeads | — | — | daemon polls |
| PR-16 | PullRequestReviewService.UpdateClone | — | — | |
| PR-17 | PullRequestReviewService.CheckHeads | session_info | — | |
| PR-18 | SessionService.ListSessions | — | `humata ls` | |

Review summary drafts: PullRequestReviewService.DraftSummary, MCP
`review_summary_draft` (PR-9, PR-12).

### SM: markdown review and comments (18)

| ID | RPC | MCP | CLI | Note |
| --- | --- | --- | --- | --- |
| SM-3, SM-4 | FileService.ReadFile | file_read | — | rendering: UI only |
| SM-6, SM-8 | CommentService.CreateComment, EditComment, DropComment | — | `humata comment add` | |
| SM-7 | CommentService.ReattachComment | — | — | |
| SM-9, SM-10, SM-14 | CommentService.SendComments | — | — | user-only |
| SM-11, SM-12 | CommentService.ResolveComment, ReopenComment | comments_list | — | |
| SM-13 | CommentService (one model) | comments_list | — | |
| SM-15 | CommentService.ListComments | comments_list | — | |
| SM-16 | Comment.addressed, COMMENT_STATUS_ADDRESSED | comments_list | — | |
| SM-17 | CommentService.ResolveComment, ReopenComment | comments_list | — | re-anchoring and PR updates never reopen: internal |
| SM-18 | CommentService.ResolveComment | comments_list | — | Resolve N addressed: no batch call yet (REVIEW adds it) |
| SM-19 | — | `comment_resolve` (not yet in MCP.md) | — | REVIEW adds the tool, its reply note and access rule |
| SM-20 | — (no ArchiveComment yet) | — | — | user-only; REVIEW adds the RPC |
| SM-21 | CommentService.ListComments | — | — | Archived list and Unarchive: REVIEW adds them |

### FM: file manager (15)

| ID | RPC | MCP | CLI | Note |
| --- | --- | --- | --- | --- |
| FM-1 | FileService.ListDirectory | files_list | `humata files ls` | |
| FM-2 | FileService.CreateFolder, Rename, TransferService.StartTransfer | folder_create, files_copy_in | `humata cp`, `mv` | |
| FM-3 | FileService.MoveToTrash, TrashService.RestoreFromTrash | trash_restore | `humata files rm`, `trash` | |
| FM-4 | TransferService.Upload | — | `humata files put` | |
| FM-5, FM-9 | TransferService (over SSH, through the Mac) | — | — | |
| FM-6..FM-8 | TransferService.ListTransfers, WatchTransfer, Pause/Resume/Cancel | — | `humata transfers` | |
| FM-10, FM-11 | FileService.ListDirectory, MachineService.GetMachine | files_list | `humata files ls` | |
| FM-12, FM-13 | MachineService.ListMachines, errors.host_unreachable | — | — | |
| FM-14 | TransferService.StartTransfer (move) | — | `humata mv` | |
| FM-15 | UI only | — | — | |

### BD: boards (10)

| ID | RPC | MCP | CLI | Note |
| --- | --- | --- | --- | --- |
| BD-1 | BoardService.ListItems (scope) | board_items_list | `humata board` | |
| BD-2 | BoardService.CreateItem, MoveItem, UpdateItem | board_item_create/move/update | `humata board add/move/edit` | |
| BD-3 | BoardService.GetItem, GetItemHistory | board_item_get, board_item_history | `humata board history` | |
| BD-4..BD-6 | BoardService.ListItems, CreateItem | board_items_list | `humata board` | |
| BD-7 | BoardService.ListItems (global) | board_items_list (GLOBAL_READ) | `humata board --global` | |
| BD-8 | BoardService.ListItems (session) | board_items_list | `humata board --session` | |
| BD-9 | BoardService.* under channel rules | board_item_* | `humata board` | |
| BD-10 | open (q5) | — | — | spec leaves it open |

### IN: Inbox and access (26)

| ID | RPC | MCP | CLI | Note |
| --- | --- | --- | --- | --- |
| IN-1..IN-4 | InboxService.ListRequests (scope) | — | `humata inbox` | |
| IN-5 | InboxRequest.kind | inbox_mine | `humata inbox` | |
| IN-6 | InboxService.AnswerAll | — | `humata inbox approve-all/deny-all` | user-only |
| IN-7 | ListRequests (answered) | — | `humata inbox --all` | |
| IN-8 | AuditService.ListAudit | — | `humata audit` | |
| IN-9, IN-10 | InboxService.GetRequest (RequestPreview) | — | `humata inbox show` | |
| IN-11 | UI only | — | — | |
| IN-12, IN-13 | InboxService.AnswerRequest | inbox_wait (result) | `humata inbox approve/deny` | |
| IN-14 | AnswerRequest (grant) | — | `humata inbox approve --always` | |
| IN-15 | EventService.Subscribe | — | — | |
| IN-16 | no-rule set: AccessService.Explain | session_info, file_*, view_*, inbox_* (none) | — | |
| IN-17 | AccessService.GetChannelRules/SetChannelRule, ListCapabilities | — | `humata access`, `access set` | D21 rows |
| IN-18 | errors.denied (rule named) | all tools | all commands | |
| IN-19 | AccessService.ListGrants/RevokeGrant | — | `humata access grants` | |
| IN-20 | Internal (context-only fork never writes parent) | file_write refuses | — | |
| IN-21 | Internal | — | — | |
| IN-22 | GrantTarget | — | `humata inbox approve --always` | |
| IN-23 | errors.user_only | all tools | all commands | |
| IN-24 | SessionService.PlanPush/PushAndOpenPullRequests | session_push | `humata session push` | |
| IN-25 | InboxService.AskUser, WaitForAnswer | inbox_ask, inbox_wait | `humata inbox reply` | up to 4 choices |
| IN-26 | InboxService.RequestAccess (ahead) | inbox_request | `humata access ask-ahead` | |

### VW: views (14)

| ID | RPC | MCP | CLI | Note |
| --- | --- | --- | --- | --- |
| VW-1, VW-2 | ViewService.PrepareView | view_prepare | — | |
| VW-3..VW-9 | ViewService.SetViewState, ListViews | — | `humata views` | offers, open, revert: app |
| VW-10, VW-11 | ViewService.ListViews | views_list | `humata views` | |
| VW-12 | ViewService (stored on the host) | view_prepare | — | |
| VW-13 | PullRequestDraft; SessionService.PushAndOpenPullRequests from_view | view_prepare, session_push | `humata session push --from-view` | |
| VW-14 | ViewService.ReviseView, MarkReady | view_revise, view_mark_ready | — | |

### TP: agent tools (16)

| ID | RPC | MCP | CLI | Note |
| --- | --- | --- | --- | --- |
| TP-1 | ChannelService.GetGrid, MachineService.ListMachines | topology_get, session_info | `humata topology` | |
| TP-2 | FileService.* | files_list, file_read, files_find | `humata files` | |
| TP-3 | FileService.* (other session/machine) | file_read | `humata files cat` | FILES_READ_OTHER_* |
| TP-4 | TransferService.StartTransfer | files_copy_in, file_fetch | `humata cp` | |
| TP-5 | SessionService.SendNote, AskSession, AnswerQuestion | session_note, session_ask, session_answer | `humata session note/ask` | |
| TP-6 | SessionService.ForkSession | session_fork | `humata session fork` | |
| TP-7 | BoardService.* | board_item_* | `humata board` | |
| TP-8 | Internal (checked on the Mac) | all tools | — | |
| TP-9 | see AW-7 | all tools | — | |
| TP-10 | Internal (nothing typed while waiting) | inbox_wait | — | |
| TP-11 | WaitForAnswer (wait_seconds) | inbox_wait | — | |
| TP-12 | WaitResponse.result, result_kept_until | inbox_wait | — | |
| TP-13 | Internal | inbox_wait | — | |
| TP-14 | request_id idempotency (common.proto) | all writing tools | — | |
| TP-15 | InboxService.WithdrawRequest, WithdrawReason | inbox_withdraw | — | |
| TP-16 | MCP naming | all tools | — | |

### HO: machines (13)

| ID | RPC | MCP | CLI | Note |
| --- | --- | --- | --- | --- |
| HO-1, HO-2 | MachineService.AddSshMachine → Operation | machine_add_ssh | `humata host add` | |
| HO-3, HO-4 | MachineService.ListMachines, GetMachine | topology_get | `humata host ls` | |
| HO-5, HO-6 | MachineService.ReconnectMachine | machine_reconnect | `humata host reconnect` | |
| HO-7 | MachineService.ListHostVariables (names only) | — | `humata host vars` | |
| HO-8 | Internal (`gh` on the Mac) | — | — | |
| HO-9 | none | — | — | |
| HO-10 | Internal | — | — | |
| HO-11 | BridgeService.OpenPortForward/ClosePortForward/ListPortForwards | network_connect, network_close | `humata bridge ls/close` | NETWORK_REACH, Ask me or Never (D20) |
| HO-12 | Internal (scope of the promise) | — | — | |
| HO-13 | MachineService.SetHostVariable, RemoveHostVariable | — (user-only) | `humata host var set/rm` | |

### SK: skills (11)

| ID | RPC | MCP | CLI | Note |
| --- | --- | --- | --- | --- |
| SK-1, SK-2 | SkillsService.SyncSkills, PeerService.ApplySkills | skills_sync | `humata skills sync` | |
| SK-3 | SettingsService.UpdateSettings | — | `humata settings` | |
| SK-4 | PeerService.ApplySkills (managed folder) | — | — | |
| SK-5, SK-11 | SkillsService.PreviewSkillsSync | skills_sync (dry run) | `humata skills sync --dry-run` | |
| SK-6 | SettingsService.UpdateSettings (excludes) | — | `humata settings` | |
| SK-7 | Internal (sync on reconnect) | — | — | |
| SK-8 | errors on the blocked session call | — | — | |
| SK-9 | SyncSkills (whole set, OPERATION_KIND_SKILLS_SYNC) | skills_sync | `humata skills sync` | |
| SK-10 | SyncSkillsResponse (skipped machines) | skills_sync | `humata skills sync` | |
| (clash) | SkillsService.ResolveSkillClash | — | — | |

### MP: machine providers (24)

| ID | RPC | MCP | CLI | Note |
| --- | --- | --- | --- | --- |
| MP-1 | Machine (one shape) | topology_get | `humata host ls` | |
| MP-2..MP-6 | ProviderService.GetProvidersConfig/SetProvidersConfig, ListProviders | — | `humata provider ls` | user-only to change |
| MP-7..MP-11 | ProviderService.CreateMachine → Operation | machine_create | `humata provider create` | |
| MP-12..MP-16 | ProviderService.InteractiveLogin | — | `humata provider login` | needs the user's terminal |
| MP-17 | ProviderService.MakeUnattended | — | — | |
| MP-18..MP-20 | CreateMachine steps in Operation | operation_get | `humata ops` | |
| MP-21 | ProviderService.RunProviderAction, DeleteWhatWasMade | machine_start, machine_stop, machine_delete | `humata provider delete` | |
| MP-22 | built-in Lima provider in ListProviders | — | `humata provider ls` | |
| MP-23 | ProviderService.PreviewProviderImport/ImportProvider, SetProvidersConfig | — (user-only) | `humata provider import` | |
| MP-24 | CreateMachine, RunProviderAction under MACHINE_* capabilities | machine_create, machine_start, machine_stop | — | |

### SL: source of truth (8)

| ID | RPC | MCP | CLI | Note |
| --- | --- | --- | --- | --- |
| SL-1, SL-2 | Internal | — | — | |
| SL-3 | PeerService.Uplink (pointers) | — | — | |
| SL-4 | SessionService.RebuildSession | session_rebuild | `humata session rebuild` | |
| SL-5 | PeerService.Uplink (journal) | journal_status | — | |
| SL-6, SL-7 | revision fields on items, comments, views | board_item_update (revision) | `humata board edit` | |
| SL-8 | BoardService.GetItemHistory (tombstones) | board_item_history | `humata board history` | |

### PER: persistence (24)

| ID | RPC | MCP | CLI | Note |
| --- | --- | --- | --- | --- |
| PER-1, PER-2 | PeerService.LaunchSession, ListHolders | — | — | |
| PER-3 | TerminalService.Attach (snapshot) | — | `humata attach` | |
| PER-4..PER-7 | Internal; PeerService.StopHolder/ResumeHolder | — | — | |
| PER-8 | MachineService.SetVmTier | — (user-only) | `humata host vm-tier` | |
| PER-9, PER-10 | Internal | — | — | |
| PER-11, PER-12 | Internal (record before launch) | — | — | |
| PER-13..PER-15 | SessionService.ResumeSession, Session.status | session_info | `humata session resume` | |
| PER-16, PER-17 | Internal; TerminalService.Attach | — | — | |
| PER-18..PER-20, PER-22 | Internal (service install) | — | — | |
| PER-21 | MachineService.ReconnectMachine | machine_reconnect | `humata host reconnect` | |
| PER-23, PER-24 | Internal (kill matrix tests) | — | — | |

### AW: while the Mac is away (12)

| ID | RPC | MCP | CLI | Note |
| --- | --- | --- | --- | --- |
| AW-1 | Internal | — | — | |
| AW-2 | TerminalService.Attach on the machine's daemon | — | `humata attach` | |
| AW-3 | PeerService.Uplink (journal) | journal_status | — | |
| AW-4 | AccessService.SetAwayRule | — | `humata access away` | |
| AW-5 | InboxService.AnswerRequest on the machine | — | `humata login`, `humata inbox` | passphrase |
| AW-6 | PeerService.Uplink (replay) | — | — | |
| AW-7 | GRPC.md §9.1, MCP.md §4 table | all tools | — | |
| AW-8 | PeerService.Uplink (PolicySnapshot) | — | — | |
| AW-9 | Internal (replay check) | board_item_history | — | |
| AW-10 | errors.already_answered | — | `humata inbox approve/deny` | |
| AW-11 | SetAwayRule validates kinds | — | `humata access away low-risk` | |
| AW-12 | AnswerRequest grant on the machine | — | `humata inbox approve --always` | |

### TR: transport (6)

| ID | RPC | MCP | CLI | Note |
| --- | --- | --- | --- | --- |
| TR-1..TR-3 | all services; Attach, Subscribe, WatchOperation streams | — | — | |
| TR-4 | offsets in Attach, Subscribe, WatchTransfer | — | `humata events --after` | |
| TR-5 | Internal | — | — | |
| TR-6 | deadlines per machine | — | — | |

### NV: navigation (27)

| ID | RPC | MCP | CLI | Note |
| --- | --- | --- | --- | --- |
| NV-1..NV-19 | UI only (pane state via ViewStateService) | — | — | NV-8, NV-9: errors only where they block |
| NV-20..NV-22, NV-24 | MarkService.SetMarks, ListPinned | — | `humata pin/unpin/vip/unvip` | user-only |
| NV-23 | SettingsService (quiet hours) | — | `humata settings` | |
| NV-25 | MarkService.ReorderPinned | — | `humata pinned reorder` | |
| NV-26 | Internal (marks dropped on delete) | — | — | |
| NV-27 | Plan*/act pairs with plan tokens | — | `--dry-run`, `--plan` | |

### HM: Home (5)

| ID | RPC | MCP | CLI | Note |
| --- | --- | --- | --- | --- |
| HM-1 | SessionService.ListSessions (needs input) | — | `humata ls` | |
| HM-2 | ActivityService.ListActivity (all channels) | — | `humata activity` | |
| HM-3 | InboxService.ListRequests (global) | — | `humata inbox` | |
| HM-4 | see BD-7 | | | |
| HM-5 | UI only | — | — | |

### KP: command palette (6)

| ID | RPC | MCP | CLI | Note |
| --- | --- | --- | --- | --- |
| KP-1..KP-3, KP-5, KP-6 | UI only | — | — | |
| KP-4 | PullRequestReviewService.ResolvePullRequest | — | — | |

### ST: settings (17)

| ID | RPC | MCP | CLI | Note |
| --- | --- | --- | --- | --- |
| ST-1, ST-2, ST-15 | AgentService.ListAgents, AddAgent, UpdateAgent; Settings defaults | — | `humata agents`, `humata settings` | user-only |
| ST-3, ST-7..ST-12 | SettingsService.UpdateSettings (terminal, long lines) | — | `humata settings` | drawing: UI only |
| ST-4 | AccessService.GetDefaultRules/SetDefaultRules | — | `humata access` | |
| ST-5 | MachineService.ChangeRoot, SetVmTier | — | `humata host root`, `vm-tier` | |
| ST-6 | AccessService.SetAwayRule (default) | — | `humata access away` | |
| ST-13 | SettingsService.UpdateSettings (branch prefix) | — | `humata settings` | |
| ST-14 | SettingsService (retention) | — | `humata settings` | |
| ST-16 | SettingsService (skills) | — | `humata settings` | |
| ST-17 | Settings.security, ChannelSettings.contain_agents | — | `humata settings contain` | |

### SEC: security (15)

| ID | RPC | MCP | CLI | Note |
| --- | --- | --- | --- | --- |
| SEC-1 | ListHostVariables (names only) | — | `humata host vars` | |
| SEC-2 | Internal | — | — | |
| SEC-3 | every call checked | all tools | all commands | |
| SEC-4 | Internal (socket mode) | — | — | |
| SEC-5, SEC-6 | Settings.security, Session.contained, WhoAmIResponse.contained | session_info | `humata settings contain`, `whoami` | |
| SEC-7 | errors.user_only; BOUNDARIES.md §5 | all tools | all commands | |
| SEC-8, SEC-9 | DaemonService.WhoAmI | — | `humata whoami` | |
| SEC-10, SEC-11 | CliAuthService.BeginCliSession/EndCliSession/ListCliSessions | — | `humata login`, `logout` | |
| SEC-12 | BeginCliSession refuses inside a session tree | — | `humata login` | |
| SEC-13 | AccessService.SetApprovalPassphrase, errors.passphrase_locked | — | `humata access passphrase` | |
| SEC-14 | CliAuthService.CreateScriptToken/ListScriptTokens/RevokeScriptToken | — | `humata token` | |
| SEC-15 | no identity argument anywhere; errors.identity_mismatch, ACTIVITY_KIND_IDENTITY_MISUSED | all tools | all commands | |

## 2. Who may do what

Tiers are those of BOUNDARIES.md §3. "Mac away" is a machine judging calls
against its last policy copy (AW-8).

| Actor | No rule (own things, IN-16) | Default Allow | Ask me | Never by default | User-only (SEC-7) | Peer |
| --- | --- | --- | --- | --- | --- | --- |
| User in the app | does it | does it | does it, no request | does it | does it | no: app has no peer link |
| Agent, its own session | does it, no setting stops it | does it unless the channel set Ask or Never | Inbox request; call waits (TP-11) | denied, rule named | refused user_only, pointed at inbox_ask | no |
| Agent, another session or machine | not in this tier: the act falls to its capability | does it, checked on the Mac (TP-8) | Inbox request on the Mac | denied | refused user_only | no |
| CLI inside a session tree (SEC-8) | as that session's agent | as the agent | as the agent | as the agent | refused | no |
| CLI outside a session, with presence | does it as the user | does it | does it, no request | does it | does it | no |
| CLI outside a session, no credential (SEC-9) | refused: only WhoAmI and logging in | refused | refused | refused | refused | no |
| A machine while the Mac is away | done on the machine | journaled (SL-5) or waits if it needs the Mac | per the While I'm away rule; never low-risk if it needs another machine (AW-11) | denied | only answering requests it holds, with the passphrase | judges with its PolicySnapshot |
| Script token (SEC-14) | none: a token has no session | only what the token lists, in its channels | refused: a token cannot wait for the user | refused unless listed | refused always | no |
| Peer daemon (PeerService) | n/a | n/a | n/a | n/a | n/a | Mac-to-daemon only, over the SSH link |

## 3. Gaps

Closed in this pass:

- Channel delete had no calls: PlanDeleteChannel/DeleteChannel now carry the
  plan fields and an operation (CH-24, CH-25).
- Port forwards had no calls: BridgeService (HO-11, AW-7).
- Queued messages could not be listed or sent early (SE-23).
- Read-only attach and several writers (SE-24, SE-25).
- Turn-end source and "probably idle" (SE-21, SE-22).
- Contain agents setting and per-channel override (SEC-5, SEC-6, ST-17).
- Request results kept and replayable, withdraw reasons (TP-11..TP-15).
- Ask-ahead requests and four-choice questions (IN-25, IN-26).
- Grant targets per IN-22; machine and board-item targets deprecated.
- Passphrase generate and lockout (SEC-13).
- Change root with a plan and moving channels (FC-10); host variable removal
  (HO-13); provider import (MP-23); every skills sync previewable (SK-11).
- Pull request from a view (VW-13); view drafts and ready (VW-14).
- Comments marked addressed (SM-16); mirror scrubbing recorded (LD-21).
- Restore and rebuild split from archive and delete (LD-4, SL-4).
- Network rule has no Allow (D20); rows IN-17 lacks folded in (D21).

Still open:

- No MCP tool watches another session's terminal read-only; the CLI has
  `humata attach --read-only` (FILES_READ_OTHER_SESSION). Agents read
  conversations through SessionService.ReadConversation only.
- Provider import passes the file's bytes in the request; the spec does not
  say how the file reaches the Mac.
- A port forward under Skip while the Mac is away fails as host_unreachable;
  the spec only says "denied saying the Mac is away".
- BD-10 (what a fork's session board starts with) is open in the spec.
- D3, D7, D11, D14, D20 and D21 are provisional or open; AW-*, SEC-8..10 and
  the IN-17 rows may change with them.
- CliAuthService and session-board todos noted in BOUNDARIES.md §9.
- The protos are not compiled yet.
