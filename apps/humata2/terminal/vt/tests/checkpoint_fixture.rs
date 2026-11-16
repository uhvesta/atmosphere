//! Native fixture for the manual native64 <-> wasm32 snapshot conformance test.
use ghostty_vt::{CHECKPOINT_FORMAT, Terminal};
fn main() {
    let mut args = std::env::args_os().skip(1);
    let path = args.next().expect("pass a destination checkpoint path");
    if path == "--verify" {
        let bytes = std::fs::read(args.next().expect("pass the Wasm checkpoint path"))
            .expect("read Wasm checkpoint");
        let terminal =
            Terminal::from_checkpoint(&bytes).expect("restore Wasm checkpoint on native64");
        assert_eq!(terminal.size().cols, 40);
        assert!(!terminal.is_alternate_screen());
        assert!(terminal.total_rows() >= 150);
        let text = (0..terminal.total_rows())
            .map(|y| {
                terminal
                    .screen_row(u32::try_from(y).unwrap())
                    .unwrap()
                    .cells
                    .into_iter()
                    .map(|cell| cell.text)
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("history 0"));
        assert!(text.contains("wasm resumed"));
        println!("wasm32-to-native64-checkpoint: restored");
        return;
    }
    let mut terminal = Terminal::new(40, 6, 1000).expect("create fixture terminal");
    for n in 0..150 {
        terminal.feed(format!("history {n}\r\n").as_bytes());
    }
    terminal.feed(b"\x1b[?1049h\x1b[?2004h\x1b[?1h\x1b[H\x1b[1;38;2;30;140;220malternate \x1b]8;;https://example.com/checkpoint\x1b\\link\x1b]8;;\x1b\\ \xf0\x9f");
    let checkpoint = terminal
        .checkpoint(8 * 1024 * 1024)
        .expect("encode fixture checkpoint");
    std::fs::write(path, &checkpoint).expect("write fixture checkpoint");
    println!("{CHECKPOINT_FORMAT}: {} bytes", checkpoint.len());
}
