//! LOOK2's, DESK's and REVIEW's exit check: each native capture of frames
//! 18–44 and 55 matches its design golden within tolerance, on the page
//! (right of the sidebar). The sidebar (left of 237 pt) is checked the same
//! way for the frames that show it, in `SIDEBARS`.
//!
//! The captures are what `native_test` records from its flows and the
//! goldens are what `design_test` draws (D76), so this test reads both
//! folders and draws nothing. The goldens of 18–35 don't draw the Desk row
//! the app has, so only the sidebars of frames 01–09, 12, 15–17 are checked.
//!
//! Pixels are compared in 2 × 2 pt blocks (the captures are 2x), so a glyph
//! anti-aliased half a pixel apart doesn't count, while a moved row, a wrong
//! colour or a missing button does. Places the decisions accept as different
//! (D147, D158, D219) are masked per frame; each mask says which ruling
//! allows it.
//! A frame fails when more than its budget of the page's blocks differ.

use std::ops::Range;
use std::path::{Path, PathBuf};

use image::RgbaImage;

/// The window the captures and goldens are drawn in, in points.
const WIDTH: u32 = 1280;
const HEIGHT: u32 = 760;
/// Device pixels per point in both folders.
const SCALE: u32 = 2;
/// The page starts right of the sidebar (`design/sidebar.rs` WIDTH, 236 pt)
/// and its 1 pt border.
const PAGE_LEFT: u32 = 237;
/// A block is this many points square.
const BLOCK: u32 = 2;
/// A block differs when one of its averaged channels is off by more than
/// this (of 255).
const CHANNEL: u32 = 48;
/// The share of the page's blocks that may differ when a frame has no budget
/// of its own.
const BUDGET: f64 = 0.005;

/// A rectangle in points: left, top, width, height.
#[derive(Clone, Copy, Debug)]
struct Rect(u32, u32, u32, u32);

impl Rect {
    fn holds(&self, x: u32, y: u32) -> bool {
        x >= self.0 && x < self.0 + self.2 && y >= self.1 && y < self.1 + self.3
    }
}

/// One frame: its file name in both folders, the places the rulings accept
/// as different, and its budget.
struct Frame {
    name: &'static str,
    masks: &'static [(Rect, &'static str)],
    budget: f64,
}

const fn frame(name: &'static str) -> Frame {
    Frame {
        name,
        masks: &[],
        budget: BUDGET,
    }
}

/// The Inbox card below its header row, down to the golden's bottom edge.
/// Its frame, top edge and corners are still compared.
const CARD_INSIDE: Rect = Rect(590, 80, 554, 430);
const CARD_INSIDE_WHY: &str =
    "D147 L2: the card's inside follows the request kind's preview, not the frame";

/// Frame 30 checks the palette; the dimmed session behind it is frame 01's,
/// which `native_test` checks, and its review draft comes from another flow.
const BEHIND_PALETTE_WHY: &str = "the session behind the palette is frame 01's";
/// The dashboard's figures are computed from the items (D147.9).
const FIGURES_WHY: &str = "D147.9: the dashboard's figures come from the items";
/// Controls with no action yet are drawn disabled; the frames draw them live.
const ADD_REPO_WHY: &str = "D147 L4: Add a GitHub repo… is disabled until it acts";
const HEADER_ACTIONS_WHY: &str = "D147 L4: the channel's Pin and … are disabled until they act";
/// A Desk session's terminal body, between the tabs row and the window's
/// bottom, left of the side panel's border.
const TERMINAL_BODY: Rect = Rect(237, 118, 638, 642);
const TERMINAL_GRID_WHY: &str = "D219.1: the grid's cell is a whole 7 px, the frame's Menlo 12 \
     advances 7.2; rows and the agent's input box sit off the frame's";

/// A document tab's Preview / Source / Side by side switch in its group row.
const MODE_SWITCH: Rect = Rect(640, 86, 232, 30);
const MODE_SWITCH_WHY: &str = "D166: the app shows the document's mode switch, frame 39 doesn't";

/// The agent's terminal body in the left of two equal tab groups (no panel).
const HALF_TERMINAL_BODY: Rect = Rect(237, 118, 489, 642);
/// The agent's terminal body in the 330 pt group beside a Side-by-side
/// document.
const NARROW_TERMINAL_BODY: Rect = Rect(237, 118, 330, 642);

const FRAMES: &[Frame] = &[
    frame("18-channel-sessions"),
    frame("19-channel-activity"),
    Frame {
        name: "20-channel-repos-machines",
        masks: &[
            (Rect(257, 243, 138, 24), ADD_REPO_WHY),
            (Rect(1116, 17, 40, 18), HEADER_ACTIONS_WHY),
        ],
        budget: BUDGET,
    },
    frame("21-board-kanban"),
    frame("22-board-item-open"),
    frame("23-board-table"),
    Frame {
        name: "24-board-dashboard",
        masks: &[
            (Rect(260, 106, 50, 28), FIGURES_WHY),
            (Rect(770, 106, 96, 28), FIGURES_WHY),
            (Rect(237, 185, 1043, 335), "D147.9: the frame draws a busier board (bars, a pr-reviews row), and the cards under it move with it"),
        ],
        budget: BUDGET,
    },
    frame("25-home"),
    Frame {
        name: "26-inbox",
        masks: &[
            (Rect(237, 466, 331, 146), "D147.8: the pr-reviews group the frame leaves out"),
            (CARD_INSIDE, CARD_INSIDE_WHY),
        ],
        budget: BUDGET,
    },
    Frame {
        name: "27-inbox-dark",
        masks: &[
            (
                Rect(237, 584, 331, 136),
                "D147.8: the pr-reviews group, and \"Allowed without asking\" is ASK's log",
            ),
            (CARD_INSIDE, CARD_INSIDE_WHY),
        ],
        budget: BUDGET,
    },
    frame("28-hosts"),
    frame("29-settings"),
    Frame {
        name: "30-palette",
        masks: &[
            (Rect(237, 0, 101, 760), BEHIND_PALETTE_WHY),
            (Rect(942, 0, 338, 760), BEHIND_PALETTE_WHY),
            (Rect(338, 0, 604, 108), BEHIND_PALETTE_WHY),
            (Rect(338, 546, 604, 214), BEHIND_PALETTE_WHY),
            (
                Rect(338, 252, 604, 294),
                "D147.10, D158.6: the palette also finds Review the schema and two more ledger \
                 files, so everything under Try BRIN moves down",
            ),
            (Rect(420, 124, 10, 22), "the caret blinks; the frame draws it on"),
        ],
        budget: BUDGET,
    },
    Frame {
        name: "31-new-session",
        masks: &[
            (Rect(477, 326, 424, 78), "D147 L13: the First message box takes its own width"),
        ],
        budget: BUDGET,
    },
    Frame {
        name: "32-toast",
        masks: &[
            (Rect(240, 8, 170, 36), "the archive that raises the toast leaves Home a Back button (Provisional, LOOK2)"),
        ],
        budget: BUDGET,
    },
    // Frame 33 is retired (D128.4): an empty workspace opens Home (34).
    frame("34-empty-home"),
    Frame {
        name: "35-empty-channel",
        masks: &[
            (Rect(925, 210, 141, 28), ADD_REPO_WHY),
            (Rect(1140, 22, 16, 8), HEADER_ACTIONS_WHY),
        ],
        budget: BUDGET,
    },
    // The Desk (DESK). The session frames show a terminal beside a panel.
    frame("36-desk"),
    Frame {
        name: "37-desk-board",
        masks: &[(TERMINAL_BODY, TERMINAL_GRID_WHY)],
        budget: BUDGET,
    },
    Frame {
        name: "38-desk-move",
        masks: &[(TERMINAL_BODY, TERMINAL_GRID_WHY)],
        budget: BUDGET,
    },
    // A document beside the agent (REVIEW, D166, flow 26.20).
    Frame {
        name: "39-desk-document",
        masks: &[(MODE_SWITCH, MODE_SWITCH_WHY)],
        budget: BUDGET,
    },
    frame("40-new-desk-session"),
    Frame {
        name: "41-review-beside-terminal",
        masks: &[(HALF_TERMINAL_BODY, TERMINAL_GRID_WHY)],
        budget: BUDGET,
    },
    frame("42-review-markdown-source"),
    Frame {
        name: "43-review-terminal-source-page",
        masks: &[(NARROW_TERMINAL_BODY, TERMINAL_GRID_WHY)],
        budget: BUDGET,
    },
    frame("44-new-channel"),
    frame("55-desk-clean-up"),
];

#[derive(Debug, thiserror::Error)]
enum LookError {
    #[error(
        "couldn't read {path}: {source}. If it is missing, record it: \
         `bazel run //apps/humata2/desktop:native_test_record` for a capture, \
         `:design_test_record` for a golden (goldens change only with the owner's OK)."
    )]
    Read {
        path: PathBuf,
        #[source]
        source: image::ImageError,
    },
    #[error(
        "{name}: the capture is {capture:?} and the golden {golden:?} pixels; both must be \
         {WIDTH}×{HEIGHT} pt at {SCALE}x. Check the window size in native.rs and design.rs."
    )]
    Size {
        name: &'static str,
        capture: (u32, u32),
        golden: (u32, u32),
    },
}

/// The snapshots folder: in the runfiles under `bazel test`, else beside
/// this file in the source tree.
fn snapshots() -> PathBuf {
    let relative = Path::new("apps/humata2/desktop/tests/__snapshots__");
    match std::env::var_os("TEST_SRCDIR") {
        Some(runfiles) => {
            let workspace = std::env::var_os("TEST_WORKSPACE").unwrap_or_else(|| "_main".into());
            PathBuf::from(runfiles).join(workspace).join(relative)
        }
        None => relative.to_path_buf(),
    }
}

fn read(path: PathBuf) -> Result<RgbaImage, LookError> {
    image::open(&path)
        .map(|image| image.to_rgba8())
        .map_err(|source| LookError::Read { path, source })
}

/// Block (`bx`, `by`), counted in blocks, averaged per channel.
fn block(image: &RgbaImage, bx: u32, by: u32) -> [u32; 3] {
    let side = BLOCK * SCALE;
    let (left, top) = (bx * side, by * side);
    let mut sum = [0u32; 3];
    for y in top..top + side {
        for x in left..left + side {
            let pixel = image.get_pixel(x, y).0;
            for (total, channel) in sum.iter_mut().zip(pixel) {
                *total += u32::from(channel);
            }
        }
    }
    sum.map(|total| total / (side * side))
}

/// The part of the window a comparison covers.
#[derive(Clone, Copy)]
enum Region {
    /// The page, right of the sidebar and its border.
    Page,
    /// The sidebar, left of the page's border at 237 pt. A block is compared
    /// only when it lies wholly left of that border, so the sidebar's last
    /// point column (236 pt) is not compared.
    Sidebar,
}

impl Region {
    /// The block columns of the region.
    fn columns(self) -> Range<u32> {
        match self {
            Self::Page => PAGE_LEFT.div_ceil(BLOCK)..WIDTH / BLOCK,
            Self::Sidebar => 0..PAGE_LEFT / BLOCK,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Page => "page",
            Self::Sidebar => "sidebar",
        }
    }
}

/// How the capture compares with the golden in `region`.
struct Comparison {
    differing: u64,
    total: u64,
    /// The golden faded, with differing blocks in red, for the test outputs.
    picture: RgbaImage,
}

fn compare(
    frame: &Frame,
    region: Region,
    capture: &RgbaImage,
    golden: &RgbaImage,
) -> Result<Comparison, LookError> {
    let expected = (WIDTH * SCALE, HEIGHT * SCALE);
    if capture.dimensions() != expected || golden.dimensions() != expected {
        return Err(LookError::Size {
            name: frame.name,
            capture: capture.dimensions(),
            golden: golden.dimensions(),
        });
    }
    let mut picture = golden.clone();
    for pixel in picture.pixels_mut() {
        let [r, g, b, _] = pixel.0;
        let luma = (u32::from(r) * 299 + u32::from(g) * 587 + u32::from(b) * 114) / 1000;
        let faded = (191 + luma / 4).min(255) as u8;
        pixel.0 = [faded, faded, faded, 255];
    }
    let (mut differing, mut total) = (0, 0);
    for by in 0..HEIGHT / BLOCK {
        for bx in region.columns() {
            let (x, y) = (bx * BLOCK, by * BLOCK);
            if frame.masks.iter().any(|(mask, _)| mask.holds(x, y)) {
                continue;
            }
            total += 1;
            let a = block(capture, bx, by);
            let b = block(golden, bx, by);
            if a.iter().zip(b).any(|(a, b)| a.abs_diff(b) > CHANNEL) {
                differing += 1;
                for py in y * SCALE..(y + BLOCK) * SCALE {
                    for px in x * SCALE..(x + BLOCK) * SCALE {
                        picture.put_pixel(px, py, image::Rgba([255, 0, 0, 255]));
                    }
                }
            }
        }
    }
    Ok(Comparison {
        differing,
        total,
        picture,
    })
}

/// Compares each of `frames` in `region` with its golden and answers one
/// message for each that can't be read or is over its budget.
fn misses_in(frames: &[Frame], region: Region) -> Vec<String> {
    let folder = snapshots();
    let outputs = std::env::var_os("TEST_UNDECLARED_OUTPUTS_DIR").map(PathBuf::from);
    let mut misses = Vec::new();
    for frame in frames {
        let file = format!("{}.png", frame.name);
        let checked = read(folder.join("native").join(&file)).and_then(|capture| {
            let golden = read(folder.join("design").join(&file))?;
            compare(frame, region, &capture, &golden)
        });
        let comparison = match checked {
            Ok(comparison) => comparison,
            Err(error) => {
                misses.push(error.to_string());
                continue;
            }
        };
        let share = comparison.differing as f64 / comparison.total.max(1) as f64;
        println!(
            "{}: {:.3}% of the {} differs (budget {:.3}%)",
            frame.name,
            share * 100.,
            region.name(),
            frame.budget * 100.
        );
        if share > frame.budget {
            let mut miss = format!(
                "{}: {:.3}% of the {} differs from the golden, over its {:.3}% budget.",
                frame.name,
                share * 100.,
                region.name(),
                frame.budget * 100.
            );
            if let Some(outputs) = &outputs {
                let path = outputs.join(format!("{}.look-diff.png", frame.name));
                if comparison.picture.save(&path).is_ok() {
                    miss.push_str(&format!(" Differing blocks are red in {}.", path.display()));
                }
            }
            misses.push(miss);
        }
    }
    misses
}

fn assert_matches(misses: &[String]) {
    assert!(
        misses.is_empty(),
        "Native captures that don't match their design goldens. Make the app match the \
         golden (the drawing code in desktop/tests/design/ has the numbers), then re-record the \
         capture with `bazel run //apps/humata2/desktop:native_test_record`. Mask a place only \
         when a decision accepts the difference, and name it beside the mask; don't raise a \
         budget to hide one.\n{}",
        misses.join("\n")
    );
}

#[test]
fn every_page_from_18_matches_its_golden() {
    assert_matches(&misses_in(FRAMES, Region::Page));
}

/// The sidebar of each frame that shows one: the session frames, the frame
/// with the board panel full screen, and the sidebar frames. Pinned rows
/// (frame 15) are the Pinned section of `ui/src/sidebar_pinned.rs`.
const SIDEBARS: &[Frame] = &[
    frame("01-session-working-light"),
    frame("02-session-working-dark"),
    frame("03-session-needs-you-light"),
    frame("04-session-needs-you-dark"),
    frame("05-session-exited"),
    frame("06-session-offline"),
    frame("07-session-board-panel"),
    frame("08-session-journal-panel"),
    frame("09-session-no-panel"),
    frame("12-panel-full-screen-board"),
    frame("15-sidebar-pinned"),
    frame("16-sidebar-fork-selected"),
    frame("17-sidebar-channels-folded"),
];

#[test]
fn every_sidebar_from_01_matches_its_golden() {
    assert_matches(&misses_in(SIDEBARS, Region::Sidebar));
}
