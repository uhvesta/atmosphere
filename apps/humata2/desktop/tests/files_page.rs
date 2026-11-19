//! The Files page as a person uses it, headless on the fakes: open it, pick
//! a machine through the popup, copy to a machine or somewhere specific,
//! make a folder through the name prompt, delete to the trash, and meet an
//! offline machine. No window is shown.
//!
//! Written for when the page is wired: `Clients` has `file_manager`, `Fakes`
//! has the fake behind it, and the Files route sends `FilesAction::Opened`.

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::{
    AnyWindowHandle, Bounds, HeadlessAppContext, Role, WindowBounds, WindowOptions, point, px, size,
};
use humata2_app::{AppAction, Route};
use humata2_clients::model::MachineId;
use humata2_clients::{Clients, FakeFileManager, FileRoot, Folder, fixture};
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
    fake: FakeFileManager,
}

fn mac() -> MachineId {
    MachineId::new("this-mac")
}

fn gpu() -> MachineId {
    MachineId::new("gpu-box")
}

impl App {
    /// The app on the standard fakes with `notes.txt` in This Mac's home.
    fn new() -> Self {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a current-thread runtime starts");
        let mut cx = humata2_engine::headless(&Resources::new(humata2_ui::Assets))
            .expect("the headless context starts");
        cx.update(humata2_ui::init);
        let (clients, fakes) = humata2_clients::fake_clients(fixture::standard(), Arc::new(TestClock::new()));
        let fake = fakes.file_manager.clone();
        fake.put_file(&Folder::home(mac()), "notes.txt", 10);
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
            fake,
        };
        app.settle();
        app
    }

    /// Runs what the last step started, then draws a frame as the app would.
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

    fn files(&mut self, action: FilesAction) {
        self.send(AppAction::Files(action));
    }

    fn open_files(&mut self) {
        self.send(AppAction::Navigate(Route::Files));
    }

    fn machine(&self, side: Side) -> Option<MachineId> {
        self.ui.read(|state| state.files.pane(side).machine.clone())
    }

    fn names(&self, side: Side) -> Vec<String> {
        self.ui.read(|state| {
            state
                .files
                .pane(side)
                .entries
                .iter()
                .map(|entry| entry.name.clone())
                .collect()
        })
    }

    fn picker(&self) -> Option<Side> {
        self.ui.read(|state| state.files.picker)
    }

    fn pick_on_right(&mut self, machine_name: &str) {
        self.click(Role::Button, "Machine of the right pane: This Mac");
        assert_eq!(self.picker(), Some(Side::Right), "the popup opens its list");
        self.click(Role::ListBoxOption, machine_name);
    }
}

#[test]
fn opening_files_lists_this_mac_in_both_panes() {
    let mut app = App::new();
    app.open_files();
    assert_eq!(app.machine(Side::Left), Some(mac()));
    assert_eq!(app.machine(Side::Right), Some(mac()));
    assert_eq!(app.names(Side::Left), ["notes.txt"]);
    assert_eq!(app.names(Side::Right), ["notes.txt"]);
}

#[test]
fn the_right_pane_picks_another_machine_through_the_popup() {
    let mut app = App::new();
    app.open_files();
    app.pick_on_right("gpu-box");
    assert_eq!(app.machine(Side::Right), Some(gpu()));
    assert_eq!(app.picker(), None, "picking closes the list");
    assert!(app.names(Side::Right).is_empty());
}

#[test]
fn an_offline_machine_cannot_be_picked() {
    let mut app = App::new();
    app.open_files();
    app.pick_on_right("ci-runner");
    assert_eq!(app.machine(Side::Right), Some(mac()));
}

#[test]
fn a_click_elsewhere_closes_the_list() {
    let mut app = App::new();
    app.open_files();
    // Both panes start on This Mac's home; one notes.txt row is the left's.
    app.pick_on_right("gpu-box");
    app.click(Role::Button, "Machine of the right pane: gpu-box");
    assert_eq!(app.picker(), Some(Side::Right));
    app.click(Role::ListItem, "notes.txt");
    assert_eq!(app.picker(), None);
}

#[test]
fn copy_to_a_machine_lands_by_rule_and_the_name_arrives_when_it_is_done() {
    // PL-3, PL-5: a Home file sent to gpu-box asks only the machine and
    // lands in gpu-box's From This Mac place.
    let mut app = App::new();
    app.open_files();
    app.pick_on_right("gpu-box");
    app.click(Role::ListItem, "notes.txt");
    app.click(Role::Button, "Copy to \u{25b8}");
    app.click(Role::ListBoxOption, "gpu-box \u{b7} From This Mac");
    let transfers = app.ui.read(|state| state.files.transfers.clone());
    assert_eq!(transfers.len(), 1);
    let received = Folder::root(gpu(), FileRoot::Received { from: mac() });
    assert!(app.fake.names_in(&received).is_empty(), "nothing is there before it is done");

    app.fake.finish(&transfers[0].id);
    app.settle();
    assert_eq!(app.fake.names_in(&received), ["notes.txt"]);
    assert_eq!(app.names(Side::Left), ["notes.txt"], "a copy leaves the original");
}

#[test]
fn somewhere_specific_copies_into_the_other_panes_folder() {
    // PL-5: Somewhere specific… is the explicit form: the other pane's folder.
    let mut app = App::new();
    app.open_files();
    app.pick_on_right("gpu-box");
    app.click(Role::ListItem, "notes.txt");
    app.click(Role::Button, "Copy to \u{25b8}");
    app.click(Role::ListBoxOption, "Somewhere specific\u{2026}");
    let transfers = app.ui.read(|state| state.files.transfers.clone());
    assert_eq!(transfers.len(), 1);
    assert!(app.names(Side::Right).is_empty());

    app.fake.finish(&transfers[0].id);
    app.settle();
    assert_eq!(app.names(Side::Right), ["notes.txt"]);
}

#[test]
fn new_folder_asks_for_a_name_and_makes_the_folder() {
    let mut app = App::new();
    app.open_files();
    app.click(Role::Button, "New folder");
    assert!(app.ui.read(|state| state.files.naming.is_some()), "the prompt opens");

    // Confirming an empty field keeps the prompt open and says why.
    app.click(Role::Button, "Create");
    let error = app.ui.read(|state| state.files.naming.as_ref().and_then(|n| n.error.clone()));
    assert!(error.is_some());

    // The text goes in through the shell's input; the headless driver has no
    // focus to type into, so the confirmed text is sent as the button does.
    app.files(FilesAction::CommitName("made".to_owned()));
    assert!(app.ui.read(|state| state.files.naming.is_none()));
    assert!(app.names(Side::Left).contains(&"made".to_owned()));
}

#[test]
fn delete_moves_the_selection_to_the_trash() {
    let mut app = App::new();
    app.open_files();
    app.pick_on_right("gpu-box");
    app.click(Role::ListItem, "notes.txt");
    app.click(Role::Button, "Delete");
    assert!(app.names(Side::Left).is_empty());
    assert_eq!(app.ui.read(|state| state.files.trash_count), 1);
}
