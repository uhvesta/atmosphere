//! D762: a question an agent's CLI asks on its own
//! screen before its input box exists, such as Copilot's and Claude Code's
//! folder trust. Text typed then goes into the question, not the agent's
//! input, and the Enter that should submit it answers the question:
//! for folder trust, that grants trust the user never gave. The input gate
//! (`input_gate`) looks for such a question before anything is typed, and
//! `screen_ask` (D767) asks it in the Inbox. Only questions seen on a
//! recorded screen are listed.

/// A question the agent's CLI is asking on its screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenQuestion {
    /// Whether to trust the files in the session's folder.
    FolderTrust,
    /// An offer the CLI shows on its own after a turn, such as Claude
    /// Code's "Teach auto mode about your environment?". It takes the keys
    /// like a question: a typed message's Enter would pick its highlighted
    /// choice.
    Offer,
}

impl ScreenQuestion {
    /// Whether the question holds its session at Needs you while it shows.
    /// An offer doesn't (D864): the agent isn't waiting on the person, so
    /// only the input gate refuses text while it is up.
    pub fn needs_you(self) -> bool {
        match self {
            Self::FolderTrust => true,
            Self::Offer => false,
        }
    }
}

/// What each question says on the screen, as `plain` leaves it:
/// - Copilot CLI 1.0.113 / 1.0.115-1 "Confirm folder trust"
/// - Claude Code 2.1.329 workspace "Quick safety check" folder trust
/// - Codex 0.169.0 "Folder access" / "Trust this folder?"
/// - Claude Code 2.1.328's auto-mode offer, as recorded in AGENTS2's live check O1.
const ASKED: &[(&str, ScreenQuestion)] = &[
    (
        "Do you trust the files in this folder?",
        ScreenQuestion::FolderTrust,
    ),
    (
        "Quick safety check: Is this a project you created or one you trust?",
        ScreenQuestion::FolderTrust,
    ),
    (
        "Trust this folder? Codex can read, edit, and run files here",
        ScreenQuestion::FolderTrust,
    ),
    (
        "Teach auto mode about your environment?",
        ScreenQuestion::Offer,
    ),
];

use std::sync::LazyLock;

static NORMALIZED_ASKED: LazyLock<Vec<(String, ScreenQuestion)>> = LazyLock::new(|| {
    ASKED
        .iter()
        .map(|(words, question)| (plain(words), *question))
        .collect()
});

/// The question `screen` (the text the tab shows now) is asking, if any.
pub fn asked_on(screen: &str) -> Option<ScreenQuestion> {
    let shown = plain(screen);
    NORMALIZED_ASKED
        .iter()
        .find(|(words, _)| shown.contains(words.as_str()))
        .map(|(_, question)| *question)
}

/// The kind of interactive prompt visibly shown on an agent CLI's screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptKind {
    /// Folder trust / workspace safety check (Claude, Codex, Copilot).
    FolderTrust,
    /// Tool or shell command permission approval.
    Permission,
    /// MCP server tool permission approval.
    Mcp,
    /// Plan approval prompt.
    Plan,
    /// Interactive multiple-choice or confirmation question.
    Question,
    /// CLI sign-in / authentication choice prompt (e.g., Codex login).
    Login,
    /// First-run CLI onboarding / setup picker (e.g., Claude theme picker).
    Onboarding,
    /// Startup authorization prompt (e.g., Antigravity `agy` "Do you authorize me").
    Authorization,
}

impl PromptKind {
    /// Canonical snapshot `expect: prompt:<kind>` label.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FolderTrust => "folder_trust",
            Self::Permission => "permission",
            Self::Mcp => "mcp",
            Self::Plan => "plan",
            Self::Question => "question",
            Self::Login => "login",
            Self::Onboarding => "onboarding",
            Self::Authorization => "authorization",
        }
    }
}

/// An active interactive prompt parsed from an agent CLI's live screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScreenPrompt {
    pub kind: PromptKind,
    pub choices: Vec<String>,
}

/// Permission approval phrases shown on supported agent CLI screens.
const PERMISSION_PHRASES: &[&str] = &[
    "do you want to proceed?",
    "do you want to make this edit",
    "do you want to create",
    "do you want to run",
    "would you like to run",
    "allow once",
    "allow for this session",
    "allow always",
    "always allow",
    "allow this tool",
    "yes, and don't ask again",
    "approve command?",
    "approve this",
    "approve once",
    "approve for this session",
    "permission required",
    "waiting for your approval",
    "waiting for approval",
    "yes, proceed",
];

/// Plan approval phrases shown on supported agent CLI screens.
const PLAN_PHRASES: &[&str] = &[
    "approve this plan",
    "proceed with this plan",
    "ready to code?",
];

/// Startup authorization phrases shown on supported agent CLI screens
/// (e.g., Antigravity `agy` "Do you authorize me").
const AUTHORIZATION_PHRASES: &[&str] = &[
    "do you authorize",
    "authorize me",
    "authorize antigravity",
    "authorize agy",
    "authorize this workspace",
    "authorize this session",
];

/// General interactive prompt phrases (questions / confirmation footers).
const QUESTION_PHRASES: &[&str] = &[
    "waiting for input",
    "enter to confirm",
    "esc to cancel",
    "press enter to confirm",
    "select an option",
    "choose an option",
];

/// Parses the active interactive prompt on `screen`, returning its kind and
/// any visible choices.
///
/// Offers that do not hold `Needs you` (such as Claude's post-turn auto-mode
/// tip) return `None`.
pub fn prompt_on(screen: &str) -> Option<ScreenPrompt> {
    if let Some(question) = asked_on(screen) {
        return if question.needs_you() {
            Some(ScreenPrompt {
                kind: PromptKind::FolderTrust,
                choices: extract_choices(screen),
            })
        } else {
            None
        };
    }
    let shown = plain(screen);
    if shown.is_empty() {
        return None;
    }
    let lower = shown.to_ascii_lowercase();

    let kind = if lower.contains("mcp server to run tool")
        || (lower.contains("mcp")
            && PERMISSION_PHRASES
                .iter()
                .any(|phrase| lower.contains(phrase)))
    {
        Some(PromptKind::Mcp)
    } else if lower.contains("choose how you want to use codex")
        || (lower.contains("sign in with chatgpt") && lower.contains("sign in"))
    {
        Some(PromptKind::Login)
    } else if lower.contains("choose the text style that looks best with your terminal") {
        Some(PromptKind::Onboarding)
    } else if PLAN_PHRASES.iter().any(|phrase| lower.contains(phrase)) {
        Some(PromptKind::Plan)
    } else if PERMISSION_PHRASES
        .iter()
        .any(|phrase| lower.contains(phrase))
    {
        Some(PromptKind::Permission)
    } else if AUTHORIZATION_PHRASES
        .iter()
        .any(|phrase| lower.contains(phrase))
    {
        Some(PromptKind::Authorization)
    } else if QUESTION_PHRASES.iter().any(|phrase| lower.contains(phrase))
        || has_numbered_choice_selector(&shown)
        || has_confirmation_suffix(&lower)
    {
        Some(PromptKind::Question)
    } else {
        None
    };

    kind.map(|kind| ScreenPrompt {
        kind,
        choices: extract_choices(screen),
    })
}

/// Interactive choice selector: focused first choice (`❯ 1.`, `› 1.`, or `> 1.`)
/// paired with a second numbered choice (`2. `).
fn has_numbered_choice_selector(shown: &str) -> bool {
    (shown.contains("❯ 1.") || shown.contains("› 1.") || shown.contains("> 1."))
        && shown.contains(" 2. ")
}

/// Confirmation prompt suffix at the end of the visible screen.
fn has_confirmation_suffix(lower: &str) -> bool {
    let trimmed = lower.trim_end();
    [
        "(y/n)",
        "[y/n]",
        "(yes/no)",
        "[yes/no]",
        "(y/n)?",
        "[y/n]?",
    ]
    .iter()
    .any(|suffix| trimmed.ends_with(suffix))
}

/// Returns `true` when `screen` visibly shows an active interactive prompt
/// (permission approval, question, folder trust, login, onboarding, or input
/// confirmation) for a supported agent CLI (`claude`, `codex`, `copilot`, `agy`).
///
/// Offers that do not hold `Needs you` (such as Claude's post-turn auto-mode
/// tip) return `false`.
pub fn has_active_prompt(screen: &str) -> bool {
    prompt_on(screen).is_some()
}

/// Extracts visible interactive menu choices from `screen`, handling both
/// numbered menus (`❯ 1. Yes`, `› 1. Allow`, `> 1. Sign in ...`) and
/// unnumbered cursor lists (such as Claude Code's `❯ No, exit` / `Yes, I trust
/// this folder` or its theme picker).
fn extract_choices(screen: &str) -> Vec<String> {
    let numbered = extract_numbered_choices(screen);
    if numbered.len() >= 2 {
        return numbered;
    }
    let unnumbered = extract_cursor_choices(screen);
    if unnumbered.len() >= 2 {
        return unnumbered;
    }
    Vec::new()
}

fn unbox_line(line: &str) -> &str {
    line.trim_matches(|c: char| c.is_whitespace() || ('\u{2500}'..='\u{257f}').contains(&c))
}

fn collapse_spaces(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strip_focus_marker(line: &str) -> (bool, &str) {
    for marker in ["❯ ", "› ", "> "] {
        if let Some(rest) = line.strip_prefix(marker) {
            return (true, rest.trim_start());
        }
    }
    (false, line)
}

fn parse_numbered_item(line: &str) -> Option<(usize, String)> {
    let (_, rest) = strip_focus_marker(line);
    let (digits, after) = rest.split_once(". ")?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let num = digits.parse::<usize>().ok()?;
    let label = collapse_spaces(after);
    if label.is_empty() {
        return None;
    }
    Some((num, label))
}

fn extract_numbered_choices(screen: &str) -> Vec<String> {
    let mut best: Vec<String> = Vec::new();
    let mut current: Vec<String> = Vec::new();
    let mut expected_num = 1usize;

    for raw in screen.lines() {
        let line = unbox_line(raw);
        if let Some((num, label)) = parse_numbered_item(line) {
            if num == 1 {
                if current.len() > best.len() {
                    best = std::mem::take(&mut current);
                } else {
                    current.clear();
                }
                current.push(label);
                expected_num = 2;
            } else if num == expected_num && !current.is_empty() {
                current.push(label);
                expected_num += 1;
            }
        }
    }
    if current.len() > best.len() {
        best = current;
    }
    best
}

fn is_choice_footer(lower: &str) -> bool {
    lower.starts_with("enter to ")
        || lower.starts_with("press enter")
        || lower.starts_with("↑/↓")
        || lower.starts_with("esc to ")
        || lower.starts_with("enter continue")
        || lower.starts_with("syntax theme:")
}

fn extract_cursor_choices(screen: &str) -> Vec<String> {
    let lines: Vec<&str> = screen.lines().collect();
    for (idx, raw) in lines.iter().enumerate() {
        let trimmed = unbox_line(raw);
        let Some(after_cursor) = trimmed.strip_prefix("❯ ") else {
            continue;
        };
        let first = collapse_spaces(after_cursor.trim_start_matches('✔').trim());
        if first.is_empty() {
            continue;
        }
        let mut choices = vec![first];
        for next_raw in &lines[idx + 1..] {
            let next = unbox_line(next_raw);
            if next.is_empty()
                || next.chars().all(|c| c == '╌' || c == '─' || c == '-')
                || is_choice_footer(&next.to_ascii_lowercase())
            {
                break;
            }
            let cleaned = collapse_spaces(next.trim_start_matches('✔').trim());
            if !cleaned.is_empty() {
                choices.push(cleaned);
            }
        }
        if choices.len() >= 2 {
            return choices;
        }
    }
    Vec::new()
}

/// `text` without box drawing, with each run of white space, line breaks
/// included, as one space: a question drawn in a box, or wrapped over rows
/// on a narrow screen, reads the same.
fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    for c in text.chars() {
        if c.is_whitespace() || ('\u{2500}'..='\u{257f}').contains(&c) {
            space = true;
            continue;
        }
        if space && !out.is_empty() {
            out.push(' ');
        }
        space = false;
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const COPILOT_TRUST: &str = "\
 [Current]  Sessions   Issues   Pull requests   Gists
╭──────────────────────────────────────────────────────────╮
│ Confirm folder trust                                     │
│ ╭──────────────────────────────────────────────────────╮ │
│ │ /home/vremote/Humata/remote-flows/sessions/rr-flow2b │ │
│ ╰──────────────────────────────────────────────────────╯ │
│ Do you trust the files in this folder?                   │
│                                                          │
│ ❯ 1. Yes                                                 │
│   2. Yes, and remember this folder for future sessions   │
│   3. No (Esc)                                            │
╰──────────────────────────────────────────────────────────╯";

    const COPILOT_READY: &str = "\
 ❯ Remember this code word: otter.                         18:16
 ● Noted: otter.
 ● MCP Servers reloaded: 2 servers connected
 ~/Humata/remote-flows/sessions/rr-flow3
──────────────────────────────────────────────────────────
❯
──────────────────────────────────────────────────────────
 ← open sidebar · Interactive · Manual Approval · / commands";

    /// Claude Code 2.1.328 after a turn, auto mode on (AGENTS2 O1), and
    /// the same session's input once the offer is gone.
    const CLAUDE_OFFER: &str = "\
⏺ You picked blue, and I wrote blue to colour.txt.

✻ Sautéed for 28s · done 2:12 AM

────────────────────────────────────────────────────────────────
  Teach auto mode about your environment?

  Auto mode works better when it knows your environment. Takes about a minute.

  ❯ 1. Yes
    2. Not now
    3. Don't show again

  Enter to confirm · Esc to cancel
────────────────────────────────────────────────────────────────
❯
────────────────────────────────────────────────────────────────
  ⏵⏵ auto mode on (shift+tab to cycle)";

    const CLAUDE_READY: &str = "\
❯ /clear

────────────────────────────────────────────────────────────────
❯
────────────────────────────────────────────────────────────────
  ⏵⏵ auto mode on (shift+tab to cycle)";

    #[test]
    fn claudes_auto_mode_offer_takes_the_keys_and_its_input_box_does_not() {
        assert_eq!(asked_on(CLAUDE_OFFER), Some(ScreenQuestion::Offer));
        assert_eq!(asked_on(CLAUDE_READY), None);
        assert!(!ScreenQuestion::Offer.needs_you());
        assert!(ScreenQuestion::FolderTrust.needs_you());
    }

    #[test]
    fn copilots_folder_trust_is_a_question_and_its_input_box_is_not() {
        assert_eq!(asked_on(COPILOT_TRUST), Some(ScreenQuestion::FolderTrust));
        assert_eq!(asked_on(COPILOT_READY), None);
    }

    #[test]
    fn a_question_wrapped_over_rows_in_a_narrow_box_is_still_seen() {
        let narrow = "│ Do you trust the files │\n│ in this    folder?     │";
        assert_eq!(asked_on(narrow), Some(ScreenQuestion::FolderTrust));
    }

    #[test]
    fn the_words_split_by_other_text_are_not_the_question() {
        assert_eq!(
            asked_on("printf 'Do you trust the files' 'in this folder?'"),
            None
        );
    }

    #[test]
    fn active_prompt_detects_live_tui_prompts_and_ignores_idle_or_streaming() {
        // Genuine interactive prompts across supported agents.
        assert!(has_active_prompt(COPILOT_TRUST));
        assert!(has_active_prompt(
            "Bash command\n  rm -rf target\nDo you want to proceed?\n❯ 1. Yes\n  2. Yes, and don't ask again\n  3. No"
        ));
        assert!(has_active_prompt(
            "Would you like to run the following command?\n  git status\n› 1. Yes, proceed\n  2. No"
        ));
        assert!(has_active_prompt(
            "Which database should we use?\n❯ 1. Postgres\n  2. SQLite"
        ));
        assert!(has_active_prompt(
            "Run command `cargo test`? [y/N]"
        ));
        assert!(has_active_prompt(
            "Antigravity: Permission required — Allow once / Allow always"
        ));

        // Idle, post-turn offer, or normal streaming output are NOT active prompts.
        assert!(!has_active_prompt(CLAUDE_READY));
        assert!(!has_active_prompt(COPILOT_READY));
        assert!(!has_active_prompt(CLAUDE_OFFER));
        assert!(!has_active_prompt(
            "Compiling humata2_svc_inbox v0.1.0\nRunning 24 tests...\nAll tests passed."
        ));
        assert!(!has_active_prompt(""));
    }

    #[test]
    fn real_recorded_cli_prompts_parse_kind_and_choices() {
        // Claude Code 2.1.329 workspace folder trust.
        let claude_trust = "\
 Accessing workspace:
 /tmp/work
 Quick safety check: Is this a project you created or one you trust? (Like your own code, a well-known open source
 project, or work from your team).
 ❯ No, exit
   Yes, I trust this folder
 Enter to confirm · Esc to cancel";
        assert_eq!(asked_on(claude_trust), Some(ScreenQuestion::FolderTrust));
        let p = prompt_on(claude_trust).expect("claude trust prompt");
        assert_eq!(p.kind, PromptKind::FolderTrust);
        assert_eq!(p.choices, vec!["No, exit", "Yes, I trust this folder"]);

        // Codex 0.169.0 folder trust.
        let codex_trust = "\
  Folder access
  /tmp/work
  Trust this folder? Codex can read, edit, and run files here, subject to your permission settings.
› 1. Trust and continue
  2. Back to Agent Command Center
  enter continue · esc back";
        assert_eq!(asked_on(codex_trust), Some(ScreenQuestion::FolderTrust));
        let p = prompt_on(codex_trust).expect("codex trust prompt");
        assert_eq!(p.kind, PromptKind::FolderTrust);
        assert_eq!(
            p.choices,
            vec!["Trust and continue", "Back to Agent Command Center"]
        );

        // Codex 0.169.0 MCP tool permission.
        let codex_mcp = "\
  Field 1/1
  Allow the screenparse MCP server to run tool \"echo\"?
  text: hi
  › 1. Allow                   Run the tool and continue
    2. Allow for this session  Run the tool and remember this choice for this session
    3. Always allow            Run the tool and remember this choice for future tool calls
    4. Cancel                  Cancel this tool call
  enter to submit | esc to cancel";
        let p = prompt_on(codex_mcp).expect("codex mcp prompt");
        assert_eq!(p.kind, PromptKind::Mcp);
        assert_eq!(p.choices.len(), 4);

        // Codex 0.169.0 command permission.
        let codex_cmd = "\
  Would you like to run the following command?
  $ sed -i '' 's/hello/goodbye/' notes.txt
› 1. Yes, proceed (y)
  2. Yes, and don't ask again for commands that start with `sed` (p)
  3. No, and tell Codex what to do differently (esc)
  Press enter to confirm or esc to cancel";
        let p = prompt_on(codex_cmd).expect("codex cmd prompt");
        assert_eq!(p.kind, PromptKind::Permission);
        assert_eq!(p.choices.len(), 3);

        // Codex 0.169.0 sign-in prompt (uses ASCII `> 1.`).
        let codex_login = "\
  Welcome to Codex, OpenAI's command-line coding agent
  Choose how you want to use Codex.
> 1. Sign in with ChatGPT
  2. Sign in with Device Code
  3. Sign in another way
  Press enter to continue";
        let p = prompt_on(codex_login).expect("codex login prompt");
        assert_eq!(p.kind, PromptKind::Login);
        assert_eq!(
            p.choices,
            vec![
                "Sign in with ChatGPT",
                "Sign in with Device Code",
                "Sign in another way",
            ]
        );

        // Claude Code 2.1.329 first-run theme picker.
        let claude_theme = "\
 Let's get started.
 Choose the text style that looks best with your terminal
 ❯ ✔ Auto (match terminal)
     Dark mode
     Light mode
 ╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌";
        let p = prompt_on(claude_theme).expect("claude theme onboarding");
        assert_eq!(p.kind, PromptKind::Onboarding);
        assert_eq!(
            p.choices,
            vec!["Auto (match terminal)", "Dark mode", "Light mode"]
        );

        // Copilot 1.0.115-1 folder trust.
        let p = prompt_on(COPILOT_TRUST).expect("copilot trust prompt");
        assert_eq!(p.kind, PromptKind::FolderTrust);
        assert_eq!(
            p.choices,
            vec![
                "Yes",
                "Yes, and remember this folder for future sessions",
                "No (Esc)",
            ]
        );
    }

    #[test]
    fn agy_startup_authorization_prompts_classify_as_authorization() {
        let agy_inline = "Antigravity (agy)\nDo you authorize me to work in this workspace? [y/N]";
        let p = prompt_on(agy_inline).expect("agy inline auth prompt");
        assert_eq!(p.kind, PromptKind::Authorization);
        assert_eq!(p.kind.as_str(), "authorization");
        assert!(has_active_prompt(agy_inline));

        let agy_numbered = "\
Antigravity startup
Do you authorize me for this session?
❯ 1. Yes, authorize for this session
  2. No, exit";
        let p = prompt_on(agy_numbered).expect("agy numbered auth prompt");
        assert_eq!(p.kind, PromptKind::Authorization);
        assert_eq!(
            p.choices,
            vec!["Yes, authorize for this session", "No, exit"]
        );

        let agy_hotkey = "Authorize Antigravity for /tmp/work? (y) Yes / (n) No";
        let p = prompt_on(agy_hotkey).expect("agy hotkey auth prompt");
        assert_eq!(p.kind, PromptKind::Authorization);
    }
}
