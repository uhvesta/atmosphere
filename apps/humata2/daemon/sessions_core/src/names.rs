use crate::error::NameError;

const MAX_SLUG: usize = 40;
const FALLBACK_SLUG: &str = "session";
/// The folder Desk sessions live under (D182.2); no channel may take it.
const DESK_FOLDER: &str = "desk";

/// The channel id, and the folder name under the channels folder, every Desk
/// session lives under (D182.1). No channel may take this name.
pub const DESK_CHANNEL: &str = "desk";

/// The channel name a Desk session's carry stages under, since a Desk
/// session has no channel folder of its own.
pub const CHANNELS_ROOT: &str = "@root";

/// Lowercase, runs of non-alphanumerics become one `-`, no leading or
/// trailing `-` (CH-2).
pub fn channel_folder(name: &str) -> Result<String, NameError> {
    let folder = hyphenate(name);
    if folder.is_empty() {
        return Err(NameError::NoLettersOrDigits {
            name: name.to_string(),
        });
    }
    if folder == DESK_FOLDER {
        return Err(NameError::Reserved {
            name: name.to_string(),
            folder,
        });
    }
    Ok(folder)
}

/// The slug of a session title: at most 40 characters, cut at a word
/// boundary (FC-13). An empty result becomes `session`.
pub fn branch_slug(title: &str) -> String {
    let full = hyphenate(title);
    let slug = if full.len() <= MAX_SLUG {
        full
    } else {
        // `full` is ASCII, so byte indexes are character indexes.
        let cut = if full.as_bytes()[MAX_SLUG] == b'-' {
            MAX_SLUG
        } else {
            full[..MAX_SLUG].rfind('-').unwrap_or(MAX_SLUG)
        };
        full[..cut].trim_matches('-').to_string()
    };
    if slug.is_empty() {
        FALLBACK_SLUG.to_string()
    } else {
        slug
    }
}

/// The branch a session uses and the session's own folder name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Branch {
    /// The prefix, the slug and any suffix.
    pub name: String,
    /// The slug and any suffix, without the prefix.
    pub folder: String,
}

/// `prefix + slug`, or with `-2`, `-3`, … while `taken` says the name is
/// in use (FC-14).
pub fn pick_branch(prefix: &str, slug: &str, taken: impl Fn(&str) -> bool) -> Branch {
    let mut folder = slug.to_string();
    let mut n = 2u32;
    while taken(&format!("{prefix}{folder}")) {
        folder = format!("{slug}-{n}");
        n += 1;
    }
    Branch {
        name: format!("{prefix}{folder}"),
        folder,
    }
}

/// Whether git accepts `name` as a branch name (the rules of
/// `git check-ref-format --branch`, for names that are not `-`-led).
pub fn valid_branch_name(name: &str) -> bool {
    if name.is_empty()
        || name == "@"
        || name.starts_with('-')
        || name.starts_with('/')
        || name.ends_with('/')
        || name.ends_with('.')
        || name.contains("..")
        || name.contains("//")
        || name.contains("@{")
    {
        return false;
    }
    if name.chars().any(|c| c.is_control() || c.is_whitespace() || "~^:?*[\\".contains(c)) {
        return false;
    }
    name.split('/').all(|part| !part.starts_with('.') && !part.ends_with(".lock"))
}

/// Whether `prefix` can begin a valid branch name; an empty one can.
pub fn valid_prefix(prefix: &str) -> bool {
    prefix.is_empty() || valid_branch_name(&format!("{prefix}x"))
}

fn hyphenate(text: &str) -> String {
    let mut out = String::new();
    let mut pending = false;
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            if pending && !out.is_empty() {
                out.push('-');
            }
            pending = false;
            out.push(c.to_ascii_lowercase());
        } else {
            pending = true;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_names_follow_git_rules() {
        for good in ["a", "me/fix-it", "team/refunds", "v1.2/x"] {
            assert!(valid_branch_name(good), "{good}");
        }
        for bad in ["", "no..dots", "a b", "-x", "x/", "/x", "a//b", "x.lock", "a/.b", "@", "a@{b", "a~b", "a:b"] {
            assert!(!valid_branch_name(bad), "{bad}");
        }
        assert!(valid_prefix("") && valid_prefix("me/") && !valid_prefix("a b/") && !valid_prefix("/"));
    }

    #[test]
    fn channel_folder_hyphenates() {
        assert_eq!(
            channel_folder("  Payments  Rewrite!! ").unwrap(),
            "payments-rewrite"
        );
        assert_eq!(channel_folder("ml_training/2").unwrap(), "ml-training-2");
    }

    #[test]
    fn channel_folder_rejects_empty() {
        assert!(channel_folder("  --  ").is_err());
        assert!(channel_folder("").is_err());
    }

    #[test]
    fn channel_folder_refuses_the_desk_folder() {
        assert!(matches!(
            channel_folder(" Desk "),
            Err(NameError::Reserved { .. })
        ));
        assert_eq!(channel_folder("Desk work").unwrap(), "desk-work");
    }

    #[test]
    fn slug_basic_and_fallback() {
        assert_eq!(branch_slug("Fix refund rounding"), "fix-refund-rounding");
        assert_eq!(branch_slug("???"), "session");
    }

    #[test]
    fn slug_cuts_at_word_boundary() {
        let slug = branch_slug("migrate the ledger schema to the new double entry format today");
        assert!(slug.len() <= 40);
        assert_eq!(slug, "migrate-the-ledger-schema-to-the-new");
        // A cut exactly on a boundary keeps the whole word.
        let exact = format!("{} tail", "a".repeat(40));
        assert_eq!(branch_slug(&exact), "a".repeat(40));
    }

    #[test]
    fn slug_hard_cuts_one_long_word() {
        assert_eq!(branch_slug(&"x".repeat(60)), "x".repeat(40));
    }

    #[test]
    fn pick_branch_adds_suffix_only_when_taken() {
        let free = pick_branch("me/", "fix", |_| false);
        assert_eq!(
            free,
            Branch {
                name: "me/fix".into(),
                folder: "fix".into()
            }
        );
        let b = pick_branch("me/", "fix", |n| n == "me/fix" || n == "me/fix-2");
        assert_eq!(
            b,
            Branch {
                name: "me/fix-3".into(),
                folder: "fix-3".into()
            }
        );
        assert_eq!(pick_branch("", "fix", |n| n == "fix").name, "fix-2");
    }
}
