//! The Review a pull request dialog (PR-1 to PR-8, D283, D287; frame 67),
//! and the parts it shares with Submit review (frame 69).
//!
//! It draws [`DialogState`] and sends [`DialogAction`]s; nothing here decides
//! anything. The pasted link is read at once, so the card shows what is
//! being fetched, what came back, or why it couldn't be read, with Try again
//! (pick 11). Start is disabled until every pull request is read and a
//! machine is picked.
//!
//! This file is a child module of `overlays.rs`; Submit review is its child
//! (`overlay_submit_review.rs`).

use super::{Kind, Overlays, sheet, sync};
use humata2_ui_core::bridge::Ui;
use humata2_ui_core::chrome;
use humata2_ui_core::chrome::runs;
use humata2_ui_core::theme::{self, Look};
use humata2_ui_core::widgets::{self, col, row};
use gpui_kit::assets::IconName;
use gpui_kit::base::Disableable as _;
use gpui_kit::base::FocusTrapElement as _;
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::Sizable as _;
use gpui_kit::{prelude::*, *};
use humata2_app::AppAction;
use humata2_clients::model::Agent;
use humata2_clients::pull_requests::{PullRequestInfo, PullRequestRef, PullRequestState};
use humata2_pr_review::dialog::{
    Candidate, DetectedStack, DialogAction, DialogState, Fetch, PasteProblem, StackImportMode,
    Start,
};
use humata2_pr_review::submit::Step;
use std::cell::Cell;
use std::rc::Rc;

#[path = "overlay_submit_review.rs"]
mod submit_review;

/// The pop-up menu a dialog row has open.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Menu {
    Closed,
    Machine,
    Agent,
}

/// The two dialogs' text fields, and which of their pop-ups and fields are
/// open. The overlays own one.
pub(super) struct Inputs {
    paste: Entity<InputState>,
    also: Entity<InputState>,
    context: Entity<TextareaState>,
    /// Whether the "+ add a PR in another repo" field shows.
    also_open: Cell<bool>,
    menu: Cell<Menu>,
    submit: submit_review::Fields,
}

impl Inputs {
    /// Makes the fields and subscribes them: Return in the link field reads
    /// the pull request, Return in the Also review field adds it, and typing
    /// in Context sets what the agent is told first.
    pub(super) fn new(
        window: &mut Window,
        cx: &mut Context<Overlays>,
        subscriptions: &mut Vec<Subscription>,
    ) -> Self {
        let paste = cx.new(|cx| InputState::new(window, cx).placeholder("Paste a pull request link"));
        let also = cx.new(|cx| InputState::new(window, cx).placeholder("Paste another pull request link"));
        let context = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("What should the agent look at first? Optional.")
                .auto_grow(2, 6)
        });
        subscriptions.push(cx.subscribe(
            &paste,
            |this: &mut Overlays, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    let text = input.read(cx).value().to_string();
                    this.ui.send(AppAction::PrDialog(DialogAction::Paste(text)), cx);
                }
            },
        ));
        subscriptions.push(cx.subscribe(
            &also,
            |this: &mut Overlays, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    let text = input.read(cx).value().to_string();
                    this.pr.also_open.set(false);
                    this.ui.send(AppAction::PrDialog(DialogAction::AddAlso(text)), cx);
                    cx.notify();
                }
            },
        ));
        subscriptions.push(cx.subscribe(
            &context,
            |this: &mut Overlays, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let text = input.read(cx).value().to_string();
                    this.ui.send(AppAction::PrDialog(DialogAction::SetContext(text)), cx);
                }
            },
        ));
        Self {
            paste,
            also,
            context,
            also_open: Cell::new(false),
            menu: Cell::new(Menu::Closed),
            submit: submit_review::Fields::default(),
        }
    }
}

/// Sets a text area's text to `value` when the stored value changed.
fn sync_area(input: &Entity<TextareaState>, value: &str, window: &mut Window, cx: &mut App) {
    let last = window.use_keyed_state(
        ("overlay-sync-area", input.entity_id()),
        cx,
        |_, _| String::new(),
    );
    if last.read(cx).as_str() == value {
        return;
    }
    last.update(cx, |synced, _| {
        synced.clear();
        synced.push_str(value);
    });
    if input.read(cx).value().as_ref() != value {
        let value = value.to_owned();
        input.update(cx, |input, cx| input.set_value(value, window, cx));
    }
}

/// The agents a review can run: the enabled ones, never a plain shell
/// (AGENTSET).
fn review_agents(ui: &Ui) -> Vec<Agent> {
    ui.read(|state| state.settings.offered_agents())
}

impl Overlays {
    /// Which of the two pull request dialogs is open: the start dialog
    /// first, then Submit review.
    pub(super) fn pr_kind(&self) -> Option<Kind> {
        self.ui.read(|state| {
            if state.pr_dialog.open {
                Some(Kind::PrReview)
            } else if state.pr_submit.step != Step::Closed {
                Some(Kind::SubmitReview)
            } else {
                None
            }
        })
    }

    /// Puts focus in the dialog that just opened, with its fields empty.
    pub(super) fn pr_focus(&self, kind: Kind, window: &mut Window, cx: &mut Context<Self>) {
        self.pr.menu.set(Menu::Closed);
        self.pr.also_open.set(false);
        if kind == Kind::PrReview {
            self.pr.paste.update(cx, |input, cx| {
                input.set_value("", window, cx);
                input.focus(window, cx);
            });
            self.pr.also.update(cx, |input, cx| input.set_value("", window, cx));
            self.pr.context.update(cx, |input, cx| input.set_value("", window, cx));
        } else {
            self.pr.submit.reset();
            window.focus(&self.sheet, cx);
        }
    }

    /// The open pull request dialog, if one is.
    pub(super) fn pr_overlay(
        &self,
        look: Look,
        ui: &Rc<Ui>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        match self.pr_kind()? {
            Kind::PrReview => {
                let state = ui.read(|state| state.pr_dialog.clone());
                Some(self.pr_review(&state, look, ui, window, cx))
            }
            _ => {
                let state = ui.read(|state| state.pr_submit.clone());
                Some(self.submit_review(&state, look, ui, window, cx))
            }
        }
    }

    fn pr_review(
        &self,
        state: &DialogState,
        look: Look,
        ui: &Rc<Ui>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let machines = ui.read(|state| state.workspace.machines.clone());
        let machine_name = state
            .machine
            .as_ref()
            .and_then(|id| machines.iter().find(|machine| &machine.id == id))
            .map(|machine| machine.name.clone());
        // What the link field shows follows the pull request that was read.
        let typed = self.pr.paste.read(cx).value().to_string();
        if let Some(main) = &state.main
            && PullRequestRef::parse(&typed).as_ref() != Some(&main.pr)
        {
            let url = format!("https://github.com/{}/pull/{}", main.pr.repo, main.pr.number);
            sync(&self.pr.paste, &url, window, cx);
        }
        sync_area(&self.pr.context, &state.context, window, cx);
        if !self.pr.also_open.get() {
            sync(&self.pr.also, "", window, cx);
        }
        let gh = machine_name
            .as_deref()
            .map_or_else(|| "gh".to_owned(), |name| format!("{name}'s gh"));

        // The strip above the dialog: the pasted link and Return.
        let strip = row()
            .id("pr-review-paste")
            .role(Role::Group)
            .aria_label("Pull request link")
            .test_support()
            .gap(px(8.))
            .h(px(34.))
            .px(px(12.))
            .rounded(px(9.))
            .bg(rgb(look.card))
            .border_1()
            .border_color(rgb(look.line))
            .shadow_lg()
            .text_size(px(12.5))
            .child(chrome::icon(IconName::Search, look.secondary, 15.))
            .child(
                div().flex_1().min_w_0().ml(px(-10.)).child(
                    Input::new(&self.pr.paste)
                        .appearance(false)
                        .text_size(px(12.5))
                        .aria_label("Pull request link"),
                ),
            )
            .child(kbd(look, "\u{21a9}"));

        let blocker = state.blocker();
        let starting = state.start == Start::Starting;

        let viewport_h = f32::from(window.viewport_size().height);
        let max_card_h = if viewport_h > 0. {
            (viewport_h - 140.).max(260.)
        } else {
            560.
        };
        let mut body = sheet("pr-review-dialog", "Review a pull request", look)
            .w(px(640.))
            .max_w_full()
            .max_h(px(max_card_h))
            .gap(px(11.))
            .p(px(20.))
            .rounded(px(12.))
            .text_size(px(12.5))
            .test_support()
            .child(
                row()
                    .gap(px(8.))
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Review a pull request"),
                    )
                    .child(secondary(look, "in # pr-reviews")),
            );
        if let Some(problem) = &state.paste_problem {
            body = body.child(alert(
                "pr-review-paste-problem",
                match problem {
                    PasteProblem::NotAPullRequest { text } => format!(
                        "\u{201c}{text}\u{201d} isn't a pull request. Paste a link like https://github.com/owner/repo/pull/123, or write owner/repo#123."
                    ),
                    PasteProblem::AlreadyAdded(pr) => format!("{pr} is in this review already."),
                },
            ));
        }
        body = match &state.main {
            Some(main) => body.child(self.candidate_card(main, &gh, look, ui)),
            None => body.child(
                col()
                    .id("pr-review-empty")
                    .role(Role::Group)
                    .aria_label("No pull request yet")
                    .test_support()
                    .p(px(10.))
                    .rounded(px(9.))
                    .border_1()
                    .border_color(rgb(look.line))
                    .child(secondary(
                        look,
                        "Paste a pull request link in the box above and press Return; Humata reads it with the chosen machine's gh.",
                    )),
            ),
        };
        if let Some(stack) = &state.detected_stack
            && stack.is_multi_layer()
        {
            let main_pr = state.main.as_ref().map(|c| &c.pr);
            body = body.child(stack_detected_card(stack, main_pr, look, ui));
        }
        body = body
            .child(form_row(look, "Also review", self.also_review(state, look, ui, cx)))
            .child(form_row(look, "Machine", self.machine_row(state, &machines, look, ui, cx)))
            .child(form_row(look, "Agent", self.agent_row(state, look, ui, cx)))
            .child(form_row(
                look,
                "Context",
                div()
                    .id("pr-review-context")
                    .role(Role::Group)
                    .aria_label("Context")
                    .test_support()
                    .min_h(px(46.))
                    // Frame 67 sits the text lower in its box; the box keeps
                    // its height, the space moved from below the text.
                    .pt(px(10.))
                    .px(px(4.))
                    .rounded(px(6.))
                    .bg(rgb(look.card))
                    .border_1()
                    .border_color(rgb(look.line))
                    .child(
                        Textarea::new(&self.pr.context)
                            .xsmall()
                            .appearance(false)
                            .bordered(false)
                            .text_size(px(12.5))
                            .aria_label("Context"),
                    ),
            ))
            .child(form_row(
                look,
                "",
                check(
                    ui,
                    look,
                    "pr-review-push",
                    "Let the agent push to the PR branch",
                    state.push_allowed,
                    AppAction::PrDialog(DialogAction::AllowPush(!state.push_allowed)),
                ),
            ));
        if let Start::Failed(error) = &state.start {
            body = body.child(alert("pr-review-start-error", error.to_string()));
        }
        let start_label = if starting {
            "Starting\u{2026}".to_owned()
        } else if let Some(stack) = &state.detected_stack
            && stack.is_multi_layer()
            && stack.mode == StackImportMode::WholeStack
        {
            format!("Import stack ({} PRs)", stack.layers.len())
        } else {
            "Start the review".to_owned()
        };
        let body = body
            .child(
                row()
                    .gap(px(8.))
                    .pt(px(4.))
                    .child(div().flex_1())
                    .child(widgets::push_quiet(
                        ui,
                        "pr-review-cancel",
                        "Cancel",
                        AppAction::PrDialog(DialogAction::Close),
                        look,
                    ))
                    .child(
                        widgets::push_primary(
                            ui,
                            "pr-review-start",
                            start_label,
                            AppAction::PrDialog(DialogAction::Start),
                            look,
                        )
                        .disabled(blocker.is_some()),
                    ),
            )
            .children(
                state
                    .progress
                    .as_ref()
                    .filter(|_| starting)
                    .map(|p| widgets::progress_line(look, p, Some(ui.now()))),
            )
            .focus_trap("pr-review-dialog", &self.pr.paste.focus_handle(cx));
        // The strip sits above the dialog, 56 pt up (the dialog is drawn 96
        // pt below the window's top edge and the strip 40).
        div()
            .relative()
            .child(
                div()
                    .absolute()
                    .top(px(-56.))
                    .left_0()
                    .w_full()
                    .child(strip),
            )
            .child(body)
            .into_any_element()
    }

    /// One pull request as a card: what it is, or that it is being read, or
    /// why it couldn't be, with Try again.
    fn candidate_card(&self, candidate: &Candidate, gh: &str, look: Look, ui: &Rc<Ui>) -> impl IntoElement {
        let pr = &candidate.pr;
        let head = row()
            .gap(px(7.))
            .child(chrome::icon(IconName::GitPullRequest, look.accent, 12.))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(format!("{} #{}", pr.repo, pr.number)),
            );
        let shell = card(look)
            .id(SharedString::from(format!("pr-review-card-{pr}")))
            .role(Role::Group)
            .aria_label(format!("{} #{}", pr.repo, pr.number))
            .test_support()
            .p(px(10.))
            .gap(px(4.));
        match &candidate.fetch {
            Fetch::Fetching => shell
                .child(head)
                .child(
                    div()
                        .id(SharedString::from(format!("pr-review-fetching-{pr}")))
                        .role(Role::Status)
                        .aria_label(format!("Fetching {} #{}", pr.repo, pr.number))
                        .test_support()
                        .text_color(rgb(look.secondary))
                        .child(format!("Fetching {} #{} with {gh}\u{2026}", pr.repo, pr.number)),
                )
                .into_any_element(),
            Fetch::Failed(error) => shell
                .child(head)
                .child(alert(
                    SharedString::from(format!("pr-review-error-{pr}")),
                    error.to_string(),
                ))
                .child(
                    row().child(widgets::push_quiet(
                        ui,
                        SharedString::from(format!("pr-review-retry-{pr}")),
                        "Try again",
                        AppAction::PrDialog(DialogAction::Retry(pr.clone())),
                        look,
                    )),
                )
                .into_any_element(),
            Fetch::Found(info) => facts(shell.child(head.child(
                div()
                    .flex_1()
                    .min_w_0()
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .child(info.title.clone()),
            )), info, look)
            .into_any_element(),
        }
    }

    /// The Also review row: add a pull request, and the ones added.
    fn also_review(
        &self,
        state: &DialogState,
        look: Look,
        ui: &Rc<Ui>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let add = widgets::push_quiet_shape("pr-review-add-also", "+ add a PR in another repo", look).on_click(
            cx.listener(|this: &mut Overlays, _: &ClickEvent, window, cx| {
                this.pr.also_open.set(true);
                this.pr.also.update(cx, |input, cx| input.focus(window, cx));
                cx.notify();
            }),
        );
        let mut line = row().flex_wrap().gap(px(10.)).child(add);
        for candidate in &state.also {
            line = line.child(also_chip(candidate, look, ui));
        }
        let mut value = col().gap(px(6.)).child(line);
        if self.pr.also_open.get() {
            value = value.child(
                div()
                    .id("pr-review-also-field")
                    .role(Role::Group)
                    .aria_label("Another pull request")
                    .test_support()
                    .h(px(26.))
                    .child(
                        Input::new(&self.pr.also)
                            .small()
                            .min_h(px(26.))
                            .text_size(px(12.5))
                            .focus_bordered(false)
                            .aria_label("Another pull request"),
                    ),
            );
        }
        for candidate in &state.also {
            if let Fetch::Failed(error) = &candidate.fetch {
                value = value.child(alert(
                    SharedString::from(format!("pr-review-error-{}", candidate.pr)),
                    error.to_string(),
                ));
            }
        }
        value.into_any_element()
    }

    fn machine_row(
        &self,
        state: &DialogState,
        machines: &[humata2_clients::model::Machine],
        look: Look,
        ui: &Rc<Ui>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let name = state
            .machine
            .as_ref()
            .and_then(|id| machines.iter().find(|machine| &machine.id == id))
            .map_or("Choose a machine", |machine| machine.name.as_str());
        let list = machines
            .iter()
            .map(|machine| {
                if machine.online {
                    machine.name.clone()
                } else {
                    format!("{} (offline)", machine.name)
                }
            })
            .collect::<Vec<_>>()
            .join(" \u{b7} ");
        let mut value = col().gap(px(6.)).child(
            row()
                .gap(px(8.))
                .child(self.popup("pr-review-machine", name.to_owned(), Menu::Machine, look, cx))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_size(px(11.))
                        .text_color(rgb(if state.machine.is_none() {
                            theme::RED_STATUS
                        } else {
                            look.tertiary
                        }))
                        .child(if state.machine.is_none() {
                            "Pick the machine that clones the pull request and runs the agent.".to_owned()
                        } else {
                            list
                        }),
                ),
        );
        if self.pr.menu.get() == Menu::Machine {
            value = value.child(row().flex_wrap().gap(px(6.)).children(
                machines.iter().filter(|machine| machine.online).map(|machine| {
                    self.pick(
                        SharedString::from(format!("pr-review-machine-{}", machine.id)),
                        machine.name.clone(),
                        AppAction::PrDialog(DialogAction::PickMachine(machine.id.clone())),
                        look,
                        ui,
                        cx,
                    )
                }),
            ));
        }
        value.into_any_element()
    }

    fn agent_row(&self, state: &DialogState, look: Look, ui: &Rc<Ui>, cx: &mut Context<Self>) -> AnyElement {
        let mut value = col().gap(px(6.)).child(
            row()
                .child(self.popup(
                    "pr-review-agent",
                    runs(state.agent).to_owned(),
                    Menu::Agent,
                    look,
                    cx,
                ))
                .child(div().flex_1()),
        );
        if self.pr.menu.get() == Menu::Agent {
            value = value.child(row().flex_wrap().gap(px(6.)).children(review_agents(&self.ui).into_iter().map(|agent| {
                self.pick(
                    SharedString::from(format!("pr-review-agent-{}", agent.name().to_lowercase())),
                    runs(agent).to_owned(),
                    AppAction::PrDialog(DialogAction::PickAgent(agent)),
                    look,
                    ui,
                    cx,
                )
            })));
        }
        value.into_any_element()
    }

    /// A pop-up button that opens or closes `menu`.
    fn popup(
        &self,
        id: &'static str,
        text: String,
        menu: Menu,
        look: Look,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        row()
            .id(id)
            .role(Role::Button)
            .aria_label(text.clone())
            .test_support()
            .gap(px(6.))
            .h(px(24.))
            .pl(px(9.))
            .pr(px(5.))
            .rounded(px(6.))
            .bg(rgb(look.fill))
            .border_1()
            .border_color(rgb(look.line))
            .text_size(px(12.))
            .flex_shrink_0()
            .cursor_pointer()
            .child(text)
            .child(chrome::icon(IconName::ChevronsUpDown, look.secondary, 12.))
            .on_click(cx.listener(move |this: &mut Overlays, _: &ClickEvent, _, cx| {
                let next = if this.pr.menu.get() == menu { Menu::Closed } else { menu };
                this.pr.menu.set(next);
                cx.notify();
            }))
    }

    /// A choice in an open pop-up: sends `action` and closes the menu.
    fn pick(
        &self,
        id: SharedString,
        text: String,
        action: AppAction,
        look: Look,
        _ui: &Rc<Ui>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        widgets::push_quiet_shape(id, text, look).on_click(cx.listener(
            move |this: &mut Overlays, _: &ClickEvent, _, cx| {
                this.pr.menu.set(Menu::Closed);
                this.ui.send(action.clone(), cx);
                cx.notify();
            },
        ))
    }
}

/// The stack-import card shown when the pasted PR belongs to a multi-layer stack.
fn stack_detected_card(
    stack: &DetectedStack,
    main_pr: Option<&PullRequestRef>,
    look: Look,
    ui: &Rc<Ui>,
) -> impl IntoElement {
    let whole_selected = stack.mode == StackImportMode::WholeStack;
    let header = row()
        .gap(px(8.))
        .items_center()
        .child(chrome::icon(IconName::GitPullRequest, look.accent, 12.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(12.))
                .child(format!(
                    "Stacked PRs detected \u{b7} {} layers (bottom to top)",
                    stack.layers.len()
                )),
        )
        .child(
            div()
                .px(px(6.))
                .py(px(1.))
                .rounded(px(4.))
                .bg(rgb(look.fill))
                .text_size(px(10.5))
                .text_color(rgb(look.secondary))
                .child(stack.source.clone()),
        );

    let toggle = row()
        .gap(px(6.))
        .child(stack_mode_pill(
            "pr-review-stack-whole",
            "Import whole stack",
            whole_selected,
            StackImportMode::WholeStack,
            look,
            ui,
        ))
        .child(stack_mode_pill(
            "pr-review-stack-single",
            "Review single PR",
            !whole_selected,
            StackImportMode::SinglePr,
            look,
            ui,
        ));

    let mut layers = col().gap(px(4.));
    for (index, layer) in stack.layers.iter().enumerate() {
        let is_main = main_pr == Some(&layer.pr);
        let dimmed = !whole_selected && !is_main;
        let text_color = if dimmed { look.tertiary } else { look.secondary };
        let title_color = if dimmed { look.tertiary } else { look.text };
        let badge_bg = if dimmed { look.fill } else { look.accent };
        let badge_fg = if dimmed { look.tertiary } else { 0xffffff };

        let mut layer_row = row()
            .gap(px(7.))
            .items_center()
            .text_size(px(11.5))
            .child(
                row()
                    .size(px(16.))
                    .flex_shrink_0()
                    .rounded(px(8.))
                    .justify_center()
                    .items_center()
                    .bg(rgb(badge_bg))
                    .text_size(px(10.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(badge_fg))
                    .child(format!("{}", index + 1)),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(title_color))
                    .child(format!("#{}", layer.pr.number)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .text_color(rgb(title_color))
                    .child(layer.title.clone()),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(11.))
                    .text_color(rgb(text_color))
                    .child(format!("{} \u{2192} {}", layer.head_branch, layer.base_branch)),
            );

        if is_main {
            layer_row = layer_row.child(
                div()
                    .flex_shrink_0()
                    .px(px(5.))
                    .py(px(1.))
                    .rounded(px(4.))
                    .bg(rgb(look.fill))
                    .text_size(px(10.))
                    .text_color(rgb(look.secondary))
                    .child("pasted"),
            );
        }

        layers = layers.child(layer_row);
    }

    card(look)
        .id("pr-review-stack-detected")
        .role(Role::Group)
        .aria_label(format!(
            "Stacked PRs detected, {} layers",
            stack.layers.len()
        ))
        .test_support()
        .p(px(10.))
        .gap(px(8.))
        .child(header)
        .child(toggle)
        .child(layers)
}

/// One option in the stack import mode segmented pill.
fn stack_mode_pill(
    id: &'static str,
    label: &'static str,
    selected: bool,
    mode: StackImportMode,
    look: Look,
    ui: &Rc<Ui>,
) -> impl IntoElement {
    let ui = ui.clone();
    row()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .aria_selected(selected)
        .test_support()
        .px(px(10.))
        .h(px(24.))
        .rounded(px(6.))
        .items_center()
        .cursor_pointer()
        .text_size(px(11.5))
        .font_weight(if selected {
            FontWeight::SEMIBOLD
        } else {
            FontWeight::NORMAL
        })
        .bg(rgb(if selected { look.accent } else { look.fill }))
        .text_color(rgb(if selected { 0xffffff } else { look.secondary }))
        .child(label)
        .on_click(move |_, _, cx| {
            ui.send(
                AppAction::PrDialog(DialogAction::SetStackImportMode(mode)),
                cx,
            );
        })
}

/// One pull request under Also review: a chip with a remove button.
fn also_chip(candidate: &Candidate, look: Look, ui: &Rc<Ui>) -> impl IntoElement {
    let pr = &candidate.pr;
    let ui = ui.clone();
    let remove = AppAction::PrDialog(DialogAction::RemoveAlso(pr.clone()));
    let note = match &candidate.fetch {
        Fetch::Fetching => Some(("Fetching\u{2026}", look.secondary)),
        Fetch::Failed(_) => Some(("couldn't read it", theme::RED_STATUS)),
        Fetch::Found(info) => match info.state {
            PullRequestState::Open => None,
            PullRequestState::Merged => Some(("merged", look.secondary)),
            PullRequestState::Closed => Some(("closed", look.secondary)),
        },
    };
    row()
        .id(SharedString::from(format!("pr-review-also-{pr}")))
        .role(Role::Group)
        .aria_label(format!("{} #{}", pr.repo, pr.number))
        .test_support()
        .gap(px(6.))
        .px(px(8.))
        .h(px(24.))
        .rounded(px(6.))
        .bg(rgb(look.fill))
        .child(chrome::icon(IconName::GitPullRequest, look.secondary, 12.))
        .child(div().whitespace_nowrap().child(format!("{} #{}", pr.repo, pr.number)))
        .child(
            div()
                .id(SharedString::from(format!("pr-review-also-linked-{pr}")))
                .test_support()
                .child(secondary(look, "linked")),
        )
        .children(note.map(|(text, color)| secondary(look, text).text_color(rgb(color))))
        .child(
            div()
                .id(SharedString::from(format!("pr-review-also-remove-{pr}")))
                .role(Role::Button)
                .aria_label(format!("Remove {} #{}", pr.repo, pr.number))
                .test_support()
                .cursor_pointer()
                .child(chrome::icon(IconName::X, look.tertiary, 12.))
                .on_click(move |_, _, cx| ui.send(remove.clone(), cx)),
        )
}

/// The facts of a pull request read: who, branches, size and checks.
fn facts(card: impl ParentElement + IntoElement, info: &PullRequestInfo, look: Look) -> impl IntoElement {
    let mut card = card
        .child(secondary(
            look,
            format!("by {} \u{b7} {} \u{2192} {}", info.author, info.head_branch, info.base_branch),
        ))
        .child(
            row()
                .gap(px(8.))
                .text_size(px(11.5))
                .child(div().text_color(rgb(look.secondary)).child(format!("{} files", info.files)))
                .child(div().text_color(rgb(theme::GREEN)).child(format!("+{}", info.added)))
                .child(div().text_color(rgb(theme::RED_STATUS)).child(format!("\u{2212}{}", info.removed))),
        );
    if !info.checks.trim().is_empty() {
        let parts: Vec<&str> = info.checks.split(", ").collect();
        let last = parts.len().saturating_sub(1);
        let mut line = row().gap(px(5.)).text_size(px(11.5));
        for (index, part) in parts.iter().enumerate() {
            let failing = part.contains("fail");
            let words = format!(
                "{}{}{}",
                if index == 0 { "checks: " } else { "" },
                part,
                if index < last { "," } else { "" }
            );
            line = line
                .child(chrome::icon(
                    if failing { IconName::CircleX } else { IconName::Check },
                    if failing { theme::RED_STATUS } else { theme::GREEN },
                    12.,
                ))
                .child(div().text_color(rgb(if failing { theme::RED_STATUS } else { look.secondary })).child(words));
        }
        card = card.child(line);
    }
    card
}

/// A card: rounded, hairline border, on the card colour.
fn card(look: Look) -> Div {
    col()
        .rounded(px(9.))
        .bg(rgb(look.card))
        .border_1()
        .border_color(rgb(look.line))
}

/// Quiet text.
fn secondary(look: Look, text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(11.5))
        .text_color(rgb(look.secondary))
        .child(text.into())
}

/// A key on the left, right-aligned in 80 pt and centred on its value (frame
/// 67), and the value filling the rest.
fn form_row(look: Look, key: &'static str, value: impl IntoElement) -> Div {
    row()
        .items_center()
        .gap(px(12.))
        .child(
            div()
                .w(px(80.))
                .flex()
                .flex_shrink_0()
                .justify_end()
                .text_color(rgb(look.secondary))
                .child(key),
        )
        .child(div().flex_1().min_w_0().child(value))
}

/// A small key cap.
fn kbd(look: Look, key: &'static str) -> Div {
    div()
        .px(px(5.))
        .rounded(px(4.))
        .bg(rgb(look.fill))
        .text_size(px(11.))
        .text_color(rgb(look.secondary))
        .flex_shrink_0()
        .child(key)
}

/// A line that says what is wrong and how to fix it.
fn alert(id: impl Into<ElementId>, text: String) -> impl IntoElement {
    div()
        .id(id)
        .role(Role::Alert)
        .aria_label(text.clone())
        .test_support()
        .text_color(rgb(theme::RED_STATUS))
        .child(text)
}

/// A check box with its words.
fn check(
    ui: &Rc<Ui>,
    look: Look,
    id: &'static str,
    text: &'static str,
    on: bool,
    action: AppAction,
) -> impl IntoElement {
    let ui = ui.clone();
    row()
        .id(id)
        .role(Role::CheckBox)
        .aria_label(text)
        .aria_selected(on)
        .test_support()
        .gap(px(7.))
        .cursor_pointer()
        .child(
            row()
                .size(px(14.))
                .flex_shrink_0()
                .rounded(px(4.))
                .justify_center()
                .when(on, |x| {
                    x.bg(rgb(look.accent))
                        .child(chrome::icon(IconName::Check, 0xffffff, 10.))
                })
                .when(!on, |x| {
                    x.border_1()
                        .border_color(rgb(look.tertiary))
                        .bg(rgb(look.card))
                }),
        )
        .child(div().min_w_0().child(text))
        .on_click(move |_, _, cx| ui.send(action.clone(), cx))
}
