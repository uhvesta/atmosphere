//! The tab strip over three terminal panes in a headless window, driven
//! through real input: the keyboard switches tabs and types into the
//! active one, ✕ closes a view and says so, a double-click renames, a
//! drag reorders.

mod support;

use std::cell::RefCell;
use std::rc::Rc;

// `#[gpui_kit::test]` expands to paths under `gpui`.
use gpui_kit::gpui;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, App, AppContext, Bounds, Entity, Focusable, Point, TestAppContext, Window,
    WindowBounds, WindowOptions, px, size,
};
use ghostty_vt::Terminal;
use support::{PtyLog, RecordingPty, shortcut};
use terminal_pane::{TabId, TabSpec, TabsEvent, TerminalPane, TerminalTabs};
use terminal_view::Theme;

struct Opened {
    window: AnyWindowHandle,
    tabs: Entity<TerminalTabs>,
    ids: Vec<TabId>,
    ptys: Vec<Rc<RefCell<PtyLog>>>,
    events: Rc<RefCell<Vec<TabsEvent>>>,
}

/// Opens a strip with a shell that has no ✕ and two closable terminals.
fn open(cx: &mut TestAppContext) -> Opened {
    let ptys: Vec<_> = (0..3).map(|_| Rc::new(RefCell::new(PtyLog::default()))).collect();
    let made = Rc::new(RefCell::new(Vec::new()));
    let window = {
        let ptys = ptys.clone();
        let made = made.clone();
        cx.update(move |cx| {
            gpui_kit::init(cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(720.), px(420.)),
                })),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, move |window, cx| {
                let tabs = cx.new(|_| TerminalTabs::new());
                let mut ids = Vec::new();
                for (n, pty) in ptys.iter().enumerate() {
                    let terminal = Terminal::new(80, 24, 100).expect("the terminal starts");
                    let pty = RecordingPty(pty.clone());
                    let pane = cx.new(|cx| TerminalPane::new(terminal, pty, Theme::default(), window, cx));
                    let focus = pane.read(cx).focus_handle(cx);
                    let spec = TabSpec {
                        title: format!("tab {n}").into(),
                        closable: n > 0,
                        content: pane.into(),
                        focus: Some(focus),
                    };
                    ids.push(tabs.update(cx, |tabs, cx| tabs.add_tab(spec, window, cx)));
                }
                *made.borrow_mut() = ids;
                tabs
            })
            .expect("the test window opens")
        })
    };
    let (window, tabs) = window;
    let ids = made.borrow_mut().clone();
    let events = Rc::new(RefCell::new(Vec::new()));
    let log = events.clone();
    cx.update(|cx| {
        cx.subscribe(&tabs, move |_, event: &TabsEvent, _| log.borrow_mut().push(event.clone()))
            .detach();
    });
    cx.update_window(window, |_, window, _| window.activate_window())
        .expect("the test window is open");
    cx.run_until_parked();
    let opened = Opened {
        window,
        tabs,
        ids,
        ptys,
        events,
    };
    // Adding the last tab focused it; draw until it has its size.
    opened.frame(cx, |_, _| {});
    opened.frame(cx, |_, _| {});
    for pty in &opened.ptys {
        pty.borrow_mut().take();
    }
    opened
}

impl Opened {
    fn frame<R>(&self, cx: &mut TestAppContext, step: impl FnOnce(&mut Window, &mut App) -> R) -> R {
        let result = cx
            .update_window(self.window, |_, window, cx| {
                let result = step(window, cx);
                window.render_frame(cx);
                result
            })
            .expect("the test window is open");
        cx.run_until_parked();
        result
    }

    fn active(&self, cx: &mut TestAppContext) -> Option<TabId> {
        self.tabs.read_with(cx, |tabs, _| tabs.active())
    }

    fn events(&self) -> Vec<TabsEvent> {
        std::mem::take(&mut self.events.borrow_mut())
    }

    /// What each tab's program was sent since the last call.
    fn sent(&self) -> Vec<Vec<u8>> {
        self.ptys.iter().map(|pty| pty.borrow_mut().take()).collect()
    }
}

fn next_tab() -> String {
    shortcut("cmd-shift-]", "ctrl-pagedown")
}

fn previous_tab() -> String {
    shortcut("cmd-shift-[", "ctrl-pageup")
}

fn nth_tab(n: usize) -> String {
    shortcut(&format!("cmd-{n}"), &format!("alt-{n}"))
}

#[gpui_kit::test]
fn the_keyboard_switches_tabs_and_types_into_the_active_one(cx: &mut TestAppContext) {
    let strip = open(cx);
    let [shell, second, third] = [strip.ids[0], strip.ids[1], strip.ids[2]];
    assert_eq!(strip.active(cx), Some(third));
    strip.frame(cx, |window, cx| window.press("a", cx));
    assert_eq!(strip.sent(), vec![vec![], vec![], b"a".to_vec()]);

    strip.frame(cx, |window, cx| window.press(&previous_tab(), cx));
    assert_eq!(strip.active(cx), Some(second));
    strip.frame(cx, |window, cx| window.press(&nth_tab(1), cx));
    assert_eq!(strip.active(cx), Some(shell));
    strip.frame(cx, |window, cx| window.press("b", cx));
    // ⌘9 / Alt+9 is the last tab however many there are.
    strip.frame(cx, |window, cx| window.press(&nth_tab(9), cx));
    assert_eq!(strip.active(cx), Some(third));
    // Next from the last wraps to the first.
    strip.frame(cx, |window, cx| window.press(&next_tab(), cx));
    assert_eq!(strip.active(cx), Some(shell));
    strip.frame(cx, |window, cx| window.press("ctrl-tab", cx));
    assert_eq!(strip.active(cx), Some(second));
    strip.frame(cx, |window, cx| window.press("c", cx));

    // The shortcuts reached no program.
    assert_eq!(strip.sent(), vec![b"b".to_vec(), b"c".to_vec(), vec![]]);
    assert_eq!(
        strip.events(),
        vec![
            TabsEvent::Activated(second),
            TabsEvent::Activated(shell),
            TabsEvent::Activated(third),
            TabsEvent::Activated(shell),
            TabsEvent::Activated(second),
        ]
    );

    // Clicking a tab activates it too.
    strip.frame(cx, |window, cx| window.click(format!("tab-{}", third.0), cx));
    assert_eq!(strip.active(cx), Some(third));
}

#[gpui_kit::test]
fn closing_a_tab_closes_its_view_and_not_its_process(cx: &mut TestAppContext) {
    let strip = open(cx);
    let [shell, second, third] = [strip.ids[0], strip.ids[1], strip.ids[2]];
    strip.frame(cx, |window, cx| window.click(format!("tab-close-{}", third.0), cx));
    assert_eq!(strip.tabs.read_with(cx, |tabs, _| tabs.tabs()), vec![shell, second]);
    // The strip only says the view closed; stopping is a separate request.
    assert_eq!(
        strip.events(),
        vec![TabsEvent::Closed(third), TabsEvent::Activated(second)]
    );
    strip.frame(cx, |window, cx| window.press("x", cx));
    assert_eq!(strip.sent(), vec![vec![], b"x".to_vec(), vec![]]);

    strip.frame(cx, |window, cx| window.press(&shortcut("cmd-w", "ctrl-shift-w"), cx));
    assert_eq!(strip.events(), vec![TabsEvent::Closed(second), TabsEvent::Activated(shell)]);

    // The shell has no ✕, and the close shortcut leaves it alone.
    strip.frame(cx, |window, cx| {
        assert!(window.try_find(format!("tab-close-{}", shell.0)).is_none());
        window.press(&shortcut("cmd-w", "ctrl-shift-w"), cx);
    });
    assert_eq!(strip.tabs.read_with(cx, |tabs, _| tabs.tabs()), vec![shell]);
    assert!(strip.events().is_empty());

    strip.tabs.update(cx, |tabs, cx| tabs.request_stop(shell, cx));
    assert_eq!(strip.events(), vec![TabsEvent::StopRequested(shell)]);
}

#[gpui_kit::test]
fn a_double_click_renames_a_tab_and_escape_keeps_the_old_name(cx: &mut TestAppContext) {
    let strip = open(cx);
    let second = strip.ids[1];
    let tab = format!("tab-{}", second.0);
    strip.frame(cx, |window, cx| window.double_click(tab.clone(), cx));
    assert_eq!(strip.tabs.read_with(cx, |tabs, _| tabs.is_renaming()), Some(second));
    // The whole old name is selected, so typing replaces it.
    strip.frame(cx, |window, cx| window.input("build", cx));
    strip.frame(cx, |window, cx| window.press("enter", cx));
    assert_eq!(strip.tabs.read_with(cx, |tabs, _| tabs.is_renaming()), None);
    let title = strip.tabs.read_with(cx, |tabs, _| tabs.title(second)).map(|title| title.to_string());
    assert_eq!(title.as_deref(), Some("build"));
    assert!(strip.events().contains(&TabsEvent::Renamed {
        id: second,
        title: "build".to_owned(),
    }));
    // Nothing typed into the name reached a program.
    assert!(strip.sent().iter().all(Vec::is_empty));

    strip.frame(cx, |window, cx| window.double_click(tab.clone(), cx));
    strip.frame(cx, |window, cx| window.input("oops", cx));
    strip.frame(cx, |window, cx| window.press("escape", cx));
    assert_eq!(strip.tabs.read_with(cx, |tabs, _| tabs.is_renaming()), None);
    let title = strip.tabs.read_with(cx, |tabs, _| tabs.title(second)).map(|title| title.to_string());
    assert_eq!(title.as_deref(), Some("build"));
    assert!(!strip.events().iter().any(|event| matches!(event, TabsEvent::Renamed { .. })));
}

#[gpui_kit::test]
fn dragging_a_tab_onto_another_moves_it_there(cx: &mut TestAppContext) {
    let strip = open(cx);
    let [shell, second, third] = [strip.ids[0], strip.ids[1], strip.ids[2]];
    strip.frame(cx, |window, cx| {
        window.drag_to(format!("tab-{}", third.0), format!("tab-{}", shell.0), cx);
    });
    assert_eq!(strip.tabs.read_with(cx, |tabs, _| tabs.tabs()), vec![third, shell, second]);
    assert!(strip.events().contains(&TabsEvent::Moved { id: third, to: 0 }));
}

#[gpui_kit::test]
fn the_new_tab_button_and_shortcut_ask_the_app_for_a_shell(cx: &mut TestAppContext) {
    let strip = open(cx);
    strip.frame(cx, |window, cx| window.click("tab-new", cx));
    strip.frame(cx, |window, cx| window.press(&shortcut("cmd-t", "ctrl-shift-t"), cx));
    assert_eq!(strip.events(), vec![TabsEvent::NewTab, TabsEvent::NewTab]);
    assert!(strip.sent().iter().all(Vec::is_empty));
}

#[gpui_kit::test]
fn an_offline_machine_shows_on_the_strip(cx: &mut TestAppContext) {
    let strip = open(cx);
    strip.tabs.update(cx, |tabs, cx| tabs.set_disconnected(true, cx));
    strip.frame(cx, |window, _| assert!(window.try_find("tabs-disconnected").is_some()));
    strip.tabs.update(cx, |tabs, cx| tabs.set_disconnected(false, cx));
    strip.frame(cx, |window, _| assert!(window.try_find("tabs-disconnected").is_none()));
}
