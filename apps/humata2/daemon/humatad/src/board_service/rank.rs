//! Fractional ranks (D155): strings over `a`..`z` compared as text, with a
//! new one always possible between two others.

/// Digits are `a` (0) to `z` (25); an open top end counts as 26.
const BASE: i32 = 26;

fn digit(byte: u8) -> i32 {
    i32::from(byte.saturating_sub(b'a')).min(BASE - 1)
}

fn letter(digit: i32) -> u8 {
    b'a' + u8::try_from(digit.clamp(0, BASE - 1)).unwrap_or(0)
}

fn middle(low: &[u8], high: Option<&[u8]>) -> Vec<u8> {
    if let Some(high) = high {
        // Digits both share stay; the rest is the middle of what's left
        // (a missing low digit counts as `a`).
        let mut shared = 0;
        while shared < high.len() && low.get(shared).copied().unwrap_or(b'a') == high[shared] {
            shared += 1;
        }
        if shared > 0 {
            let mut out = high[..shared].to_vec();
            out.extend(middle(low.get(shared..).unwrap_or(&[]), Some(&high[shared..])));
            return out;
        }
    }
    let below = low.first().map_or(0, |byte| digit(*byte));
    let above = high.and_then(|high| high.first()).map_or(BASE, |byte| digit(*byte));
    if above - below > 1 {
        return vec![letter((below + above + 1) / 2)];
    }
    match high {
        // The digits are neighbours and `high` goes on: its first digit
        // alone is above `low` and below `high`.
        Some(high) if high.len() > 1 => vec![high[0]],
        _ => {
            let mut out = vec![letter(below)];
            out.extend(middle(low.get(1..).unwrap_or(&[]), None));
            out
        }
    }
}

/// A rank after `low` and before `high`; an empty `low` is the start and an
/// empty `high` the end. Ranks made here never end in `a`, so there is
/// always room before them. Given `low` not below `high`, which no write of
/// ours makes, it returns something after `low`.
pub(super) fn between(low: &str, high: &str) -> String {
    let high_bytes = (!high.is_empty()).then_some(high.as_bytes());
    let bytes = if high_bytes.is_some_and(|high| low.as_bytes() >= high) {
        let mut out = low.as_bytes().to_vec();
        out.push(letter(BASE / 2));
        out
    } else {
        middle(low.as_bytes(), high_bytes)
    };
    bytes.into_iter().map(char::from).collect()
}

#[cfg(test)]
mod tests {
    use super::between;

    fn lies_between(low: &str, high: &str) {
        let made = between(low, high);
        assert!(made.as_str() > low, "{made} after {low}");
        if !high.is_empty() {
            assert!(made.as_str() < high, "{made} before {high}");
        }
        assert!(!made.ends_with('a'));
        assert!(made.bytes().all(|byte| byte.is_ascii_lowercase()));
    }

    #[test]
    fn an_open_range_has_a_rank() {
        lies_between("", "");
    }

    #[test]
    fn a_rank_goes_before_the_first_and_after_the_last() {
        lies_between("", "n");
        lies_between("", "b");
        lies_between("", "an");
        lies_between("z", "");
        lies_between("zzn", "");
    }

    #[test]
    fn neighbouring_ranks_still_have_room_between() {
        lies_between("a", "b");
        lies_between("an", "b");
        lies_between("n", "o");
        lies_between("n", "on");
        lies_between("nz", "o");
        lies_between("abc", "abd");
    }

    #[test]
    fn repeated_insertion_at_one_spot_keeps_ordering() {
        let high = "n".to_owned();
        let mut low = String::new();
        for _ in 0..200 {
            let made = between(&low, &high);
            assert!(made > low && made < high);
            low = made;
        }
        let mut high = "n".to_owned();
        for _ in 0..200 {
            let made = between("", &high);
            assert!(made.as_str() > "" && made < high);
            high = made;
        }
    }

    #[test]
    fn ranks_made_in_order_sort_in_order() {
        let mut ranks = vec![between("", "")];
        for _ in 0..50 {
            let last = ranks.last().cloned().unwrap_or_default();
            ranks.push(between(&last, ""));
        }
        let mut sorted = ranks.clone();
        sorted.sort();
        assert_eq!(ranks, sorted);
    }

    #[test]
    fn a_range_that_is_backwards_still_gives_something_after_low() {
        assert!(between("n", "b").as_str() > "n");
        assert!(between("n", "n").as_str() > "n");
    }
}
