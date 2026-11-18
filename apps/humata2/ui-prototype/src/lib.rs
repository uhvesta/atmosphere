mod board;
mod flows;
mod journeys;
mod panes;
mod rich;
mod terminal;
mod ui;

use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::{prelude::*, *};

gpui_kit::assets::icon_assets!(
    PrototypeIcons,
    [
        Search,
        Plus,
        ChevronDown,
        ChevronRight,
        ChevronLeft,
        PanelLeft,
        PanelRight,
        Terminal,
        GitBranch,
        GitFork,
        Files,
        File,
        Folder,
        Settings,
        Inbox,
        LayoutDashboard,
        House,
        Server,
        Check,
        X,
        ArrowUp,
        ArrowRight,
        Ellipsis,
        Circle,
        Pin,
        Sun,
        Moon,
        ChevronsUpDown,
        Maximize,
        MessageSquare,
        Columns2,
        Square,
    ]
);

// GPUI’s SVG renderer asks the asset source for this bundled font as well.
// Sharing it keeps Mermaid labels available in the browser without system fonts.
struct PrototypeAssets;
impl AssetSource for PrototypeAssets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<std::borrow::Cow<'static, [u8]>>> {
        match path {
            "fonts/ibm-plex-sans/IBMPlexSans-Regular.ttf" => Ok(Some(std::borrow::Cow::Borrowed(
                include_bytes!("../assets/IBMPlexSans-Regular.ttf"),
            ))),
            "fonts/lilex/Lilex-Regular.ttf" => Ok(Some(std::borrow::Cow::Borrowed(
                include_bytes!("../assets/JetBrainsMono-Regular.ttf"),
            ))),
            _ => PrototypeIcons.load(path),
        }
    }
    fn list(&self, path: &str) -> gpui_kit::Result<Vec<SharedString>> {
        let mut paths = PrototypeIcons.list(path)?;
        for font in [
            "fonts/ibm-plex-sans/IBMPlexSans-Regular.ttf",
            "fonts/lilex/Lilex-Regular.ttf",
        ] {
            if font.starts_with(path) {
                paths.push(font.into());
            }
        }
        Ok(paths)
    }
}

#[cfg(target_family = "wasm")]
#[wasm_bindgen::prelude::wasm_bindgen(
    inline_js = "export function prototype_scene() { return new URL(location.href).searchParams.get('scene') || ''; }"
)]
extern "C" {
    fn prototype_scene() -> String;
}

#[cfg(target_family = "wasm")]
fn startup_scene() -> String {
    prototype_scene()
}

/// The prototype's assets and bundled fonts, the same for the desktop app,
/// the browser build and the headless tests, so text measures alike.
fn resources() -> humata2_engine::Resources {
    // Bundle fonts on both platforms so the same view has comparable metrics.
    humata2_engine::Resources::new(PrototypeAssets)
        .font(include_bytes!("../assets/Inter-Regular.ttf"))
        .font(include_bytes!("../assets/IBMPlexSans-Regular.ttf"))
        .font(include_bytes!("../assets/JetBrainsMono-Regular.ttf"))
}

/// The prototype's theme: dark, Inter and JetBrains Mono.
fn apply_theme(cx: &mut App) {
    Theme::change(ThemeMode::Dark, None, cx);
    Theme::update(cx, |theme| {
        theme.font_family = "Inter".into();
        theme.mono_font_family = "JetBrains Mono".into();
        theme.font_size = px(13.);
        theme.mono_font_size = px(12.);
        theme.focus_ring = false;
    });
}

fn launch(cx: &mut App) {
    if let Err(error) = humata2_engine::init(cx, &resources()) {
        eprintln!("Could not start the prototype: {error}. Restore the files in assets/ and rebuild.");
        return;
    }
    apply_theme(cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(1512.), px(960.)),
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some("Humata 2 · UI Prototype".into()),
            appears_transparent: false,
            ..Default::default()
        }),
        ..Default::default()
    };
    match gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| ui::Prototype::new(window, cx))
    }) {
        Ok(_) => cx.activate(true),
        Err(error) => eprintln!(
            "Could not open the prototype window: {error}. Restart it in a graphical desktop session, or reload the browser and check graphics support."
        ),
    }
}

/// Native entry point used by this isolated prototype's executable.
#[cfg(not(target_family = "wasm"))]
pub fn run_desktop() {
    humata2_engine::application(&resources()).run(launch);
}

#[cfg(target_family = "wasm")]
thread_local! {
    static APPLICATION: std::cell::RefCell<Option<ApplicationHandle>> = const { std::cell::RefCell::new(None) };
}

/// Browser entry point; keeps the asynchronously started GPUI application alive.
#[cfg(target_family = "wasm")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn run() {
    use gpui_kit::web::{CanvasFontFallback, WebBackendPreference, WebPlatform};
    console_error_panic_hook::set_once();
    gpui_kit::platform::web_init();
    let platform = std::rc::Rc::new(WebPlatform::new_with_backend_and_font_fallback(
        false,
        WebBackendPreference::Auto,
        CanvasFontFallback::EmojiAndCjk,
    ));
    let http = std::sync::Arc::new(platform.fetch_http_client());
    let application = Application::with_platform(platform)
        .with_http_client(http)
        .with_assets(resources().asset_source());
    APPLICATION.with(|slot| *slot.borrow_mut() = Some(application.run_embedded(launch)));
}
