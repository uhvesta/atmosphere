//! The terminal's identity clients see (D271): the tab and the generation
//! of the holder process behind it. A revived tab has a new holder whose
//! output offsets start again, so a client that sees a new identity drops
//! the screen it kept instead of feeding it the new process's offsets.

use uuid::Uuid;

/// Names the identities this daemon makes, so they stay its own.
const NAMESPACE: &[u8] = b"humata2.terminal";

/// The identity for `tab_id`'s holder of `generation`, as a hyphenated
/// UUID. The same inputs always give the same text, and a new generation
/// or tab gives a different one. Name-based and stable across restarts, so
/// a daemon that reattaches the same live holder shows the same identity.
pub fn terminal_uuid(tab_id: &str, generation: u64) -> String {
    let namespace = Uuid::new_v5(&Uuid::NAMESPACE_OID, NAMESPACE);
    let name = format!("{tab_id}/{generation}");
    Uuid::new_v5(&namespace, name.as_bytes())
        .hyphenated()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_tab_and_generation_give_the_same_identity() {
        assert_eq!(terminal_uuid("tb_a", 3), terminal_uuid("tb_a", 3));
    }

    #[test]
    fn a_new_generation_gives_a_new_identity() {
        assert_ne!(terminal_uuid("tb_a", 0), terminal_uuid("tb_a", 1));
    }

    #[test]
    fn another_tab_gives_another_identity() {
        assert_ne!(terminal_uuid("tb_a", 1), terminal_uuid("tb_b", 1));
    }

    #[test]
    fn a_generation_cannot_be_mistaken_for_part_of_the_tab() {
        assert_ne!(terminal_uuid("tb_1", 23), terminal_uuid("tb_12", 3));
    }

    #[test]
    fn the_identity_is_a_uuid() {
        let text = terminal_uuid("tb_a", 7);
        assert!(Uuid::parse_str(&text).is_ok(), "{text}");
    }

    #[test]
    fn the_identity_is_a_name_based_uuid() {
        let text = terminal_uuid("tb_a", 7);
        let parsed = Uuid::parse_str(&text).expect("a uuid");
        assert_eq!(parsed.get_version_num(), 5, "{text}");
    }
}
