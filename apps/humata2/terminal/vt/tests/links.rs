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

#[test]
fn osc8_file_uri_is_returned() {
    let mut t = Terminal::new(60, 4, 100).unwrap();
    t.feed(b"\x1b]8;;file:///session/SPEC.md:10\x1b\\SPEC.md:10\x1b]8;;\x1b\\ ok\r\n");
    t.sync();
    assert_eq!(
        t.hyperlink(at(2, 0)).as_deref(),
        Some("file:///session/SPEC.md:10")
    );
    assert_eq!(t.hyperlink(at(12, 0)), None);
}

#[test]
fn plain_file_paths_support_line_suffixes_and_wrapping() {
    let mut t = Terminal::new(24, 8, 100).unwrap();
    t.feed(
        b"/tmp/foo.rs:10\r\nfeatures/app/src/lib.rs:1856-1866\r\nsrc/main.rs:42:5\r\nfile:///tmp/demo.rs:42\r\n",
    );
    t.sync();
    assert_eq!(t.hyperlink(at(2, 0)).as_deref(), Some("/tmp/foo.rs:10"));
    assert_eq!(
        t.hyperlink(at(5, 1)).as_deref(),
        Some("features/app/src/lib.rs:1856-1866")
    );
    // Wrapped continuation of features/app/src/lib.rs:1856-1866 on row 2.
    assert_eq!(
        t.hyperlink(at(3, 2)).as_deref(),
        Some("features/app/src/lib.rs:1856-1866")
    );
    assert_eq!(t.hyperlink(at(4, 3)).as_deref(), Some("src/main.rs:42:5"));
    assert_eq!(
        t.hyperlink(at(4, 4)).as_deref(),
        Some("file:///tmp/demo.rs:42")
    );
}

#[test]
fn bare_filenames_are_recognized_and_non_links_are_ignored() {
    let mut t = Terminal::new(60, 6, 100).unwrap();
    t.feed(b"See SPEC.md, FILES_TAB_REFRESH_SPEC.md, and Cargo.toml:12-20.\r\n");
    t.feed(b"prose v1.2 127.0.0.1 12:30 ftp://example.com/a\r\n");
    t.sync();
    assert_eq!(t.hyperlink(at(5, 0)).as_deref(), Some("SPEC.md"));
    assert_eq!(
        t.hyperlink(at(16, 0)).as_deref(),
        Some("FILES_TAB_REFRESH_SPEC.md")
    );
    assert_eq!(t.hyperlink(at(46, 0)).as_deref(), Some("Cargo.toml:12-20"));
    assert_eq!(t.hyperlink(at(0, 1)), None); // prose
    assert_eq!(t.hyperlink(at(7, 1)), None); // v1.2
    assert_eq!(t.hyperlink(at(13, 1)), None); // 127.0.0.1
    assert_eq!(t.hyperlink(at(22, 1)), None); // 12:30
    assert_eq!(t.hyperlink(at(30, 1)), None); // ftp://example.com/a
}
