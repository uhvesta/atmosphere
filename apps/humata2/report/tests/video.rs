//! A recording's video: frames of the window kept as changes, saved as an
//! animated PNG and a frame list.

use std::io::Cursor;
use std::path::Path;

use humata2_report::video::Video;

/// What one decoded frame holds.
struct Frame {
    width: u32,
    height: u32,
    x: u32,
    y: u32,
    delay_cs: u16,
    rgba: Vec<u8>,
}

struct Decoded {
    width: u32,
    height: u32,
    declared_frames: u32,
    frames: Vec<Frame>,
}

/// A `width x height` window of one colour, in device pixels.
fn solid(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
    let mut out = Vec::new();
    for _ in 0..width * height {
        out.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
    }
    out
}

/// Sets the device pixel `(x, y)` of `rgba`, `width` wide.
fn paint(rgba: &mut [u8], width: u32, x: u32, y: u32, rgb: [u8; 3]) {
    let at = ((y * width + x) * 4) as usize;
    rgba[at..at + 4].copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
}

fn decode(dir: &Path) -> Decoded {
    let bytes = std::fs::read(dir.join("video.png")).expect("video.png is written");
    let mut reader = png::Decoder::new(Cursor::new(bytes)).read_info().expect("a valid PNG");
    let info = reader.info();
    let (width, height) = (info.width, info.height);
    let declared_frames = info.animation_control().expect("an animated PNG").num_frames;
    let mut buf = vec![0; reader.output_buffer_size().expect("fits in memory")];
    let mut frames = Vec::new();
    for _ in 0..declared_frames {
        let out = reader.next_frame(&mut buf).expect("a frame decodes");
        let control = reader.info().frame_control.expect("each frame has a control chunk");
        frames.push(Frame {
            width: out.width,
            height: out.height,
            x: control.x_offset,
            y: control.y_offset,
            delay_cs: control.delay_num,
            rgba: buf[..out.buffer_size()].to_vec(),
        });
    }
    Decoded {
        width,
        height,
        declared_frames,
        frames,
    }
}

fn lines(dir: &Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(dir.join("video.jsonl"))
        .expect("video.jsonl is written")
        .lines()
        .map(|line| serde_json::from_str(line).expect("each line is JSON"))
        .collect()
}

#[test]
fn a_static_screen_is_one_frame_shown_for_as_long_as_it_stayed() {
    let mut video = Video::new(1);
    let screen = solid(8, 6, [10, 20, 30]);
    for at in [0, 200, 400, 600, 800] {
        video.push(at, 8, 6, &screen).expect("frames are kept");
    }
    assert_eq!(video.frames(), 1);

    let dir = tempfile::tempdir().expect("a folder");
    let files = video.write(dir.path()).expect("it writes").expect("there is a video");
    assert_eq!(files.frames, 1);
    let decoded = decode(dir.path());
    assert_eq!(decoded.declared_frames, 1);
    assert_eq!(decoded.frames[0].delay_cs, 80);
    let lines = lines(dir.path());
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["delay_ms"], 800);
}

#[test]
fn a_small_change_is_a_frame_cropped_to_what_changed() {
    let mut video = Video::new(1);
    let before = solid(20, 10, [0, 0, 0]);
    let mut after = before.clone();
    paint(&mut after, 20, 5, 3, [255, 0, 0]);
    paint(&mut after, 20, 7, 4, [255, 0, 0]);
    video.push(0, 20, 10, &before).expect("kept");
    video.push(200, 20, 10, &after).expect("kept");

    let dir = tempfile::tempdir().expect("a folder");
    video.write(dir.path()).expect("it writes");
    let decoded = decode(dir.path());
    assert_eq!(decoded.frames.len(), 2);
    let delta = &decoded.frames[1];
    assert_eq!((delta.x, delta.y, delta.width, delta.height), (5, 3, 3, 2));

    // Pixels inside the box that didn't change are transparent; the ones
    // that did are opaque.
    let alpha = |x: usize, y: usize| delta.rgba[(y * 3 + x) * 4 + 3];
    assert_eq!(alpha(0, 0), 255);
    assert_eq!(alpha(1, 0), 0);
    assert_eq!(alpha(2, 1), 255);

    let lines = lines(dir.path());
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[1]["at_ms"], 200);
    assert_eq!(
        [&lines[1]["x"], &lines[1]["y"], &lines[1]["width"], &lines[1]["height"]],
        [&5, &3, &3, &2]
    );
}

#[test]
fn frames_are_scaled_to_points_by_averaging() {
    // 4x2 device pixels at scale 2 is 2x1 points; the first point averages
    // two black and two white pixels, the second is all white.
    let mut rgba = solid(4, 2, [0, 0, 0]);
    paint(&mut rgba, 4, 0, 0, [255, 255, 255]);
    paint(&mut rgba, 4, 1, 1, [255, 255, 255]);
    for y in 0..2 {
        for x in 2..4 {
            paint(&mut rgba, 4, x, y, [255, 255, 255]);
        }
    }
    let mut video = Video::new(2);
    video.push(0, 4, 2, &rgba).expect("kept");

    let dir = tempfile::tempdir().expect("a folder");
    video.write(dir.path()).expect("it writes");
    let decoded = decode(dir.path());
    assert_eq!((decoded.width, decoded.height), (2, 1));
    assert_eq!(&decoded.frames[0].rgba[..4], &[128, 128, 128, 255]);
    assert_eq!(&decoded.frames[0].rgba[4..8], &[255, 255, 255, 255]);
}

#[test]
fn a_frame_of_another_size_is_left_out() {
    let mut video = Video::new(1);
    video.push(0, 8, 6, &solid(8, 6, [1, 1, 1])).expect("kept");
    video.push(200, 10, 6, &solid(10, 6, [2, 2, 2])).expect("left out, not an error");
    video.push(400, 8, 6, &[0; 5]).expect("left out, not an error");
    assert_eq!(video.frames(), 1);
    assert_eq!(video.skipped(), 2);
}

#[test]
fn the_saved_animation_has_the_window_size_and_every_frame() {
    let mut video = Video::new(2);
    let mut screen = solid(40, 20, [0, 0, 0]);
    for step in 0..4 {
        paint(&mut screen, 40, step * 2, 0, [255, 255, 255]);
        video.push(u64::from(step) * 200, 40, 20, &screen).expect("kept");
    }
    let dir = tempfile::tempdir().expect("a folder");
    let files = video.write(dir.path()).expect("it writes").expect("a video");
    assert_eq!(files.frames, 4);
    let decoded = decode(dir.path());
    assert_eq!((decoded.width, decoded.height), (20, 10));
    assert_eq!(decoded.declared_frames, 4);
    let first = &decoded.frames[0];
    assert_eq!((first.width, first.height), (20, 10));
    assert_eq!(lines(dir.path()).len(), 4);
}

#[test]
fn past_its_limit_the_oldest_frames_fold_into_the_first() {
    let mut video = Video::with_limits(1, 5, usize::MAX);
    let mut screen = solid(32, 4, [0, 0, 0]);
    for step in 0..20u32 {
        paint(&mut screen, 32, step, 0, [255, 255, 255]);
        video.push(u64::from(step) * 200, 32, 4, &screen).expect("kept");
        assert!(video.frames() <= 5);
    }
    assert_eq!(video.frames(), 5);

    let dir = tempfile::tempdir().expect("a folder");
    let files = video.write(dir.path()).expect("it writes").expect("a video");
    assert_eq!(files.frames, 5);
    // The first frame is the screen as it was at step 15, whole.
    assert_eq!(files.video_start_ms, 15 * 200);
    let decoded = decode(dir.path());
    assert_eq!(decoded.declared_frames, 5);
    let first = &decoded.frames[0];
    assert_eq!((first.width, first.height), (32, 4));
    let white = |x: usize| first.rgba[x * 4..x * 4 + 4] == [255, 255, 255, 255];
    assert!((0..=15).all(white));
    assert!(!white(16));
    let lines = lines(dir.path());
    assert_eq!(lines.len(), 5);
    assert_eq!(lines[0]["at_ms"], 15 * 200);
}

#[test]
fn past_its_byte_limit_the_oldest_changes_fold_too() {
    // A limit nothing fits under keeps only the whole first frame.
    let mut video = Video::with_limits(1, 900, 0);
    let mut screen = solid(16, 4, [0, 0, 0]);
    for step in 0..6u32 {
        paint(&mut screen, 16, step, 0, [255, 255, 255]);
        video.push(u64::from(step) * 200, 16, 4, &screen).expect("kept");
    }
    assert_eq!(video.frames(), 1);
}

#[test]
fn no_frames_write_nothing() {
    let video = Video::new(2);
    let dir = tempfile::tempdir().expect("a folder");
    assert!(video.write(dir.path()).expect("nothing to fail").is_none());
    assert_eq!(std::fs::read_dir(dir.path()).expect("readable").count(), 0);
}

#[test]
fn the_files_are_readable_only_by_this_user() {
    use std::os::unix::fs::PermissionsExt as _;
    let mut video = Video::new(1);
    video.push(0, 4, 4, &solid(4, 4, [9, 9, 9])).expect("kept");
    let dir = tempfile::tempdir().expect("a folder");
    video.write(dir.path()).expect("it writes");
    for name in ["video.png", "video.jsonl"] {
        let mode = std::fs::metadata(dir.path().join(name)).expect("exists").permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
