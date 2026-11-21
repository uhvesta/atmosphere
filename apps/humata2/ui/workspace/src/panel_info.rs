//! The Info panel: agent, machine, clones, forks, recovery and permissions.
use crate::panel_prelude::*;
use humata2_pr_review::info::{InfoAction, Switch};

const RED: u32 = 0xff453a;

pub fn info(ui: &Rc<Ui>, state: &AppState, session: &Session, full: bool, look: Look) -> Div {
    let kv = |key: &'static str, value: Div| {
        row()
            .items_start()
            .gap(px(10.))
            .py(px(3.))
            .child(
                text(key)
                    .w(px(84.))
                    .flex_shrink_0()
                    .text_color(rgb(look.secondary)),
            )
            .child(value.flex_1().min_w_0())
    };
    let machine = state.workspace.machine(&session.machine);
    let forks = state.workspace.forks(&session.id).map(|fork| {
        let click_ui = ui.clone();
        let id = fork.id.clone();
        Button::new(SharedString::from(format!("session-info-fork {}", fork.id)))
            .accessibility_label(fork.title.clone())
            .ghost()
            .small()
            .w_full()
            .h(px(23.))
            .p_0()
            .child(
                text(fork.title.clone())
                    .w_full()
                    .text_left()
                    .text_color(rgb(look.accent)),
            )
            .on_click(move |_, _, cx| {
                click_ui.send(AppAction::Navigate(Route::Session(id.clone())), cx)
            })
    });
    let recovery = match (&session.recovery, &session.restart_detail) {
        (Some(recovery), Some(detail)) => format!("{}. {detail}", recovery.label()),
        (Some(recovery), None) => recovery.label(),
        (None, Some(detail)) => detail.clone(),
        (None, None) => "Recovery has not been reported for this session.".to_owned(),
    };
    // The session's own repository (its main clone) first, then the others.
    let mut clones: Vec<&str> = Vec::new();
    if !session.repo.is_empty() {
        clones.push(session.repo.as_str());
    }
    for clone in &session.clones {
        if !clones.contains(&clone.repo.as_str()) {
            clones.push(clone.repo.as_str());
        }
    }
    for file in state
        .workspace
        .session_files
        .get(&session.id)
        .into_iter()
        .flatten()
    {
        if !file.repo.is_empty() && !clones.contains(&file.repo.as_str()) {
            clones.push(file.repo.as_str());
        }
    }
    let chip = |label: String, color: u32| {
        text(label)
            .px(px(6.))
            .py(px(2.))
            .rounded(px(4.))
            .border_1()
            .border_color(rgb(look.line))
            .text_size(px(11.))
            .text_color(rgb(color))
    };
    let on_this_mac = machine.is_some_and(humata2_clients::is_this_mac);
    let clone_rows = clones.into_iter().map(|repo| {
        let main = repo == session.repo;
        // PL-10: the clone's real path sits behind its right-click menu.
        let path = session
            .clones
            .iter()
            .find(|clone| clone.repo == repo)
            .map(|clone| clone.path.clone())
            .filter(|path| !path.is_empty());
        let menu_open = path
            .as_ref()
            .is_some_and(|path| state.session.clone_menu.as_ref() == Some(path));
        let line = row()
            .id(SharedString::from(format!("session-info-clone {repo}")))
            .test_support()
            .flex_wrap()
            .gap(px(6.))
            .child(text(repo.rsplit('/').next().unwrap_or(repo).to_owned()))
            .child(chip(session.branch.clone(), look.secondary))
            .when(main, |row| {
                row.child(chip("main clone".to_owned(), look.accent))
            })
            .when_some(path.clone(), |line, path| {
                let click_ui = ui.clone();
                let toggled = if menu_open { None } else { Some(path) };
                line.on_mouse_down(MouseButton::Right, move |_, _, cx| {
                    click_ui.send(
                        AppAction::Session(SessionAction::CloneMenu(toggled.clone())),
                        cx,
                    )
                })
            });
        let choices = path.filter(|_| menu_open).map(|path| {
            let here = if on_this_mac {
                (
                    "Reveal in Finder",
                    SessionAction::RevealClone(path.clone()),
                )
            } else {
                (
                    "Open terminal here",
                    SessionAction::OpenTerminalIn {
                        session: session.id.clone(),
                        folder: path.clone(),
                    },
                )
            };
            let copy = ("Copy path", SessionAction::CopyClonePath(path));
            col().pl(px(8.)).children([here, copy].map(|(label, action)| {
                let click_ui = ui.clone();
                Button::new(SharedString::from(format!("session-info-clone-menu {repo} {label}")))
                    .accessibility_label(label)
                    .ghost()
                    .small()
                    .w_full()
                    .h(px(23.))
                    .p_0()
                    .child(text(label).w_full().text_left().text_color(rgb(look.accent)))
                    .on_click(move |_, _, cx| {
                        click_ui.send(AppAction::Session(action.clone()), cx)
                    })
            }))
        });
        col().child(line).children(choices)
    });
    let details = col()
        .child(kv(
            "Agent",
            text({
                let agent = if session.agent == humata2_clients::model::Agent::Claude {
                    "Claude Code"
                } else {
                    session.agent.name()
                };
                session
                    .model
                    .as_ref()
                    .map_or_else(|| agent.to_owned(), |model| format!("{agent} · {model}"))
            }),
        ))
        .child(kv(
            "Machine",
            text(machine.map_or_else(
                || session.machine.to_string(),
                |machine| format!("{} · {}", machine.name, machine.detail),
            )),
        ))
        .child(kv("Started", text(session.started.clone())))
        .child(kv("Forks", col().children(forks)))
        .child(kv("Clones", col().gap(px(3.)).children(clone_rows)));
    let restart = card(look)
        .p(px(10.))
        .gap(px(4.))
        .child(
            row()
                .gap(px(6.))
                .child(icon(IconName::ShieldCheck, GREEN, 12.))
                .child(text("Survives restarts").font_weight(FontWeight::SEMIBOLD)),
        )
        .child(secondary(look, recovery));
    let permissions = card(look)
        .p(px(10.))
        .gap(px(4.))
        .child(
            row()
                .gap(px(6.))
                .child(icon(IconName::Bot, look.accent, 12.))
                .child(text("What this agent may do").font_weight(FontWeight::SEMIBOLD)),
        )
        .when(session.permissions.is_empty(), |card| {
            card.child(secondary(
                look,
                "Permission rules have not been reported for this session.",
            ))
        })
        .children(
            session
                .permissions
                .iter()
                .map(|line| secondary(look, line.clone())),
        )
        .child(
            Button::new("session-channel-rules")
                .accessibility_label("Channel rules")
                .child(text("Channel rules").w_full().text_left())
                .ghost()
                .w_full()
                .small()
                .text_color(rgb(look.accent))
                .disabled(true),
        );
    let review = session
        .pull_request_review
        .then(|| review_switches(ui, state, look));
    col()
        .size_full()
        .child(panel_header(ui, state, Panel::Info, full, None, look))
        .child(
            col()
                .id(SharedString::from(format!("session-info {}", session.id)))
                .test_support()
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .gap(px(10.))
                .p(px(12.))
                .text_size(px(12.5))
                .child(details)
                .child(restart)
                .children(humata2_ui_desk::desk_info::access(ui, state, session, look))
                .children(humata2_ui_desk::desk_info::channel_switches(ui, state, session, look))
                .children(humata2_ui_desk::desk_info::targeting(ui, state, session, look))
                .children(review)
                .child(permissions),
        )
}

/// One on/off switch, drawn as the Settings pages draw theirs, that sends
/// `action` when clicked.
fn pr_switch(ui: &Rc<Ui>, id: &'static str, label: &'static str, on: bool, action: AppAction, look: Look) -> Div {
    let ui = ui.clone();
    row()
        .gap(px(10.))
        .items_center()
        .child(div().flex_1().min_w_0().child(label))
        .child(
            div()
                .id(id)
                .role(Role::Button)
                .aria_label(label)
                .aria_selected(on)
                .test_support()
                .w(px(28.))
                .h(px(16.))
                .flex_shrink_0()
                .rounded_full()
                .p(px(2.))
                .bg(rgb(if on { 0x28cd41 } else { look.fill }))
                .flex()
                .when(on, |x| x.justify_end())
                .child(div().size(px(12.)).rounded_full().bg(rgb(0xffffff)))
                .cursor_pointer()
                .on_click(move |_, _, cx| ui.send(action.clone(), cx)),
        )
}

/// A review's switches (PR-8, PR-25): whether the agent may push to the PR
/// branch, and which of Focus, Notes and Order it may do.
fn review_switches(ui: &Rc<Ui>, state: &AppState, look: Look) -> Div {
    let info = &state.pr_info;
    let agent = |id, label, which: Switch, on: bool| {
        pr_switch(ui, id, label, on, AppAction::PrInfo(InfoAction::SetSwitch(which, !on)), look)
    };
    card(look)
        .p(px(10.))
        .gap(px(6.))
        .child(
            row()
                .gap(px(6.))
                .child(icon(IconName::GitPullRequest, look.accent, 12.))
                .child(text("This review").font_weight(FontWeight::SEMIBOLD)),
        )
        .child(pr_switch(
            ui,
            "session-info-push",
            "Let the agent push to the PR branch",
            info.push_allowed,
            AppAction::PrInfo(InfoAction::SetPush(!info.push_allowed)),
            look,
        ))
        .child(secondary(look, "Agent may:"))
        .child(agent("session-info-agent-focus", "Focus", Switch::Focus, info.switches.focus))
        .child(agent("session-info-agent-notes", "Notes", Switch::Notes, info.switches.notes))
        .child(agent("session-info-agent-order", "Order", Switch::Order, info.switches.order))
        .when_some(info.error.as_ref(), |card, error| {
            card.child(
                row()
                    .id("session-info-review-error")
                    .test_support()
                    .gap(px(6.))
                    .child(div().flex_1().min_w_0().text_color(rgb(RED)).child(error.to_string()))
                    .child(
                        Button::new("session-info-review-error-dismiss")
                            .accessibility_label("Dismiss")
                            .ghost()
                            .small()
                            .child(text("Dismiss"))
                            .on_click({
                                let ui = ui.clone();
                                move |_, _, cx| ui.send(AppAction::PrInfo(InfoAction::DismissError), cx)
                            }),
                    ),
            )
        })
}
