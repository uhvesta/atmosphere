use ghostty_vt::{
    DecMode, Error, Event, Grid, Key, KeyEvent, Mods, MouseAction, MouseButton, MouseEvent,
    OptionAsAlt, Scroll, Terminal, ViewportPoint,
};
const LIMIT: usize = 16 * 1024 * 1024;
fn same(a: &mut Terminal, b: &mut Terminal) {
    a.sync();
    b.sync();
    assert_eq!(a.size(), b.size());
    assert_eq!(a.total_rows(), b.total_rows());
    assert_eq!(a.cursor(), b.cursor());
    assert_eq!(a.cursor_style(), b.cursor_style());
    assert_eq!(a.colors(), b.colors());
    for y in 0..a.size().rows {
        assert_eq!(a.row(y), b.row(y), "viewport row {y}");
    }
    for y in 0..a.total_rows() {
        assert_eq!(
            a.screen_row(u32::try_from(y).unwrap()).unwrap(),
            b.screen_row(u32::try_from(y).unwrap()).unwrap(),
            "history row {y}"
        );
    }
}
#[test]
fn checkpoint_keeps_history_alt_screen_modes_links_and_live_continuation() {
    let mut source = Terminal::new(48, 7, 2000).unwrap();
    source.resize_with_cell_size(48, 7, 8, 16).unwrap();
    source.feed(b"\x1b]2;editor\x07\x1b]7;file:///tmp/project\x07");
    for n in 0..1000 {
        source.feed(format!("\x1b[38;2;11;99;200mhistory {n}\x1b[0m\r\n").as_bytes());
    }
    source.feed(b"\x1b]8;;https://example.com/primary\x1b\\primary link\x1b]8;;\x1b\\\r\n");
    source.feed(b"\x1b[?1049h\x1b[?1h\x1b[?1004h\x1b[?2004h\x1b[?1000h\x1b[?1006h\x1b[2J\x1b[H");
    source.feed("\x1b[1;3m日e\u{301} 👩\u{200d}💻\x1b[0m\r\n".as_bytes());
    source.feed(
        b"\x1b]8;;https://example.com/alternate\x1b\\alternate link\x1b]8;;\x1b\\\x1b[4;9H\x1b[6 q",
    );
    let bytes = source.checkpoint(LIMIT).unwrap();
    let mut restored = Terminal::from_checkpoint(&bytes).unwrap();
    assert!(
        restored.drain_events().is_empty(),
        "restoration must not repeat clipboard/title/PTY side effects"
    );
    same(&mut source, &mut restored);
    assert!(restored.is_alternate_screen());
    assert_eq!(restored.title().as_deref(), Some("editor"));
    assert_eq!(restored.pwd(), source.pwd());
    assert_eq!(
        restored.hyperlink(ViewportPoint { x: 2, y: 1 }).as_deref(),
        Some("https://example.com/alternate")
    );
    for mode in [
        DecMode::ApplicationCursorKeys,
        DecMode::FocusReporting,
        DecMode::BracketedPaste,
    ] {
        assert!(restored.is_mode_set(mode));
    }
    assert_eq!(
        source.encode_key(&KeyEvent::press(Key::ArrowUp)).unwrap(),
        restored.encode_key(&KeyEvent::press(Key::ArrowUp)).unwrap()
    );
    let mouse = MouseEvent {
        action: MouseAction::Press,
        button: Some(MouseButton::Left),
        mods: Mods::default(),
        x: 20.,
        y: 40.,
    };
    assert_eq!(
        source.encode_mouse(&mouse).unwrap(),
        restored.encode_mouse(&mouse).unwrap()
    );
    for t in [&mut source, &mut restored] {
        t.feed(b"\x1b[?1049l");
        t.scroll_viewport(Scroll::Top).unwrap();
    }
    same(&mut source, &mut restored);
    for t in [&mut source, &mut restored] {
        t.scroll_viewport(Scroll::Bottom).unwrap();
        t.feed(b"\r\ncontinued");
    }
    same(&mut source, &mut restored);
    restored.feed(b"\x07\x1b[6n");
    let events = restored.drain_events();
    assert!(events.iter().any(|e| matches!(e, Event::Bell)));
    assert!(events.iter().any(|e| matches!(e, Event::Reply(_))));
}
#[test]
fn unfinished_utf8_and_osc_sequences_resume_after_checkpoint() {
    for (prefix, suffix) in [
        (b"text \xf0\x9f".as_slice(), b"\x91\xa9 done".as_slice()),
        (
            b"\x1b]8;;https://example.com".as_slice(),
            b"\x1b\\label\x1b]8;;\x1b\\".as_slice(),
        ),
        (b"\x1b[38;2;255;".as_slice(), b"100;12mcolor".as_slice()),
    ] {
        let mut source = Terminal::new(40, 5, 100).unwrap();
        source.feed(prefix);
        let bytes = source.checkpoint(LIMIT).unwrap();
        let mut restored = Terminal::from_checkpoint(&bytes).unwrap();
        source.feed(suffix);
        restored.feed(suffix);
        same(&mut source, &mut restored);
    }
}
#[test]
fn malformed_or_oversized_checkpoints_leave_existing_terminal_untouched() {
    let mut original = Terminal::new(40, 5, 100).unwrap();
    original.feed(b"keep me");
    original.set_option_as_alt(OptionAsAlt::On);
    assert!(matches!(
        original.checkpoint(32),
        Err(Error::CheckpointTooLarge { .. })
    ));
    let valid = original.checkpoint(LIMIT).unwrap();
    for malformed in [
        valid[..valid.len() - 1].to_vec(),
        [valid.as_slice(), b"extra"].concat(),
        b"unrecognized".to_vec(),
    ] {
        assert!(original.restore_checkpoint(&malformed).is_err());
        let mut expected = Terminal::from_checkpoint(&valid).unwrap();
        same(&mut original, &mut expected);
    }
    original.restore_checkpoint(&valid).unwrap();
    assert_eq!(original.option_as_alt(), OptionAsAlt::On);
}
#[test]
fn reducing_history_limits_reclaims_existing_history() {
    let mut t = Terminal::new(80, 10, 20000).unwrap();
    for n in 0..12000 {
        t.feed(format!("history {n}\r\n").as_bytes());
    }
    let before = t.total_rows();
    assert!(before >= 12000);
    t.set_scrollback_limits(100, usize::MAX).unwrap();
    let after = t.total_rows();
    assert!(
        after < before,
        "lowered history limit should evict old pages immediately: {before} -> {after}"
    );
    assert!(
        after < 500,
        "page-granular retention should still be close to the requested budget: {after}"
    );
    let mut restored = Terminal::from_checkpoint(&t.checkpoint(LIMIT).unwrap()).unwrap();
    same(&mut t, &mut restored);
}

#[test]
fn from_checkpoint_bounded_caps_scrollback_rows() {
    let mut t = Terminal::new(80, 10, 10_000).unwrap();
    for n in 0..8_000 {
        t.feed(format!("bounded-line-{n}\r\n").as_bytes());
    }
    assert!(t.scrollback_rows() > 6_000);
    let bytes = t.checkpoint(LIMIT).unwrap();
    let restored = Terminal::from_checkpoint_bounded(
        &bytes,
        ghostty_vt::CLIENT_SCROLLBACK_MAX_LINES,
        ghostty_vt::CLIENT_SCROLLBACK_MAX_BYTES,
    )
    .unwrap();
    assert!(
        restored.scrollback_rows() < t.scrollback_rows(),
        "bounded restore should trim scrollback: {} -> {}",
        t.scrollback_rows(),
        restored.scrollback_rows()
    );
    assert!(
        restored.scrollback_rows() <= ghostty_vt::CLIENT_SCROLLBACK_MAX_LINES + 512,
        "page-granular retention should stay close to the 5k line cap: {}",
        restored.scrollback_rows()
    );
}

fn terminal_payload(checkpoint: &[u8]) -> usize {
    // The pinned format begins with our 20-byte envelope, Ghostty's 10-byte
    // envelope, then a 10-byte TERMINAL record header.
    assert_eq!(&checkpoint[20..28], b"GHOSTSNP");
    assert_eq!(
        u16::from_le_bytes(checkpoint[30..32].try_into().unwrap()),
        1
    );
    40
}

#[test]
fn corrupt_embedded_string_length_is_rejected_by_allocation_quota() {
    let original = Terminal::new(40, 5, 100).unwrap();
    let mut bytes = original.checkpoint(LIMIT).unwrap();
    let payload = terminal_payload(&bytes);
    let palette_mask = payload + 103 + 40usize.div_ceil(8) + 256 * 3;
    let overrides = bytes[palette_mask..palette_mask + 32]
        .iter()
        .map(|b| b.count_ones() as usize)
        .sum::<usize>();
    let pwd_len = palette_mask + 32 + overrides * 3;
    bytes[pwd_len..pwd_len + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(bytes.len() < 4096);
    for _ in 0..100 {
        assert!(
            matches!(Terminal::from_checkpoint(&bytes), Err(Error::CheckpointMemoryLimit { requested, .. }) if requested == u32::MAX as usize)
        );
    }
}

#[test]
fn grid_limits_apply_before_creating_resizing_or_decoding_native_state() {
    assert!(matches!(
        Terminal::new(4097, 2, 10),
        Err(Error::GridLimit { .. })
    ));
    assert!(matches!(
        Terminal::new(2048, 1024, 10),
        Err(Error::GridLimit { .. })
    ));
    let mut terminal = Terminal::new(40, 5, 100).unwrap();
    assert!(matches!(
        terminal.resize(65535, 65535),
        Err(Error::GridLimit { .. })
    ));
    assert_eq!(terminal.size().cols, 40);
    let valid = terminal.checkpoint(LIMIT).unwrap();
    let mut outer = valid.clone();
    outer[8..12].copy_from_slice(&[255, 255, 255, 255]);
    assert!(matches!(
        Terminal::from_checkpoint(&outer),
        Err(Error::GridLimit { .. })
    ));
    let mut embedded = valid;
    let payload = terminal_payload(&embedded);
    embedded[payload..payload + 4].copy_from_slice(&[255, 255, 255, 255]);
    assert!(matches!(
        Terminal::from_checkpoint(&embedded),
        Err(Error::GridLimit { .. })
    ));
}

#[test]
fn oversized_page_capacity_is_rejected_before_native_mmap() {
    let terminal = Terminal::new(40, 5, 100).unwrap();
    let mut bytes = terminal.checkpoint(LIMIT).unwrap();
    let mut at = 30;
    loop {
        let tag = u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap());
        let len = u32::from_le_bytes(bytes[at + 2..at + 6].try_into().unwrap()) as usize;
        if tag == 3 {
            bytes[at + 10 + 12..at + 10 + 16].copy_from_slice(&u32::MAX.to_le_bytes());
            break;
        }
        at += 10 + len;
    }
    assert!(matches!(
        Terminal::from_checkpoint(&bytes),
        Err(Error::CheckpointPageLimit { .. })
    ));
}

#[test]
fn bounded_restoration_keeps_ten_thousand_history_lines_and_accepts_live_output() {
    let mut source = Terminal::new(80, 24, 20000).unwrap();
    for n in 0..10000 {
        source.feed(format!("history {n}\r\n").as_bytes());
    }
    let mut restored = Terminal::from_checkpoint(&source.checkpoint(LIMIT).unwrap()).unwrap();
    assert_eq!(restored.total_rows(), source.total_rows());
    restored.feed(b"continued after bounded restore");
    let mut again = Terminal::from_checkpoint(&restored.checkpoint(LIMIT).unwrap()).unwrap();
    same(&mut restored, &mut again);
}
