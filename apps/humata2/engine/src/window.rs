//! The window switches and counters only one engine has, behind one API, so
//! tests and benchmarks never name an engine. On ordinary GPUI the switches
//! report [`Unsupported`] and the counters are `None`.

use std::time::Duration;

use gpui_kit::Window;

/// A switch the running engine doesn't have.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error(
    "{switch} needs GPUI Fast, but this build draws with {engine}. Build with the `fast` feature to test it, or leave the switch at its ordinary-GPUI setting"
)]
pub struct Unsupported {
    /// The switch that was asked for.
    pub switch: &'static str,
    /// The engine compiled in.
    pub engine: &'static str,
}

/// What a window's frames did since its counters were last reset. GPUI Fast
/// counts these; ordinary GPUI doesn't.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameCounts {
    /// Frames drawn.
    pub frames: u64,
    /// Views built (rendered, laid out, prepainted and painted).
    pub views_built: u64,
    /// Views drawn again from the last frame instead.
    pub views_reused: u64,
    /// Layout nodes created and kept.
    pub nodes_created: u64,
    /// Layout nodes kept from the last frame.
    pub nodes_reused: u64,
    /// Scroll-layer frames composited from a layer's texture.
    pub layer_frames_composited: u64,
    /// Scroll-layer frames that repainted a layer.
    pub layer_frames_repainted: u64,
    /// Lines of text shaped.
    pub lines_shaped: u64,
    /// Time spent building views.
    pub build: Duration,
    /// Time spent prepainting.
    pub prepaint: Duration,
    /// Time spent painting.
    pub paint: Duration,
}

/// The window's frame counters, or `None` on an engine without them.
#[must_use]
pub fn frame_counts(window: &Window) -> Option<FrameCounts> {
    #[cfg(feature = "fast")]
    {
        let stats = window.layout_stats();
        Some(FrameCounts {
            frames: stats.frames,
            views_built: stats.views_built,
            views_reused: stats.views_reused,
            nodes_created: stats.nodes_created,
            nodes_reused: stats.nodes_reused,
            layer_frames_composited: stats.layer_frames_composited,
            layer_frames_repainted: stats.layer_frames_repainted,
            lines_shaped: stats.lines_shaped,
            build: stats.build_time,
            prepaint: stats.prepaint_time,
            paint: stats.paint_time,
        })
    }
    #[cfg(not(feature = "fast"))]
    {
        let _ = window;
        None
    }
}

/// Zeroes the window's frame counters. Does nothing on an engine without
/// them.
pub fn reset_frame_counts(window: &mut Window) {
    #[cfg(feature = "fast")]
    window.reset_layout_stats();
    #[cfg(not(feature = "fast"))]
    let _ = window;
}

/// Whether the window draws unchanged views again from the last frame.
/// Always `false` on ordinary GPUI.
#[must_use]
pub fn view_retention(window: &Window) -> bool {
    #[cfg(feature = "fast")]
    {
        window.view_retention()
    }
    #[cfg(not(feature = "fast"))]
    {
        let _ = window;
        false
    }
}

/// Turns view retention on or off. GPUI Fast refreshes the whole window when
/// it changes, so draw a frame before measuring anything after it.
///
/// # Errors
///
/// [`Unsupported`] when asking ordinary GPUI to retain views; turning
/// retention off there succeeds, since it never retains.
pub fn set_view_retention(window: &mut Window, enabled: bool) -> Result<(), Unsupported> {
    #[cfg(feature = "fast")]
    {
        window.set_view_retention(enabled);
        Ok(())
    }
    #[cfg(not(feature = "fast"))]
    {
        let _ = window;
        unsupported("view retention", enabled)
    }
}

/// Turns scroll layers on or off, dropping every layer. GPUI Fast refreshes
/// the whole window when it changes, so draw a frame before measuring
/// anything after it.
///
/// # Errors
///
/// [`Unsupported`] when asking ordinary GPUI for scroll layers; turning them
/// off there succeeds, since it has none.
pub fn set_scroll_layers(window: &mut Window, enabled: bool) -> Result<(), Unsupported> {
    #[cfg(feature = "fast")]
    {
        window.set_scroll_layers(enabled);
        Ok(())
    }
    #[cfg(not(feature = "fast"))]
    {
        let _ = window;
        unsupported("scroll layers", enabled)
    }
}

/// The glyphs, icons, images and underlines of the last drawn frame, one
/// line each with bounds, clip and colour, or `None` on an engine that
/// doesn't list them. Complete under view retention: a retained view's
/// sprites are copied into the frame.
#[must_use]
pub fn painted_sprites(window: &Window) -> Option<Vec<String>> {
    #[cfg(feature = "fast")]
    {
        Some(window.painted_sprites())
    }
    #[cfg(not(feature = "fast"))]
    {
        let _ = window;
        None
    }
}

#[cfg(not(feature = "fast"))]
fn unsupported(switch: &'static str, enabled: bool) -> Result<(), Unsupported> {
    if enabled {
        Err(Unsupported {
            switch,
            engine: crate::Engine::CURRENT.name(),
        })
    } else {
        Ok(())
    }
}
