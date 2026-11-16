use ghostty_vt::Terminal;
use std::time::{Duration, Instant};
use terminal_codec::{LZ4, Limits, Registry, ZLIB};

#[test]
fn compare_codecs_on_real_dense_ten_thousand_line_native_checkpoint() {
    let mut terminal = Terminal::new(100, 30, 10_000).unwrap();
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
    let raw = terminal.checkpoint(16 << 20).unwrap();
    assert!(terminal.scrollback_rows() >= 9500);
    assert!(raw.len() > 4 << 20);
    let registry = Registry::default();
    for codec in [LZ4, ZLIB] {
        let mut encode_time = Duration::ZERO;
        let mut decode_time = Duration::ZERO;
        let mut bytes = 0;
        for _ in 0..3 {
            let start = Instant::now();
            let envelope = registry.encode(codec, &raw, Limits::default()).unwrap();
            encode_time += start.elapsed();
            bytes = envelope.data.len();
            let start = Instant::now();
            let restored = registry.decode(&envelope, Limits::default()).unwrap();
            decode_time += start.elapsed();
            assert_eq!(restored, raw);
            assert!(bytes <= 4 << 20);
        }
        println!(
            "dense_codec codec={codec} raw_bytes={} encoded_bytes={bytes} retained_rows={} samples=3 avg_encode_us={} avg_decode_us={}",
            raw.len(),
            terminal.scrollback_rows(),
            encode_time.as_micros() / 3,
            decode_time.as_micros() / 3
        );
    }
}
