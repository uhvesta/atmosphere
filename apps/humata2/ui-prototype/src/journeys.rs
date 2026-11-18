//! Hardcoded product journeys for design review, independent of production services.
#[derive(Clone, Copy)]
pub(super) struct Screen {
    pub(super) title: &'static str,
    pub(super) description: &'static str,
    pub(super) rows: &'static [&'static str],
    pub(super) action: &'static str,
}
pub(super) struct Journey {
    pub(super) key: &'static str,
    pub(super) title: &'static str,
    pub(super) screens: Vec<Screen>,
    pub(super) at: usize,
    pub(super) selected: Vec<usize>,
    pub(super) error: bool,
}
pub(super) const CATALOG: &[(&str, &str, &str)] = &[
    (
        "channel",
        "Create a channel",
        "Repos, machines, branch naming, access defaults and an all-remote workspace.",
    ),
    (
        "repos",
        "Add and remove repositories",
        "Include a repo in a channel, choose sessions to clone it into, and inspect unpushed work before removal.",
    ),
    (
        "transfer",
        "Files and interrupted transfers",
        "Two machine panes, copy or move, collision choices, paused progress, retry and checksum confirmation.",
    ),
    (
        "provider",
        "Add a provider machine",
        "Template parameters, interactive login, connectivity checks, provisioning and retry.",
    ),
    (
        "move",
        "Move a session",
        "Destination, unpushed changes, transfer progress, resume and source retention.",
    ),
    (
        "pr",
        "Review pull requests",
        "Read-only checkout, draft comments, changed commits, outdated anchors and explicit GitHub submission.",
    ),
    (
        "spec",
        "Iterate on a specification",
        "Rendered Markdown, diagrams, passage comments, agent feedback and stale anchors.",
    ),
    (
        "prepared",
        "Agent-prepared walkthrough",
        "Open, Later, Not now, step through changes, Revert and reopen from the Journal.",
    ),
    (
        "lost",
        "Machine lost permanently",
        "Review cached work, distinguish pushed from unrecoverable changes, and rebuild on another machine.",
    ),
    (
        "restart",
        "Reboot and crash recovery",
        "App reconnect, holder survival, daemon restart, conversation resume and shell restart.",
    ),
    (
        "storage",
        "Storage and retention",
        "Space by machine, archive preview, Trash, undo and irreversible expiration.",
    ),
    (
        "access",
        "Access rules and away mode",
        "Allow, Ask me and Never; scoped approvals, expired requests and denied actions.",
    ),
    (
        "skills",
        "Synchronize skills",
        "Compare hashes, choose files, handle an offline machine and verify after reconnect.",
    ),
    (
        "palette",
        "Command palette and navigation",
        "Search across sessions, channels, work and machines; switch without losing your open view.",
    ),
    (
        "settings",
        "Agent and appearance settings",
        "Agent commands, turn hooks, resume support, defaults, fonts and terminal scrollback.",
    ),
];
fn screen(
    title: &'static str,
    description: &'static str,
    rows: &'static [&'static str],
    action: &'static str,
) -> Screen {
    Screen {
        title,
        description,
        rows,
        action,
    }
}
impl Journey {
    pub(super) fn new(key: &'static str) -> Self {
        let title = CATALOG
            .iter()
            .find(|(id, _, _)| *id == key)
            .map(|(_, title, _)| *title)
            .unwrap_or("Workspace");
        let screens = match key {
            "channel" => vec![
                screen(
                    "New channel",
                    "A channel groups related repositories, machines and agent sessions. It can be entirely remote.",
                    &[
                        "Name · payments-rewrite",
                        "Repos · acme/ledger, acme/api, acme/docs",
                        "Machines · devbox-1, gpu-box",
                        "Branch pattern · uhvesta/{work-slug}",
                    ],
                    "Review channel",
                ),
                screen(
                    "Review channel setup",
                    "Each selected machine gets a read-only main clone. Sessions get their own branches.",
                    &[
                        "devbox-1 · 3 repositories",
                        "gpu-box · 3 repositories",
                        "Agent access · Ask before changing another session",
                        "Local clones · none",
                    ],
                    "Create channel",
                ),
                screen(
                    "Channel ready",
                    "All repository clones are available. Start an agent session on either machine.",
                    &["#payments-rewrite", "devbox-1 · Ready", "gpu-box · Ready"],
                    "Done",
                ),
            ],
            "repos" => vec![
                screen(
                    "Repositories & machines",
                    "Select the repositories and session clones to include. Existing sessions are left alone unless selected.",
                    &[
                        "acme/ledger · This Mac, devbox-1",
                        "acme/api · This Mac, devbox-1",
                        "acme/docs · This Mac, gpu-box",
                        "Add acme/billing-sdk",
                    ],
                    "Add repository",
                ),
                screen(
                    "Include in sessions",
                    "The new repository gets its own branch in each selected session.",
                    &[
                        "Ledger schema migration · This Mac",
                        "Try Postgres partitioning · devbox-1",
                        "Review migration · read-only parent context",
                    ],
                    "Clone into selected sessions",
                ),
                screen(
                    "Repository added",
                    "The new main clones and selected session clones are ready.",
                    &[
                        "acme/billing-sdk · 2 machines",
                        "Ledger schema migration · own clone",
                        "Try Postgres partitioning · own clone",
                    ],
                    "Review removal",
                ),
                screen(
                    "Remove repository?",
                    "Unpushed work exists in one session. Keep a recoverable copy before removing the repository.",
                    &[
                        "Ledger schema migration · 2 unpushed commits",
                        "Keep snapshot in Trash · 30 days",
                        "Other sessions · no unpushed work",
                    ],
                    "Remove and keep snapshot",
                ),
                screen(
                    "Repository removed",
                    "The recoverable snapshot remains in this machine’s Trash.",
                    &[
                        "acme/billing-sdk · removed",
                        "Trash · 1 recoverable snapshot",
                    ],
                    "Done",
                ),
            ],
            "transfer" => vec![
                screen(
                    "Files · This Mac ⇄ gpu-box",
                    "Select files in the left pane. Copy keeps the originals; Move deletes the source only after destination checksums match.",
                    &[
                        "Left · This Mac /Downloads/fixtures",
                        "datasets/train.parquet · 1.8 GB",
                        "datasets/eval.parquet · 310 MB",
                        "Right · gpu-box /Humata/ml/shared",
                    ],
                    "Copy selected files →",
                ),
                screen(
                    "File already exists",
                    "Choose how to handle train.parquet at the destination.",
                    &[
                        "Keep both · train (2).parquet",
                        "Replace destination",
                        "Skip existing file",
                    ],
                    "Copy with selected policy",
                ),
                screen(
                    "Transfer paused · gpu-box disconnected",
                    "1.4 of 2.1 GB confirmed. The source is intact. Reconnect to continue from the last verified chunk.",
                    &[
                        "train.parquet · 1.4 / 1.8 GB",
                        "eval.parquet · waiting",
                        "Source files · unchanged",
                    ],
                    "Reconnect gpu-box",
                ),
                screen(
                    "Verifying transferred files",
                    "Both files arrived. A transfer is complete only when destination checksums match.",
                    &[
                        "train.parquet · checksum matched",
                        "eval.parquet · checksum matched",
                    ],
                    "Finish transfer",
                ),
                screen(
                    "Transfer complete",
                    "Both files are present on gpu-box. Originals remain on This Mac.",
                    &[
                        "2 files · 2.1 GB verified",
                        "Destination · /Humata/ml/shared/datasets",
                    ],
                    "Done",
                ),
            ],
            "provider" => vec![
                screen(
                    "Add machine",
                    "Choose an existing SSH machine or a provider template. Credentials remain local.",
                    &[
                        "Provider · Cloud VM",
                        "Template · GPU runner",
                        "Name · gpu-box-2",
                        "Region · us-west",
                        "Instance · 2 GPUs",
                    ],
                    "Continue to login",
                ),
                screen(
                    "Provider login required",
                    "Complete the provider’s device login before provisioning. You can cancel and return later.",
                    &[
                        "Account · not connected",
                        "Device code · HUMATA-DEMO",
                        "Permissions · create and delete this VM",
                    ],
                    "Complete demo login",
                ),
                screen(
                    "Review machine",
                    "This VM may incur provider charges in the real product. The prototype creates nothing.",
                    &[
                        "gpu-box-2 · us-west · 2 GPUs",
                        "SSH · through provider connector",
                        "Channel · model-training",
                        "Agent tools · use channel defaults",
                    ],
                    "Create machine",
                ),
                screen(
                    "Machine ready",
                    "SSH, daemon and repository checks passed. The machine is ready for sessions.",
                    &[
                        "Provider instance · ready",
                        "SSH · connected",
                        "humatad · ready",
                        "Repos and skills · synchronized",
                    ],
                    "Done",
                ),
            ],
            "move" => vec![
                screen(
                    "Move Ledger schema migration",
                    "The session keeps its identity, work links and conversation. Select a destination machine in this channel.",
                    &[
                        "Source · This Mac",
                        "devbox-1 · online",
                        "gpu-box · online",
                        "devbox-2 · disconnected, unavailable",
                    ],
                    "Review move",
                ),
                screen(
                    "Unpushed work travels with the session",
                    "The source remains recoverable until the destination is verified.",
                    &[
                        "2 unpushed commits",
                        "3 modified files",
                        "Conversation · resumable",
                        "Target branch · uhvesta/ledger-schema",
                    ],
                    "Move session",
                ),
                screen(
                    "Destination verified",
                    "Files and conversation arrived. Resume the agent on devbox-1 before cleaning up the source.",
                    &[
                        "Checksums · matched",
                        "Original conversation · restored",
                        "Source copy · retained",
                    ],
                    "Resume on devbox-1",
                ),
                screen(
                    "Session moved",
                    "The agent is attached on devbox-1. The original copy is recoverable from This Mac’s Trash.",
                    &[
                        "Agent · working",
                        "Machine · devbox-1",
                        "Original source · recoverable for 30 days",
                    ],
                    "Done",
                ),
            ],
            "pr" => vec![
                screen(
                    "Start pull request review",
                    "Select one or more pull requests. The review checkout is read-only until you explicitly allow edits.",
                    &[
                        "acme/ledger #42 · Add double-entry ledger",
                        "acme/api #91 · Post invoices through ledger",
                        "Base · main",
                        "Head · 4a29b7d",
                    ],
                    "Open review",
                ),
                screen(
                    "Review · 2 pull requests",
                    "Comments are private drafts. Nothing is posted to GitHub until you submit.",
                    &[
                        "migrations/004_ledger.sql · +42",
                        "src/billing/service.rs · +18 −6",
                        "Draft · Add an account_id index",
                        "Draft · Check duplicate webhook delivery",
                    ],
                    "Check updated commits",
                ),
                screen(
                    "Pull request changed",
                    "New commits arrived. One draft’s line moved; inspect the old and new context before submitting.",
                    &[
                        "Head · 4a29b7d → 78c901a",
                        "Index comment · reattached",
                        "Webhook comment · outdated, needs review",
                    ],
                    "Review outdated comment",
                ),
                screen(
                    "Submit review",
                    "Choose Comment, Approve or Request changes. This is the only step that publishes your review.",
                    &[
                        "Comment · selected",
                        "Approve",
                        "Request changes",
                        "2 comments · ready to submit",
                    ],
                    "Submit review to GitHub",
                ),
                screen(
                    "Review submitted",
                    "Your two comments are attached to the reviewed commit. A retry will not duplicate them.",
                    &["acme/ledger #42 · submitted", "acme/api #91 · submitted"],
                    "Done",
                ),
            ],
            "spec" => vec![
                screen(
                    "Specification · billing-v2/SPEC.md",
                    "Read the rendered document, select a passage and leave feedback for its agent session.",
                    &[
                        "Passage · Existing invoices remain readable",
                        "Comment · Add a rollback acceptance test",
                        "Target · Claude Code / Ledger schema migration",
                    ],
                    "Send passage comment",
                ),
                screen(
                    "Agent updated the specification",
                    "The commented passage changed. Keep the original quotation beside the updated document.",
                    &[
                        "Original · Existing invoices remain readable",
                        "Updated · Existing invoices and refunds remain readable",
                        "Comment · needs your review",
                    ],
                    "Review changed passage",
                ),
                screen(
                    "Resolve comment",
                    "Resolving a comment keeps its history. Reopen it if the acceptance criterion is still missing.",
                    &[
                        "Rollback acceptance test · added",
                        "Comment · resolved",
                        "History · original passage and agent revision kept",
                    ],
                    "Done",
                ),
            ],
            "prepared" => vec![
                screen(
                    "Claude Code prepared a review walkthrough",
                    "Prepared views never replace your workspace without your choice. Later and Not now keep it in the Journal.",
                    &[
                        "Ledger migration walkthrough",
                        "3 files · 2 repositories",
                        "Summary · schema, posting path and rollback",
                    ],
                    "Open walkthrough",
                ),
                screen(
                    "Step 1 of 3 · Schema",
                    "The walkthrough temporarily opens the relevant diff and selection.",
                    &[
                        "migrations/004_ledger.sql · lines 1–13",
                        "Agent note · integer postings and balance constraint",
                        "Previous workspace · kept",
                    ],
                    "Next step",
                ),
                screen(
                    "Step 2 of 3 · Posting path",
                    "Inspect the calling code before continuing.",
                    &[
                        "src/billing/service.rs · lines 82–99",
                        "Agent note · same transaction boundary",
                    ],
                    "Revert to previous workspace",
                ),
                screen(
                    "Previous workspace restored",
                    "Your open pane and terminal tab are restored. The walkthrough is still in the Journal.",
                    &[
                        "Journal · Ledger migration walkthrough",
                        "State · available to reopen",
                    ],
                    "Done",
                ),
            ],
            "lost" => vec![
                screen(
                    "devbox-2 is unreachable",
                    "If this machine is permanently lost, its pushed commits can be cloned again. Unsynced files may be unavailable.",
                    &[
                        "Runner autoscaling · cached transcript available",
                        "Last snapshot · 10 minutes ago",
                        "Pushed commits · available",
                        "2 unsynced files · not recoverable from this snapshot",
                    ],
                    "Review recovery options",
                ),
                screen(
                    "Rebuild on another machine",
                    "Rebuild uses the last available snapshot. It cannot recover changes that never reached the host or a Git remote.",
                    &[
                        "Target · devbox-1",
                        "Restore · pushed commits and cached conversation",
                        "Unavailable · 2 unsynced files",
                        "Lost machine record · kept for now",
                    ],
                    "Rebuild available work",
                ),
                screen(
                    "Rebuilt with missing work noted",
                    "Inspect what is missing before starting the agent. Forgetting the old machine is separate from recovery.",
                    &[
                        "Session · rebuilt on devbox-1",
                        "Missing work · recorded in session activity",
                        "Old machine · still listed",
                    ],
                    "Done",
                ),
            ],
            "restart" => vec![
                screen(
                    "Welcome back",
                    "The app reattaches to surviving holders. An app restart alone does not restart the agent process.",
                    &[
                        "This Mac · terminal holder alive",
                        "devbox-1 · reconnecting on your request",
                        "gpu-box · cached screen available",
                    ],
                    "Reattach terminals",
                ),
                screen(
                    "One machine rebooted",
                    "Its agent process ended. Resume uses the pinned conversation; plain shells start fresh in the same folder.",
                    &[
                        "Claude Code · conversation available",
                        "zsh · fresh shell required",
                        "Scrollback · preserved",
                    ],
                    "Resume agent and reopen shell",
                ),
                screen(
                    "Sessions restored",
                    "Live output catches up from the last confirmed cursor. Any output gap is disclosed.",
                    &[
                        "Agent · resumed with original conversation",
                        "Shell · new process, same folder",
                        "Output · 24 lines not retained before crash",
                    ],
                    "Done",
                ),
            ],
            "storage" => vec![
                screen(
                    "Storage · This Mac",
                    "Review what consumes disk space before choosing cleanup.",
                    &[
                        "Active session clones · 12.4 GB",
                        "Archived summaries · 38 MB",
                        "Trash · 4.2 GB",
                        "Retention · off by default",
                    ],
                    "Preview cleanup",
                ),
                screen(
                    "Archive preview",
                    "Only the selected sessions are affected. Unpushed work stays recoverable.",
                    &[
                        "Flaky test hunt · 1.2 GB · no unpushed work",
                        "Old dashboard experiment · 2.1 GB · snapshot required",
                        "Keep conversations and review history",
                    ],
                    "Archive selected sessions",
                ),
                screen(
                    "Cleanup complete",
                    "3.3 GB freed. Snapshots can be restored from Trash for 30 days.",
                    &[
                        "Conversation and diff summaries · kept",
                        "Trash snapshots · 30 days",
                        "Retention rule · still off",
                    ],
                    "Inspect Trash",
                ),
                screen(
                    "Trash",
                    "Emptying Trash permanently removes these snapshots. This step requires a separate confirmation in the real UI.",
                    &[
                        "Old dashboard experiment · 29 days remaining",
                        "Deleted repository clone · 12 days remaining",
                    ],
                    "Keep Trash and finish",
                ),
            ],
            "access" => vec![
                screen(
                    "Agent access · #payments-rewrite",
                    "Rules apply to this channel. Agents cannot edit these rules or grant themselves more access.",
                    &[
                        "Own files · Allow",
                        "Other session’s files · Ask me",
                        "Edit others’ tasks or steps · Ask me",
                        "Change access rules · Never",
                    ],
                    "Review approval",
                ),
                screen(
                    "Codex requests another session’s files",
                    "Inspect the requester, exact target and reason before deciding.",
                    &[
                        "Requester · API rate limiter",
                        "Target · Ledger schema migration / src/ledger.rs",
                        "Reason · confirm transaction boundary",
                        "Grant · this read only",
                    ],
                    "Allow once",
                ),
                screen(
                    "Approval applied",
                    "Only the requested read was allowed. Channel defaults remain unchanged.",
                    &[
                        "Request · approved once",
                        "Audit · actor, scope and decision kept",
                        "Standing rules · unchanged",
                    ],
                    "Review away behavior",
                ),
                screen(
                    "While the Mac is away",
                    "Low-risk operations can follow explicit standing rules. Other requests wait or are refused when no user can answer.",
                    &[
                        "Own work edits · journaled",
                        "Requests needing approval · waiting",
                        "Expired request · no longer actionable",
                        "Never actions · denied with reason",
                    ],
                    "Done",
                ),
            ],
            "skills" => vec![
                screen(
                    "Skills sync · #payments-rewrite",
                    "Compare content hashes before replacing files on another machine.",
                    &[
                        "This Mac /skills/review.md · changed",
                        "devbox-1 /skills/review.md · older",
                        "gpu-box · disconnected",
                    ],
                    "Choose sync targets",
                ),
                screen(
                    "Review replacements",
                    "Only selected files and targets are synchronized.",
                    &[
                        "devbox-1 · review.md",
                        "gpu-box · wait for reconnect",
                        "Existing files · keep recoverable previous versions",
                    ],
                    "Synchronize",
                ),
                screen(
                    "Sync paused on gpu-box",
                    "devbox-1 is verified. gpu-box waits for a manual reconnect.",
                    &["devbox-1 · hash matched", "gpu-box · disconnected"],
                    "Reconnect and continue",
                ),
                screen(
                    "Skills synchronized",
                    "Every selected file has the expected content hash on its destination.",
                    &["devbox-1 · verified", "gpu-box · verified"],
                    "Done",
                ),
            ],
            "palette" => vec![
                screen(
                    "Find or run a command",
                    "Search sessions, channels, milestones, tasks and machines from one place.",
                    &[
                        "Session · Ledger schema migration",
                        "Milestone · Ship the new ledger",
                        "Channel · #payments-rewrite",
                        "Machine · devbox-1",
                    ],
                    "Open selected result",
                ),
                screen(
                    "Navigation preserves your workspace",
                    "Returning restores the open pane, terminal tab and review drafts.",
                    &[
                        "Previous session · kept",
                        "Diff drafts · kept",
                        "Terminal · same process",
                        "Scope · shown in breadcrumb",
                    ],
                    "Done",
                ),
            ],
            _ => vec![
                screen(
                    "Settings · Agents",
                    "Each agent defines its executable, context support and turn-end source.",
                    &[
                        "Claude Code · claude · hooks · resumable",
                        "Codex · codex · output fallback · resumable",
                        "OpenCode · opencode · no context forks",
                        "Shell · zsh · no turns",
                    ],
                    "Review defaults",
                ),
                screen(
                    "Settings · Appearance & terminal",
                    "Preview settings before saving. The prototype keeps them only for this run.",
                    &[
                        "Theme · dark",
                        "UI density · compact",
                        "Terminal font · JetBrains Mono 12",
                        "Scrollback · 100,000 lines",
                        "Default agent · Claude Code",
                    ],
                    "Save preferences",
                ),
                screen(
                    "Preferences saved",
                    "New sessions use the selected defaults. Existing sessions keep their current agent and conversation.",
                    &[
                        "Agent defaults · saved",
                        "Appearance · applied",
                        "Existing sessions · unchanged",
                    ],
                    "Done",
                ),
            ],
        };
        Self {
            key,
            title,
            screens,
            at: 0,
            selected: vec![0],
            error: false,
        }
    }
    pub(super) fn next(&mut self) {
        if self.at + 1 < self.screens.len() {
            self.at += 1;
            self.selected = vec![0];
            self.error = false;
        }
    }
}
