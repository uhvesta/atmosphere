use ghostty_vt::{Grid, Terminal};
use std::time::Instant;

#[test]
fn core_round_trips_ten_thousand_dense_colored_unicode_lines() {
    let mut terminal = Terminal::new(100, 30, 10_000).unwrap();
    let feed = Instant::now();
    for line in 0..12_000u32 {
        let mut bytes = format!("{line:05} 界e\u{301}👩\u{200d}💻").into_bytes();
        for chunk in 0..8u32 {
            bytes.extend_from_slice(
                format!(
                    "\x1b[38;2;{};{};{}m",
                    (line * 17 + chunk * 31) % 256,
                    (line * 43 + chunk * 7) % 256,
                    (line * 73 + chunk * 19) % 256
                )
                .as_bytes(),
            );
            for column in 0..11u32 {
                bytes.push(b'a' + ((line * 11 + chunk * 17 + column * 7) % 26) as u8);
            }
        }
        bytes.extend_from_slice(b"Z\x1b[0m\r\n");
        terminal.feed(&bytes);
    }
    let feed_elapsed = feed.elapsed();
    let encode = Instant::now();
    let checkpoint = terminal.checkpoint(16 << 20).unwrap();
    let encode_elapsed = encode.elapsed();
    let retained = terminal.scrollback_rows();
    assert!(
        retained >= 9500,
        "the configured 10k lookback must retain approximately 10k physical rows"
    );
    let start = Instant::now();
    let mut restored = Terminal::from_checkpoint(&checkpoint).unwrap();
    let restore_elapsed = start.elapsed();
    eprintln!(
        "dense-history: cols=100 delivered=12000 retained={retained} raw_bytes={} feed_ms={} encode_ms={} restore_ms={}",
        checkpoint.len(),
        feed_elapsed.as_millis(),
        encode_elapsed.as_millis(),
        restore_elapsed.as_millis()
    );
    assert_eq!(restored.total_rows(), terminal.total_rows());
    for y in [0, terminal.total_rows() / 2, terminal.total_rows() - 1] {
        assert_eq!(
            restored.screen_row(u32::try_from(y).unwrap()).unwrap(),
            terminal.screen_row(u32::try_from(y).unwrap()).unwrap()
        );
    }
    restored.feed(b"");
    assert_eq!(restored.scrollback_rows(), retained);
    restored.feed(b"x");
    assert_eq!(restored.scrollback_rows(), retained);
    restored.sync();
    assert_eq!(restored.scrollback_rows(), retained);
    assert!(restored.row(0).is_some());
}
