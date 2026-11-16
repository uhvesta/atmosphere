//! Item keys (D155): each board's items are numbered under a prefix made
//! from its channel's name when the first item is made, and kept when the
//! channel is renamed.

use humata2_store::{PrefixClaim, Store, StoreError};

/// The prefix the Global board's items use.
const GLOBAL: &str = "GLB";

/// The owner the Global board's prefix is kept under.
pub(super) const GLOBAL_OWNER: &str = "global";

/// How many candidates are tried before giving up.
const TRIES: usize = 10_000;

/// The prefixes to try for `name`, best first: its first three ASCII
/// letters or digits, then those plus its next one, and on, then the
/// first three with 2, 3, ... after them. Fewer than two such characters
/// make it `CH`.
pub(super) fn candidates(name: &str) -> Vec<String> {
    let alnum: Vec<char> = name
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_uppercase())
        .collect();
    let base: String = if alnum.len() < 2 {
        "CH".to_owned()
    } else {
        alnum.iter().take(3).collect()
    };
    let mut list = vec![base.clone()];
    if alnum.len() > 3 {
        for length in 4..=alnum.len() {
            list.push(alnum.iter().take(length).collect());
        }
    }
    list.extend((2..TRIES).map(|n| format!("{base}{n}")));
    list
}

/// The prefix `owner` (a channel id, or the Global board's) numbers its
/// items under, claiming one for the channel `name` on the first call.
pub(super) fn prefix_of(store: &Store, owner: &str, name: &str) -> Result<String, StoreError> {
    if let Some(prefix) = store.board_prefix(owner)? {
        return Ok(prefix);
    }
    let wanted = if owner == GLOBAL_OWNER { GLOBAL } else { name };
    let tries = if owner == GLOBAL_OWNER {
        let mut list = vec![GLOBAL.to_owned()];
        list.extend((2..TRIES).map(|n| format!("{GLOBAL}{n}")));
        list
    } else {
        candidates(wanted)
    };
    for candidate in tries {
        match store.claim_board_prefix(owner, &candidate)? {
            PrefixClaim::Claimed => return Ok(candidate),
            PrefixClaim::AlreadyYours(mine) => return Ok(mine),
            PrefixClaim::TakenByAnother => {}
        }
    }
    Err(StoreError::NoBoardPrefix {
        owner: owner.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::candidates;

    #[test]
    fn a_name_gives_its_first_three_characters_uppercased() {
        assert_eq!(candidates("Payments")[0], "PAY");
    }

    #[test]
    fn a_taken_prefix_lengthens_with_the_next_characters_then_digits() {
        let list = candidates("Pay-roll!");
        assert_eq!(&list[..3], ["PAY", "PAYR", "PAYRO"]);
        assert_eq!(list[4], "PAYROLL");
        assert_eq!(list[5], "PAY2");
    }

    #[test]
    fn a_name_with_fewer_than_two_usable_characters_is_ch() {
        assert_eq!(candidates("é-1")[0], "CH");
        assert_eq!(candidates("")[0], "CH");
        assert_eq!(candidates("")[1], "CH2");
    }

    #[test]
    fn a_short_name_goes_straight_to_digits() {
        assert_eq!(&candidates("ab")[..2], ["AB", "AB2"]);
    }
}
