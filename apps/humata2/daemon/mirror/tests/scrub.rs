//! Host variable values become their names before anything is mirrored
//! (LD-21).

use humata2_mirror::Scrubber;

fn scrubber(vars: &[(&str, &str)]) -> Scrubber {
    Scrubber::new(
        vars.iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned())),
    )
}

#[test]
fn every_occurrence_of_a_value_becomes_its_name() {
    let scrub = scrubber(&[("API_TOKEN", "tok-12345"), ("DB_PASSWORD", "hunter2")]);

    let out = scrub.scrub("export T=tok-12345; psql -W hunter2 # tok-12345 again");

    assert_eq!(
        out,
        "export T=‹API_TOKEN›; psql -W ‹DB_PASSWORD› # ‹API_TOKEN› again"
    );
}

#[test]
fn a_value_inside_a_longer_one_is_replaced_whole() {
    let text = "short abc, long abcdef, short again abc";
    let expected = "short ‹SHORT›, long ‹LONG›, short again ‹SHORT›";

    let short_first = scrubber(&[("SHORT", "abc"), ("LONG", "abcdef")]);
    let long_first = scrubber(&[("LONG", "abcdef"), ("SHORT", "abc")]);

    assert_eq!(short_first.scrub(text), expected);
    assert_eq!(long_first.scrub(text), expected);
}

#[test]
fn a_value_is_replaced_wherever_it_sits_in_a_word() {
    let scrub = scrubber(&[("KEY", "s3cr3t")]);

    assert_eq!(
        scrub.scrub("url=https://u:s3cr3t@host/x?k=s3cr3ts3cr3t"),
        "url=https://u:‹KEY›@host/x?k=‹KEY›‹KEY›"
    );
}

#[test]
fn only_the_exact_value_is_replaced() {
    let scrub = scrubber(&[("KEY", "s3cr3t")]);

    assert_eq!(scrub.scrub("S3CR3T s3cr3 s3cr-3t"), "S3CR3T s3cr3 s3cr-3t");
}

#[test]
fn a_variable_with_an_empty_value_changes_nothing() {
    let scrub = scrubber(&[("EMPTY", ""), ("KEY", "s3cr3t")]);

    assert_eq!(scrub.scrub("plain text"), "plain text");
    assert_eq!(scrub.scrub("has s3cr3t"), "has ‹KEY›");
    assert_eq!(scrub.scrub(""), "");
}

#[test]
fn text_is_unchanged_without_variables() {
    let scrub = scrubber(&[]);

    assert_eq!(
        scrub.scrub("nothing ‹to› hide: tok-12345"),
        "nothing ‹to› hide: tok-12345"
    );
}

#[test]
fn values_and_text_outside_ascii_are_handled() {
    let scrub = scrubber(&[("PASSPHRASE", "pässwörd-گذرواژه")]);

    assert_eq!(
        scrub.scrub("ورود با pässwörd-گذرواژه ✓"),
        "ورود با ‹PASSPHRASE› ✓"
    );
}

#[test]
fn a_value_with_several_lines_is_replaced() {
    let key = "-----BEGIN KEY-----\nabc\ndef\n-----END KEY-----";
    let scrub = scrubber(&[("SIGNING_KEY", key)]);

    assert_eq!(
        scrub.scrub(&format!("cat key.pem\n{key}\n$")),
        "cat key.pem\n‹SIGNING_KEY›\n$"
    );
}
