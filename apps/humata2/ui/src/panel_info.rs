//! The Info panel: agent, machine, clones, forks, recovery and permissions.
use super::*;

pub(super) fn info(ui: &Rc<Ui>, state: &AppState, session: &Session, full: bool, look: Look) -> Div {
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
    for file in state
        .workspace
        .session_files
        .get(&session.id)
        .into_iter()
        .flatten()
    {
        if !clones.contains(&file.repo.as_str()) {
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
    let clone_rows = clones.into_iter().map(|repo| {
        let main = repo == session.repo;
        row()
            .flex_wrap()
            .gap(px(6.))
            .child(text(repo.rsplit('/').next().unwrap_or(repo).to_owned()))
            .child(chip(session.branch.clone(), look.secondary))
            .when(main, |row| {
                row.child(chip("main clone".to_owned(), look.accent))
            })
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
    col()
        .size_full()
        .child(header(ui, Panel::Info, full, None, look))
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
                .children(crate::desk_info::access(ui, state, session, look))
                .child(permissions),
        )
}
