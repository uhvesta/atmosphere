use ghostty_vt::{Grid, Scroll, Terminal, ViewportPoint};
fn at(x: u16, y: u16) -> ViewportPoint {
    ViewportPoint { x, y }
}
#[test]
fn osc8_uses_hidden_uri_and_survives_scrolling_and_resize() {
    let mut t = Terminal::new(50, 4, 200).unwrap();
    t.feed(b"\x1b]8;;https://example.com/real\x1b\\friendly label\x1b]8;;\x1b\\ plain\r\n");
    t.sync();
    assert_eq!(
        t.hyperlink(at(2, 0)).as_deref(),
        Some("https://example.com/real")
    );
    assert_eq!(t.hyperlink(at(17, 0)), None);
    t.feed(b"1\r\n2\r\n3\r\n4\r\n");
    t.scroll_viewport(Scroll::Top).unwrap();
    t.sync();
    assert_eq!(
        t.hyperlink(at(2, 0)).as_deref(),
        Some("https://example.com/real")
    );
    t.resize(60, 4).unwrap();
    t.sync();
    assert_eq!(
        t.hyperlink(at(2, 0)).as_deref(),
        Some("https://example.com/real")
    );
}
#[test]
fn plain_urls_support_wrapping_unicode_and_balanced_punctuation() {
    let mut t = Terminal::new(22, 6, 100).unwrap();
    t.feed("日 (https://example.com/a_(b)).\r\nno-link\r\n".as_bytes());
    t.sync();
    assert_eq!(
        t.hyperlink(at(6, 0)).as_deref(),
        Some("https://example.com/a_(b)")
    );
    assert_eq!(
        t.hyperlink(at(3, 1)).as_deref(),
        Some("https://example.com/a_(b)")
    );
    assert_eq!(t.hyperlink(at(0, 0)), None);
    assert_eq!(t.hyperlink(at(0, 2)), None);
}
#[test]
fn deleted_links_are_not_retained_at_the_same_cells() {
    let mut t = Terminal::new(50, 4, 100).unwrap();
    t.feed(b"\x1b]8;;https://example.com\x07link\x1b]8;;\x07");
    t.sync();
    assert!(t.hyperlink(at(0, 0)).is_some());
    t.feed(b"\r\x1b[2Kgone");
    t.sync();
    assert_eq!(t.hyperlink(at(0, 0)), None);
}
