mod board;
mod ui;

use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::{prelude::*, *};

gpui_kit::assets::icon_assets!(
    PrototypeAssets,
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

fn launch(cx: &mut App) {
    gpui_kit::init(cx);
    // Bundle fonts on both platforms so the same view has comparable metrics.
    if let Err(error) = cx.text_system().add_fonts(vec![
        std::borrow::Cow::Borrowed(include_bytes!("../assets/Inter-Regular.ttf")),
        std::borrow::Cow::Borrowed(include_bytes!("../assets/IBMPlexSans-Regular.ttf")),
        std::borrow::Cow::Borrowed(include_bytes!("../assets/JetBrainsMono-Regular.ttf")),
    ]) {
        eprintln!(
            "Could not load the prototype fonts: {error}. Restore the files in assets/ and rebuild."
        );
        return;
    }
    Theme::change(ThemeMode::Dark, None, cx);
    Theme::update(cx, |theme| {
        theme.font_family = "Inter".into();
        theme.mono_font_family = "JetBrains Mono".into();
        theme.font_size = px(13.);
        theme.mono_font_size = px(12.);
        theme.focus_ring = false;
    });
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
    gpui_kit::application()
        .with_assets(PrototypeAssets)
        .run(launch);
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
        .with_assets(PrototypeAssets);
    APPLICATION.with(|slot| *slot.borrow_mut() = Some(application.run_embedded(launch)));
}
