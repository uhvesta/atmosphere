//! Select mode's shared parts (MASSDELETE, D875): the tick box, the tint a
//! ticked row takes, the sidebar's action card and the 48 pt bottom bar on
//! Home and the machine page. The sidebar, Home and the machine page each
//! draw their own rows; they all read the same [`Snapshot`] and send the same
//! [`MassDeleteAction`]s.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

pub use humata2_mass_delete::{MassDeleteAction, Surface};

use gpui_kit::assets::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::{prelude::*, *};
use humata2_app::AppAction;
use humata2_clients::model::{ChannelId, MachineId, SessionId};

use crate::bridge::Ui;
use crate::chrome;
use crate::theme::{Look, RED_STATUS as RED};
use crate::widgets::{self, col, row};

/// A mass-delete action as the app's.
#[must_use]
pub fn act(action: MassDeleteAction) -> AppAction {
    AppAction::MassDelete(action)
}

/// What one tick box shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tick {
    /// Not ticked.
    Off,
    /// Ticked.
    On,
    /// A context fork that goes along: ticked, paler, can't be cleared.
    Along,
    /// Some of a channel's or machine's sessions are ticked.
    Dash,
    /// Nothing here can be ticked: an offline machine, an empty channel.
    Barred,
}

/// A tick box of `size` points, as the frames draw it.
#[must_use]
pub fn tick_box(look: Look, size: f32, tick: Tick) -> Div {
    let base = div()
        .size(px(size))
        .flex_shrink_0()
        .rounded(px(if size > 13.5 { 4. } else { 3.5 }))
        .flex()
        .items_center()
        .justify_center();
    match tick {
        Tick::Off => base
            .border_1()
            .border_color(rgb(look.tertiary))
            .bg(rgb(look.card)),
        Tick::On => base
            .bg(rgb(look.accent))
            .child(chrome::icon(IconName::Check, 0xffffff, size - 4.)),
        Tick::Along => base
            .bg(rgb(look.accent))
            .opacity(0.5)
            .child(chrome::icon(IconName::Check, 0xffffff, size - 4.)),
        Tick::Dash => base.bg(rgb(look.accent)).child(
            div()
                .w(px(size - 6.))
                .h(px(1.5))
                .rounded_full()
                .bg(rgb(0xffffff)),
        ),
        Tick::Barred => base
            .border_1()
            .border_dashed()
            .border_color(rgb(look.tertiary))
            .opacity(0.6),
    }
}

/// The faint accent tint behind a ticked row.
#[must_use]
pub fn tint(look: Look) -> Rgba {
    let accent = rgb(look.accent);
    Rgba {
        a: if look.sidebar == 0x2a2a2d { 0.22 } else { 0.16 },
        ..accent
    }
}

/// What select mode ticks, read once per draw.
#[derive(Clone, Debug)]
pub struct Snapshot {
    /// Where select mode was entered.
    pub surface: Surface,
    ticked: HashSet<SessionId>,
    along: HashSet<SessionId>,
    blocked: HashSet<SessionId>,
    channels: HashMap<ChannelId, (usize, usize)>,
    machines: HashMap<MachineId, (usize, usize)>,
    offline: HashSet<MachineId>,
    channel_members: HashMap<ChannelId, Vec<SessionId>>,
    machine_members: HashMap<MachineId, Vec<SessionId>>,
    picked_ids: HashSet<SessionId>,
    /// Picks plus the forks that go along.
    pub selected: usize,
    /// The sessions the person ticked.
    pub picked: usize,
    /// Forks that go along.
    pub forks: usize,
}

impl Snapshot {
    /// Select mode as it is now; `None` when it is off.
    #[must_use]
    pub fn read(ui: &Rc<Ui>) -> Option<Self> {
        ui.read(Self::of)
    }

    /// Select mode as `state` has it; `None` when it is off.
    #[must_use]
    pub fn of(state: &humata2_app::AppState) -> Option<Self> {
        {
            let m = &state.mass_delete;
            let surface = m.surface.clone()?;
            let along = m.forks_along();
            let ticked: HashSet<SessionId> =
                m.picked.iter().cloned().chain(along.iter().cloned()).collect();
            let mut channels: HashMap<ChannelId, (usize, usize)> = HashMap::new();
            let mut machines: HashMap<MachineId, (usize, usize)> = HashMap::new();
            let mut blocked = HashSet::new();
            let mut channel_members: HashMap<ChannelId, Vec<SessionId>> = HashMap::new();
            let mut machine_members: HashMap<MachineId, Vec<SessionId>> = HashMap::new();
            for entry in &m.listing.rows {
                if entry.desk {
                    blocked.insert(entry.id.clone());
                }
                let live = entry.live && !entry.archived && !entry.desk;
                if !live || m.listing.offline.contains(&entry.machine) {
                    continue;
                }
                channel_members
                    .entry(entry.channel.clone())
                    .or_default()
                    .push(entry.id.clone());
                machine_members
                    .entry(entry.machine.clone())
                    .or_default()
                    .push(entry.id.clone());
                let on = usize::from(ticked.contains(&entry.id));
                for (ticks, total) in [
                    channels.entry(entry.channel.clone()).or_default(),
                    machines.entry(entry.machine.clone()).or_default(),
                ] {
                    *ticks += on;
                    *total += 1;
                }
            }
            Some(Self {
                surface,
                ticked,
                along: along.iter().cloned().collect(),
                blocked,
                channels,
                machines,
                offline: m.listing.offline.iter().cloned().collect(),
                channel_members,
                machine_members,
                picked_ids: m.picked.iter().cloned().collect(),
                selected: m.selected_count(),
                picked: m.picked_count(),
                forks: along.len(),
            })
        }
    }

    /// Select mode as it is now, when it was entered on `surface`.
    #[must_use]
    pub fn on(ui: &Rc<Ui>, surface: &Surface) -> Option<Self> {
        Self::read(ui).filter(|snap| &snap.surface == surface)
    }

    /// What a session's box shows.
    #[must_use]
    pub fn tick(&self, id: &SessionId) -> Tick {
        if self.along.contains(id) {
            Tick::Along
        } else if self.ticked.contains(id) {
            Tick::On
        } else {
            Tick::Off
        }
    }

    /// Whether a session's box is ticked, picked or along.
    #[must_use]
    pub fn ticked(&self, id: &SessionId) -> bool {
        self.ticked.contains(id)
    }

    /// Whether a click on this session's box does anything.
    #[must_use]
    pub fn can_pick(&self, id: &SessionId) -> bool {
        !self.along.contains(id) && !self.blocked.contains(id)
    }

    /// What a channel's box shows, and whether it can be ticked.
    #[must_use]
    pub fn channel(&self, channel: &ChannelId) -> (Tick, bool) {
        Self::tri(self.channels.get(channel).copied(), false)
    }

    /// What a machine's box shows, and whether it can be ticked.
    #[must_use]
    pub fn machine(&self, machine: &MachineId) -> (Tick, bool) {
        Self::tri(self.machines.get(machine).copied(), self.offline.contains(machine))
    }

    /// What a click on a channel's box sends: everything in it unless it is
    /// all ticked, then the picks in it come off one by one.
    #[must_use]
    pub fn channel_click(&self, channel: &ChannelId) -> Vec<MassDeleteAction> {
        let all = self.channel(channel).0 == Tick::On;
        if all {
            self.untick(self.channel_members.get(channel))
        } else {
            vec![MassDeleteAction::SelectChannel(channel.clone())]
        }
    }

    /// What a click on a machine's box sends, as [`Self::channel_click`].
    #[must_use]
    pub fn machine_click(&self, machine: &MachineId) -> Vec<MassDeleteAction> {
        let all = self.machine(machine).0 == Tick::On;
        if all {
            self.untick(self.machine_members.get(machine))
        } else {
            vec![MassDeleteAction::SelectMachine(machine.clone())]
        }
    }

    fn untick(&self, members: Option<&Vec<SessionId>>) -> Vec<MassDeleteAction> {
        members
            .into_iter()
            .flatten()
            .filter(|id| self.picked_ids.contains(*id))
            .map(|id| MassDeleteAction::Toggle(id.clone()))
            .collect()
    }

    /// The sessions of a machine that a whole-machine pick takes, ticked
    /// of total.
    #[must_use]
    pub fn machine_counts(&self, machine: &MachineId) -> (usize, usize) {
        self.machines.get(machine).copied().unwrap_or_default()
    }

    /// The sessions of a channel that a whole-channel pick takes, ticked of
    /// total.
    #[must_use]
    pub fn channel_counts(&self, channel: &ChannelId) -> (usize, usize) {
        self.channels.get(channel).copied().unwrap_or_default()
    }

    fn tri(counts: Option<(usize, usize)>, offline: bool) -> (Tick, bool) {
        match counts {
            Some((ticked, total)) if total > 0 && !offline => (
                if ticked == 0 {
                    Tick::Off
                } else if ticked == total {
                    Tick::On
                } else {
                    Tick::Dash
                },
                true,
            ),
            _ => (Tick::Barred, false),
        }
    }

    /// "Delete N sessions…".
    #[must_use]
    pub fn delete_label(&self) -> String {
        match self.selected {
            1 => "Delete 1 session\u{2026}".to_owned(),
            n => format!("Delete {n} sessions\u{2026}"),
        }
    }

    /// "3 picked · 1 fork goes along": said only while forks go along.
    #[must_use]
    pub fn picked_note(&self) -> Option<String> {
        match self.forks {
            0 => None,
            1 => Some(format!("{} picked \u{b7} 1 fork goes along", self.picked)),
            k => Some(format!("{} picked \u{b7} {k} forks go along", self.picked)),
        }
    }
}

/// The key hint the bottom bar ends with.
pub const KEY_HINT: &str =
    "\u{2318}A all \u{b7} \u{21e7}-click a range \u{b7} space ticks \u{b7} esc cancels";

/// An outlined red "Delete N sessions…" button; off while nothing is
/// selected.
#[must_use]
pub fn delete_button(
    ui: &Rc<Ui>,
    id: &'static str,
    snap: &Snapshot,
    look: Look,
) -> impl IntoElement + Styled {
    let ui = ui.clone();
    let on = snap.selected > 0;
    let label = snap.delete_label();
    row()
        .id(id)
        .role(Role::Button)
        .aria_label(label.clone())
        .h(px(24.))
        .px(px(10.))
        .rounded(px(6.))
        .flex_shrink_0()
        .justify_center()
        .whitespace_nowrap()
        .border_1()
        .border_color(rgb(RED))
        .bg(rgb(look.card))
        .text_color(rgb(RED))
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .when(on, |button| {
            button.cursor_pointer().on_click(move |_, _, cx| {
                cx.stop_propagation();
                ui.send(act(MassDeleteAction::OpenPicked), cx);
            })
        })
        .when(!on, |button| button.opacity(0.45))
        .child(label)
        .test_support()
}

fn cancel_button(ui: &Rc<Ui>, id: &'static str, look: Look) -> Button {
    widgets::push_quiet(ui, id, "Cancel", act(MassDeleteAction::Exit), look)
}

/// The sidebar's card above Settings: the count, the note, Cancel and
/// Delete.
#[must_use]
pub fn card(ui: &Rc<Ui>, snap: &Snapshot, look: Look) -> Div {
    div().px(px(10.)).pb(px(6.)).flex_shrink_0().child(
        col()
            .id("select-card")
            .role(Role::Group)
            .aria_label("Select sessions")
            .gap(px(2.))
            .p(px(12.))
            .rounded(px(10.))
            .bg(rgb(look.card))
            .border_1()
            .border_color(rgb(look.line))
            .shadow_sm()
            .child(
                div()
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(format!("{} selected", snap.selected)),
            )
            .children(snap.picked_note().map(|note| {
                div()
                    .text_size(px(11.5))
                    .text_color(rgb(look.secondary))
                    .child(note)
            }))
            .child(
                row()
                    .mt(px(8.))
                    .gap(px(6.))
                    .child(cancel_button(ui, "select-cancel", look))
                    .child(delete_button(ui, "select-delete", snap, look).flex_1()),
            )
            .test_support(),
    )
}

/// What the bar's Select menu needs to name things, read from the app.
struct Names {
    channels: Vec<(ChannelId, String)>,
    machines: Vec<(MachineId, String, bool)>,
}

/// The bottom bar of Home and the machine page: the count, the note, the
/// Select pop-up, the key hint, Cancel and Delete (D875).
#[derive(IntoElement)]
pub struct Bar {
    ui: Rc<Ui>,
    snap: Snapshot,
    /// The sessions the page shows, top to bottom, for "Everything on …".
    visible: Vec<SessionId>,
    /// "Home" or the machine's name: the words after "Everything on".
    scope: String,
    /// The note's last part, such as "Review the schema is not on Home".
    outside: Option<String>,
    look: Look,
}

/// A bottom bar for `snap`, when select mode was entered on its surface.
#[must_use]
pub fn bar(
    ui: &Rc<Ui>,
    snap: Snapshot,
    visible: Vec<SessionId>,
    scope: impl Into<String>,
    look: Look,
) -> Bar {
    let scope = scope.into();
    let outside = ui.read(|state| {
        let outside: Vec<&str> = state
            .workspace
            .sessions
            .iter()
            .filter(|s| snap.ticked(&s.id) && !visible.contains(&s.id))
            .map(|s| s.title.as_str())
            .collect();
        match outside.as_slice() {
            [] => None,
            [one] => Some(format!("{one} is not on {scope}")),
            many => Some(format!("{} are not on {scope}", many.len())),
        }
    });
    Bar {
        ui: ui.clone(),
        snap,
        visible,
        scope,
        outside,
        look,
    }
}

impl RenderOnce for Bar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let look = self.look;
        let ui = self.ui.clone();
        let open = window.use_keyed_state("select-menu-open", cx, |_, _| false);
        let menu_open = *open.read(cx);
        let names = ui.read(|state| Names {
            channels: state
                .workspace
                .channels
                .iter()
                .map(|c| (c.id.clone(), c.name.clone()))
                .collect(),
            machines: state
                .workspace
                .machines
                .iter()
                .map(|m| (m.id.clone(), m.name.clone(), m.online))
                .collect(),
        });
        let note = self
            .snap
            .picked_note()
            .into_iter()
            .chain(self.outside.clone())
            .collect::<Vec<_>>()
            .join(" \u{b7} ");
        let note = if note.is_empty() {
            self.whole(&names).unwrap_or_default()
        } else {
            note
        };
        let toggle = {
            let open = open.clone();
            move |cx: &mut App| open.update(cx, |open, cx| {
                *open = !*open;
                cx.notify();
            })
        };
        let select = row()
            .id("select-menu")
            .role(Role::Button)
            .aria_label("Select")
            .h(px(24.))
            .pl(px(9.))
            .pr(px(5.))
            .gap(px(6.))
            .rounded(px(6.))
            .flex_shrink_0()
            .cursor_pointer()
            .bg(rgb(look.fill))
            .border_1()
            .border_color(rgb(if menu_open { look.accent } else { look.line }))
            .text_size(px(12.))
            .text_color(rgb(if menu_open { look.accent } else { look.text }))
            .child("Select")
            .child(chrome::icon(IconName::ChevronsUpDown, look.secondary, 12.))
            .on_click({
                let toggle = toggle.clone();
                move |_, _, cx| toggle(cx)
            })
            .test_support();
        row()
            .id("select-bar")
            .role(Role::Group)
            .aria_label("Selected sessions")
            .relative()
            .h(px(48.))
            .px(px(20.))
            .gap(px(12.))
            .flex_shrink_0()
            .border_t_1()
            .border_color(rgb(look.line))
            .bg(rgb(look.panel))
            .child(
                div()
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .flex_shrink_0()
                    .child(format!("{} selected", self.snap.selected)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(px(12.))
                    .text_color(rgb(look.secondary))
                    .child(note),
            )
            .child(select)
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(rgb(look.tertiary))
                    .whitespace_nowrap()
                    .flex_shrink_0()
                    .child(KEY_HINT),
            )
            .child(cancel_button(&ui, "select-bar-cancel", look))
            .child(delete_button(&ui, "select-bar-delete", &self.snap, look))
            .when(menu_open, |bar| {
                bar.child(deferred(self.menu(&names, open, look)).with_priority(1))
            })
    }
}

impl Bar {
    /// "All of # payments-rewrite" or "Every session on devbox-1", when the
    /// ticks are exactly that.
    fn whole(&self, names: &Names) -> Option<String> {
        if self.snap.selected == 0 {
            return None;
        }
        for (id, name) in &names.channels {
            let (ticked, total) = self.snap.channel_counts(id);
            if total > 0 && ticked == total && ticked == self.snap.selected {
                return Some(format!("All of # {name}"));
            }
        }
        for (id, name, _) in &names.machines {
            let (ticked, total) = self.snap.machine_counts(id);
            if total > 0 && ticked == total && ticked == self.snap.selected {
                return Some(format!("Every session on {name}"));
            }
        }
        None
    }

    /// The Select pop-up's menu, above the bar.
    fn menu(&self, names: &Names, open: Entity<bool>, look: Look) -> impl IntoElement {
        let ui = &self.ui;
        let close = {
            let open = open.clone();
            move |cx: &mut App| open.update(cx, |open, cx| {
                *open = false;
                cx.notify();
            })
        };
        let entry = |id: SharedString,
                     text: String,
                     right: String,
                     checked: bool,
                     action: Option<MassDeleteAction>| {
            let (ui, close) = (ui.clone(), close.clone());
            let live = action.is_some();
            row()
                .id(id)
                .role(Role::Button)
                .aria_label(text.clone())
                .h(px(25.))
                .px(px(8.))
                .gap(px(6.))
                .rounded(px(5.))
                .text_size(px(12.5))
                .text_color(rgb(if live { look.text } else { look.tertiary }))
                .when(live, |item| {
                    item.cursor_pointer().hover(|x| x.bg(rgb(look.fill)))
                })
                .child(div().w(px(14.)).flex_shrink_0().children(
                    checked.then(|| chrome::icon(IconName::Check, look.text, 12.)),
                ))
                .child(div().flex_1().whitespace_nowrap().child(text))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(look.secondary))
                        .child(right),
                )
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    if let Some(action) = &action {
                        ui.send(act(action.clone()), cx);
                        close(cx);
                    }
                })
                .test_support()
        };
        let head = |text: &'static str| {
            div()
                .px(px(8.))
                .pt(px(5.))
                .pb(px(2.))
                .text_size(px(11.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(look.secondary))
                .child(text)
        };
        let rule = || div().h(px(1.)).mx(px(4.)).my(px(4.)).bg(rgb(look.line));
        let mut menu = col()
            .id("select-menu-card")
            .role(Role::Menu)
            .aria_label("Select")
            .absolute()
            .bottom(px(52.))
            .right(px(380.))
            .w(px(236.))
            .p(px(5.))
            .gap(px(1.))
            .bg(rgb(look.card))
            .border_1()
            .border_color(rgb(look.line))
            .rounded(px(7.))
            .shadow_lg()
            .child(head("Every session in a channel"));
        for (id, name) in &names.channels {
            let (ticked, total) = self.snap.channel_counts(id);
            if total == 0 {
                continue;
            }
            menu = menu.child(entry(
                SharedString::from(format!("select-channel-{id}")),
                format!("# {name}"),
                total.to_string(),
                ticked == total,
                Some(MassDeleteAction::SelectChannel(id.clone())),
            ));
        }
        menu = menu.child(rule()).child(head("Every session on a machine"));
        for (id, name, online) in &names.machines {
            let (_, total) = self.snap.machine_counts(id);
            let offline = !online;
            if !offline && total == 0 {
                continue;
            }
            menu = menu.child(entry(
                SharedString::from(format!("select-machine-{id}")),
                name.clone(),
                if offline { "offline".to_owned() } else { total.to_string() },
                false,
                (!offline).then(|| MassDeleteAction::SelectMachine(id.clone())),
            ));
        }
        menu = menu
            .child(rule())
            .child(entry(
                "select-everything".into(),
                format!("Everything on {}", self.scope),
                "\u{2318}A".to_owned(),
                false,
                Some(MassDeleteAction::SelectAllVisible(self.visible.clone())),
            ))
            .child(entry(
                "select-nothing".into(),
                "Nothing".to_owned(),
                String::new(),
                false,
                Some(MassDeleteAction::SelectNothing),
            ));
        // A click anywhere else closes the menu.
        let scrim = div()
            .id("select-menu-scrim")
            .absolute()
            .top(px(-10_000.))
            .left(px(-10_000.))
            .w(px(20_000.))
            .h(px(20_000.))
            .on_click(move |_, _, cx| close(cx));
        div().absolute().inset_0().child(scrim).child(menu.test_support())
    }
}

/// What a click on a session row does: in select mode it ticks the row (or,
/// with Shift, the range from the last tick, over `visible`); outside it
/// Command-click enters select mode on `surface` with this row ticked; else
/// it sends `open`.
#[must_use]
pub fn row_click(
    ui: &Rc<Ui>,
    surface: Surface,
    visible: Rc<Vec<SessionId>>,
    id: SessionId,
    selecting: bool,
    open: AppAction,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    let ui = ui.clone();
    move |event, _, cx| {
        let keys = event.modifiers();
        if selecting {
            cx.stop_propagation();
            let action = if keys.shift {
                MassDeleteAction::RangeTo {
                    to: id.clone(),
                    visible: visible.to_vec(),
                }
            } else {
                MassDeleteAction::Toggle(id.clone())
            };
            ui.send(act(action), cx);
        } else if keys.platform {
            cx.stop_propagation();
            ui.send(
                act(MassDeleteAction::Enter {
                    surface: surface.clone(),
                    first: id.clone(),
                }),
                cx,
            );
        } else {
            ui.send(open.clone(), cx);
        }
    }
}
