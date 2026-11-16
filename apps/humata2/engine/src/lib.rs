//! The one place Humata 2's GPUI code learns which engine it runs on.
//!
//! gpui-kit draws with ordinary GPUI or, with its `gpui-fast` feature, with
//! GPUI Fast. Both have the same API, but each has its own text system and
//! renderer crates, and only Fast retains views between frames. Everything
//! that would otherwise name an engine directly goes through here: starting
//! the app with its assets and fonts ([`Resources`], [`init`],
//! [`application`]), making a headless context for tests and benchmarks
//! (`headless`, with `test-support`), the engine-only window switches and
//! counters (`window`), and the engine's identity for artifacts ([`Backend`]).
//!
//! Reducers, the daemon and tca-rust never depend on this crate: engine
//! choice stays in the GUI.

use std::borrow::Cow;
use std::fmt;
use std::sync::Arc;

use gpui_kit::{App, AssetSource, SharedString};

#[cfg(feature = "test-support")]
pub mod frame;
#[cfg(feature = "test-support")]
pub mod parity;
#[cfg(feature = "test-support")]
pub mod window;

// gpui-kit is one crate in a build, so every crate sees the same engine. This
// crate's `fast` feature picks the text system that goes with it; the two
// must agree or a headless context would mix engines.
const _: () = assert!(
    gpui_kit::GPUI_FAST == cfg!(feature = "fast"),
    "gpui-kit and humata2-engine disagree about the GPUI engine: enable humata2-engine's `fast` feature exactly when gpui-kit's `gpui-fast` is on (the prototype's `fast` feature turns on both)"
);

/// The GPUI engine this build draws with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Engine {
    /// Ordinary GPUI (`gpui-pre`): every view is rebuilt each frame, except
    /// cached views that weren't notified.
    Gpui,
    /// GPUI Fast: views that nothing they depend on changed are drawn again
    /// from the last frame, and scroll containers can be composited layers.
    GpuiFast,
}

impl Engine {
    /// The engine compiled into this build.
    pub const CURRENT: Self = if gpui_kit::GPUI_FAST {
        Self::GpuiFast
    } else {
        Self::Gpui
    };

    /// A short name for file names and reports: `gpui` or `gpui-fast`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Gpui => "gpui",
            Self::GpuiFast => "gpui-fast",
        }
    }

    /// The engine crate's version. It is the version gpui-kit pins; update
    /// it together with the pins in this crate's Cargo.toml.
    #[must_use]
    pub const fn version(self) -> &'static str {
        match self {
            Self::Gpui => "gpui-pre 0.3.12",
            Self::GpuiFast => "gpui-fast 0.1.14",
        }
    }

    /// Whether the engine can draw a view again from the last frame.
    #[must_use]
    pub const fn retains_views(self) -> bool {
        matches!(self, Self::GpuiFast)
    }
}

/// gpui-kit's version, which picks both engines' versions.
pub const GPUI_KIT_VERSION: &str = "gpui-kit 0.9.1";

/// Which engine, build profile and target produced a test run, a snapshot
/// artifact or a benchmark: written into each so numbers and pictures are
/// never compared across engines by accident.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Backend {
    /// The engine compiled in.
    pub engine: Engine,
    /// `release` or `debug`, from `debug_assertions`.
    pub profile: &'static str,
    /// The CPU architecture, as [`std::env::consts::ARCH`].
    pub arch: &'static str,
    /// The operating system, as [`std::env::consts::OS`].
    pub os: &'static str,
}

impl Backend {
    /// The backend of this build.
    #[must_use]
    pub const fn current() -> Self {
        Self {
            engine: Engine::CURRENT,
            profile: if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            },
            arch: std::env::consts::ARCH,
            os: std::env::consts::OS,
        }
    }
}

impl fmt::Display for Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} ({}, {GPUI_KIT_VERSION}), {}, {}-{}",
            self.engine.name(),
            self.engine.version(),
            self.profile,
            self.arch,
            self.os
        )
    }
}

/// What starting GPUI failed at.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// The text system refused a bundled font.
    #[error(
        "could not load the app's {count} bundled fonts: {source}. Check that each bundled font file is an intact TrueType or OpenType file and rebuild"
    )]
    Fonts {
        /// How many fonts were offered.
        count: usize,
        /// What the text system reported.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

/// The assets and fonts an app brings to GPUI, given once and used the same
/// way by the desktop app, the browser build and headless tests, so all of
/// them measure and draw text alike.
#[derive(Clone)]
pub struct Resources {
    assets: Arc<dyn AssetSource>,
    fonts: Vec<Cow<'static, [u8]>>,
}

impl Resources {
    /// Resources that load assets (icons, images, fonts the SVG renderer
    /// asks for) from `assets`.
    pub fn new(assets: impl AssetSource) -> Self {
        Self {
            assets: Arc::new(assets),
            fonts: Vec::new(),
        }
    }

    /// Adds a font file, registered with the text system at [`init`].
    #[must_use]
    pub fn font(mut self, bytes: &'static [u8]) -> Self {
        self.fonts.push(Cow::Borrowed(bytes));
        self
    }

    /// The asset source, for an `Application` built by hand (the browser
    /// build makes its own platform).
    #[must_use]
    pub fn asset_source(&self) -> impl AssetSource + use<> {
        SharedAssets(self.assets.clone())
    }
}

struct SharedAssets(Arc<dyn AssetSource>);

impl AssetSource for SharedAssets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<Cow<'static, [u8]>>> {
        self.0.load(path)
    }

    fn list(&self, path: &str) -> gpui_kit::Result<Vec<SharedString>> {
        self.0.list(path)
    }
}

/// Registers the resources' fonts, then initializes gpui-kit in `cx`. Call it
/// first in the app's launch callback, and nowhere else: [`headless`] calls
/// it too.
///
/// # Errors
///
/// [`EngineError::Fonts`] when a bundled font can't be loaded.
pub fn init(cx: &mut App, resources: &Resources) -> Result<(), EngineError> {
    // Component initialization measures its default font immediately. Web
    // platforms have no system fonts, so bundled families must exist first.
    if !resources.fonts.is_empty() {
        cx.text_system()
            .add_fonts(resources.fonts.clone())
            .map_err(|source| EngineError::Fonts {
                count: resources.fonts.len(),
                source: source.into(),
            })?;
    }
    gpui_kit::init(cx);
    Ok(())
}

/// The desktop application, with the resources' assets.
#[cfg(not(target_family = "wasm"))]
#[must_use]
pub fn application(resources: &Resources) -> gpui_kit::Application {
    gpui_kit::application().with_assets(resources.asset_source())
}

/// A headless context that draws with the platform's renderer off screen
/// (Metal on macOS) and measures text with the platform's fonts, for tests
/// and benchmarks. No window is shown, nothing takes focus, and it works on
/// any thread. gpui-kit and the resources are initialized in it.
///
/// # Errors
///
/// [`EngineError::Fonts`] when a bundled font can't be loaded.
#[cfg(all(feature = "test-support", target_os = "macos"))]
pub fn headless(resources: &Resources) -> Result<gpui_kit::HeadlessAppContext, EngineError> {
    #[cfg(feature = "fast")]
    use fast_apple as apple;
    #[cfg(not(feature = "fast"))]
    use pre_apple as apple;

    let mut cx = gpui_kit::HeadlessAppContext::with_platform(
        Arc::new(apple::AppleTextSystem::new()),
        resources.assets.clone(),
        gpui_kit::platform::current_headless_renderer,
    );
    cx.update(|cx| init(cx, resources))?;
    Ok(cx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_names_the_engine_compiled_in() {
        let backend = Backend::current();
        assert_eq!(backend.engine == Engine::GpuiFast, gpui_kit::GPUI_FAST);
        assert_eq!(backend.engine.retains_views(), gpui_kit::GPUI_FAST);
        assert!(backend.to_string().starts_with(backend.engine.name()));
    }
}
