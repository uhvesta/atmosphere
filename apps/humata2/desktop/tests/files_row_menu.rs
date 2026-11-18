//! A file manager row's menu as a person uses it, headless on the fakes
//! (SPEC PL-10): Copy path puts the absolute path on the clipboard, Reveal
//! in Finder asks Finder (the fake, never the real one), and a remote row
//! offers Open terminal here instead. No window is shown.

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::{
    AnyWindowHandle, Bounds, HeadlessAppContext, Role, WindowBounds, WindowOptions, point, px, size,
};
use humata2_app::{AppAction, Route};
use humata2_clients::model::MachineId;
use humata2_clients::{Clients, Fakes, fixture};
use humata2_engine::Resources;
use humata2_files::{FilesAction, Side};
use humata2_flow::{Action, Driver as _, FrameMode, GpuiDriver, Target};
use humata2_ui::bridge::Ui;
use humata2_ui::record::Recorder;
use tca::TestClock;
use tokio::runtime::Runtime;

struct App {
    runtime: Runtime,
    cx: HeadlessAppContext,
    window: AnyWindowHandle,
    ui: Rc<Ui>,
    fakes: Fakes,
}

fn gpu() -> MachineId {
    MachineId::new("gpu-box")
}

impl App {
    /// The app on the standard fakes with `notes.txt` in This Mac's home and
    /// in gpu-box's.
    fn new() -> Self {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a current-thread runtime starts");
        let mut cx = humata2_engine::headless(&Resources::new(humata2_ui::Assets))
            .expect("the headless context starts");
        cx.update(humata2_ui::init);
        let (clients, fakes) = Clients::fake(fixture::standard(), Arc::new(TestClock::new()));
        for machine in ["this-mac", "gpu-box"] {
            fakes
                .file_manager
                .put_file(&humata2_clients::Folder::home(MachineId::new(machine)), "notes.txt", 10);
        }
        let ui = Ui::new(runtime.handle(), clients, Recorder::new());
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                point(px(0.), px(0.)),
                size(px(humata2_ui::WINDOW.0), px(humata2_ui::WINDOW.1)),
            ))),
            show: false,
            focus: false,
            ..WindowOptions::default()
        };
        let (window, _shell) = cx
            .update(|cx| humata2_ui::open(cx, &ui, options))
            .expect("the window opens");
        let mut app = Self {
            runtime,
            cx,
            window,
            ui,
            fakes,
        };
        app.settle();
        app
    }

    fn settle(&mut self) {
        for _ in 0..10 {
            self.runtime.block_on(async {
                for _ in 0..32 {
                    tokio::task::yield_now().await;
                }
            });
            self.cx.run_until_parked();
        }
        self.cx
            .update_window(self.window, |_, window, cx| {
                humata2_engine::frame::natural_frame(window, cx);
            })
            .expect("the window stays open");
    }

    fn click(&mut self, role: Role, label: &str) {
        GpuiDriver::with_frames(&mut self.cx, self.window, FrameMode::Natural)
            .perform(&Action::Click(Target::new(role, label)))
            .expect("the control is drawn and can be clicked");
        self.settle();
    }

    fn send(&mut self, action: AppAction) {
        self.cx.update(|cx| self.ui.send(action, cx));
        self.settle();
    }

    /// Opens the Files page and the menu of `notes.txt` in `side`.
    fn open_menu(&mut self, side: Side) {
        self.send(AppAction::Navigate(Route::Files));
        if side == Side::Right {
            self.send(AppAction::Files(FilesAction::PickMachine(Side::Right, gpu())));
        }
        self.send(AppAction::Files(FilesAction::OpenRowMenu(side, "notes.txt".to_owned())));
    }

    fn menu_open(&self) -> bool {
        self.ui.read(|state| state.files.row_menu.is_some())
    }
}

#[test]
fn copy_path_in_a_this_mac_rows_menu_copies_its_path() {
    let mut app = App::new();
    app.open_menu(Side::Left);
    assert!(app.menu_open());
    app.click(Role::ListBoxOption, "Copy path");
    assert_eq!(app.fakes.clipboard.copied().as_deref(), Some("~/notes.txt"));
    assert!(!app.menu_open(), "choosing closes the menu");
}

#[test]
fn reveal_in_finder_in_a_this_mac_rows_menu_asks_finder() {
    let mut app = App::new();
    app.open_menu(Side::Left);
    app.click(Role::ListBoxOption, "Reveal in Finder");
    assert_eq!(app.fakes.reveal.revealed(), ["~/notes.txt"]);
}

#[test]
fn a_remote_rows_menu_offers_a_terminal_instead_of_reveal() {
    let mut app = App::new();
    app.open_menu(Side::Right);
    app.click(Role::ListBoxOption, "Open terminal here");
    assert!(!app.menu_open(), "choosing closes the menu");
    assert!(app.fakes.reveal.revealed().is_empty());
}
