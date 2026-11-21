//! Settings › Machines › Providers (PROVIDERS): the commands Humata runs on
//! this Mac to make, start, stop and delete machines. One row per provider;
//! Commands opens a row in place, one at a time, with every command
//! verbatim, its params and the host variables it reads; Remove asks first,
//! or says which machines still use it; Import provider… reviews a pasted
//! provider file before anything is added.
//!
//! A host variable's secure field is emptied once its value is sent; the
//! value is never drawn, kept in the app state, logged or recorded.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui_kit::base::{Disableable as _, ObservedElement};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState};
use humata2_clients::machines::{ProviderEntry, ProviderImportPlan};
use humata2_settings::{ImportDraft, ProvidersAction as P, ProvidersState, Secret, SettingsAction as A};

use crate::settings::{button, card, dim};
use crate::bridge::Ui;
use crate::theme::Look;
use crate::widgets::{self, col, row};

const RED: u32 = 0xff453a;

fn send(action: P) -> AppAction {
    AppAction::Settings(A::Providers(action))
}

/// The fields the block types into: the paste field and one secure field
/// per host variable, made the first time each is drawn. A field set from
/// the state remembers the text it was set to, so the change it reports
/// back isn't taken for typing.
pub struct ProvidersInputs {
    ui: Rc<Ui>,
    paste: Entity<TextareaState>,
    paste_echo: Rc<RefCell<Option<String>>>,
    variables: RefCell<HashMap<String, Entity<InputState>>>,
    add_repo: RefCell<Option<Entity<InputState>>>,
    add_repo_echo: Rc<RefCell<Option<String>>>,
    text_fields: RefCell<HashMap<String, (Entity<InputState>, Rc<RefCell<Option<String>>>)>>,
    subscriptions: RefCell<Vec<Subscription>>,
}

impl ProvidersInputs {
    pub fn new(ui: &Rc<Ui>, window: &mut Window, cx: &mut App) -> Self {
        let paste = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Paste the provider's TOML")
                .auto_grow(4, 16)
        });
        let paste_echo = Rc::new(RefCell::new(None::<String>));
        let (typing_ui, echo) = (ui.clone(), paste_echo.clone());
        let typing = cx.subscribe(
            &paste,
            move |input: Entity<TextareaState>, event: &InputEvent, cx: &mut App| {
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                let text = input.read(cx).value().to_string();
                if echo.borrow().as_deref() == Some(text.as_str()) {
                    *echo.borrow_mut() = None;
                    return;
                }
                typing_ui.send(send(P::ImportText(text)), cx);
            },
        );
        Self {
            ui: ui.clone(),
            paste,
            paste_echo,
            variables: RefCell::new(HashMap::new()),
            add_repo: RefCell::new(None),
            add_repo_echo: Rc::new(RefCell::new(None)),
            text_fields: RefCell::new(HashMap::new()),
            subscriptions: RefCell::new(vec![typing]),
        }
    }

    /// The inline `owner/name` field for Settings › General › Repositories › "+ Add repo".
    pub(crate) fn add_repo_input(
        &self,
        text: &str,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<InputState> {
        let field = if let Some(field) = self.add_repo.borrow().as_ref() {
            field.clone()
        } else {
            let field = cx.new(|cx| InputState::new(window, cx).placeholder("owner/name"));
            let (ui, echo) = (self.ui.clone(), self.add_repo_echo.clone());
            self.subscriptions.borrow_mut().push(cx.subscribe(
                &field,
                move |input: Entity<InputState>, event: &InputEvent, cx: &mut App| match event {
                    InputEvent::Change => {
                        let value = input.read(cx).value().to_string();
                        if echo.borrow().as_deref() == Some(value.as_str()) {
                            *echo.borrow_mut() = None;
                            return;
                        }
                        ui.send(
                            AppAction::Settings(A::Repos(
                                humata2_settings::RepositoriesAction::AddRepoChanged(value),
                            )),
                            cx,
                        );
                    }
                    InputEvent::PressEnter { .. } => ui.send(
                        AppAction::Settings(A::Repos(
                            humata2_settings::RepositoriesAction::AddRepoSubmitted,
                        )),
                        cx,
                    ),
                    InputEvent::Focus | InputEvent::Blur => {}
                },
            ));
            *self.add_repo.borrow_mut() = Some(field.clone());
            field
        };
        if field.read(cx).value().as_ref() != text {
            *self.add_repo_echo.borrow_mut() = Some(text.to_owned());
            let owned = text.to_owned();
            field.update(cx, |input, cx| input.set_value(owned, window, cx));
        }
        field
    }

    /// A keyed single-line text input for Settings and Channel Settings forms.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn text_input(
        &self,
        key: &str,
        placeholder: &'static str,
        text: &str,
        on_change: impl Fn(String) -> AppAction + 'static,
        on_enter: Option<AppAction>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<InputState> {
        let (field, echo) = if let Some(existing) = self.text_fields.borrow().get(key) {
            (existing.0.clone(), existing.1.clone())
        } else {
            let field = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
            let echo = Rc::new(RefCell::new(None::<String>));
            let (ui, echo_cb) = (self.ui.clone(), echo.clone());
            self.subscriptions.borrow_mut().push(cx.subscribe(
                &field,
                move |input: Entity<InputState>, event: &InputEvent, cx: &mut App| match event {
                    InputEvent::Change => {
                        let value = input.read(cx).value().to_string();
                        if echo_cb.borrow().as_deref() == Some(value.as_str()) {
                            *echo_cb.borrow_mut() = None;
                            return;
                        }
                        ui.send(on_change(value), cx);
                    }
                    InputEvent::PressEnter { .. } => {
                        if let Some(action) = &on_enter {
                            ui.send(action.clone(), cx);
                        }
                    }
                    InputEvent::Focus | InputEvent::Blur => {}
                },
            ));
            self.text_fields
                .borrow_mut()
                .insert(key.to_owned(), (field.clone(), echo.clone()));
            (field, echo)
        };
        if field.read(cx).value().as_ref() != text {
            *echo.borrow_mut() = Some(text.to_owned());
            let owned = text.to_owned();
            field.update(cx, |input, cx| input.set_value(owned, window, cx));
        }
        field
    }

    /// The secure field for host variable `name`.
    fn variable(&self, name: &str, window: &mut Window, cx: &mut App) -> Entity<InputState> {
        if let Some(field) = self.variables.borrow().get(name) {
            return field.clone();
        }
        let field = cx.new(|cx| InputState::new(window, cx).placeholder("Value").masked(true));
        let (ui, for_name) = (self.ui.clone(), name.to_owned());
        self.subscriptions.borrow_mut().push(cx.subscribe(
            &field,
            move |input: Entity<InputState>, event: &InputEvent, cx: &mut App| match event {
                InputEvent::Change => {
                    let text = input.read(cx).value().to_string();
                    ui.send(send(P::VariableValue(for_name.clone(), Secret::new(text))), cx);
                }
                InputEvent::PressEnter { .. } => ui.send(send(P::SetVariable(for_name.clone())), cx),
                InputEvent::Focus | InputEvent::Blur => {}
            },
        ));
        self.variables.borrow_mut().insert(name.to_owned(), field.clone());
        field
    }

    /// Brings the fields to `state`: the paste field to the draft, and each
    /// secure field empty once its value has left the state (sent, or the
    /// row closed).
    fn sync(&self, state: &ProvidersState, window: &mut Window, cx: &mut App) {
        let text = state.import.as_ref().map_or("", |draft| draft.text.as_str());
        if self.paste.read(cx).value().as_ref() != text {
            *self.paste_echo.borrow_mut() = Some(text.to_owned());
            let text = text.to_owned();
            self.paste.update(cx, |input, cx| input.set_value(text, window, cx));
        }
        for (name, field) in self.variables.borrow().iter() {
            // Emptying reports an empty value, which the state drops anyway.
            if !state.values.contains_key(name) && !field.read(cx).value().is_empty() {
                field.update(cx, |input, cx| input.set_value(String::new(), window, cx));
            }
        }
    }
}

fn alert(id: impl Into<ElementId>, text: &str) -> ObservedElement<Stateful<Div>> {
    div()
        .id(id)
        .role(Role::Alert)
        .aria_label(text.to_owned())
        .text_size(px(11.5))
        .text_color(rgb(RED))
        .child(text.to_owned())
        .test_support()
}

fn mono(look: Look, text: &str) -> Div {
    div()
        .font_family("Menlo")
        .text_size(px(11.5))
        .text_color(rgb(look.text))
        .child(text.to_owned())
}

/// `a`, `a and b`, `a, b and c`.
fn and_list(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// `id · Built in · create, delete · 2 machines · Needs HCLOUD_TOKEN`.
fn note(provider: &ProviderEntry) -> String {
    let mut parts = vec![
        provider.id.clone(),
        if provider.built_in { "Built in" } else { "Imported" }.to_owned(),
    ];
    if !provider.actions.is_empty() {
        parts.push(provider.actions.join(", "));
    }
    match provider.machines.len() {
        0 => {}
        1 => parts.push("1 machine".to_owned()),
        n => parts.push(format!("{n} machines")),
    }
    let unset: Vec<&str> = provider.unset().collect();
    if !unset.is_empty() {
        parts.push(format!("Needs {}", unset.join(", ")));
    }
    parts.join(" \u{b7} ")
}

/// Every command `provider` runs, verbatim, read-only.
fn commands(look: Look, provider: &ProviderEntry) -> Div {
    let mut block = col().gap(px(6.));
    if provider.commands.is_empty() {
        return block.child(dim(look, "Humata doesn't list this provider's commands yet."));
    }
    for command in &provider.commands {
        block = block.child(
            col()
                .gap(px(2.))
                .child(dim(look, command.action.clone()))
                .child(
                    div()
                        .px(px(8.))
                        .py(px(6.))
                        .rounded(px(6.))
                        .bg(rgb(look.fill))
                        .child(mono(look, &command.command)),
                ),
        );
    }
    block
}

fn params(look: Look, provider: &ProviderEntry) -> Option<Div> {
    if provider.params.is_empty() {
        return None;
    }
    let mut block = col().gap(px(2.)).child(dim(look, "Asks for"));
    for param in &provider.params {
        let default = if param.default.is_empty() {
            "no default".to_owned()
        } else {
            format!("default {}", param.default)
        };
        block = block.child(mono(look, &format!("{} \u{b7} {default}", param.name)));
    }
    Some(block)
}

fn variables(
    ui: &Rc<Ui>,
    state: &ProvidersState,
    inputs: &ProvidersInputs,
    provider: &ProviderEntry,
    look: Look,
    window: &mut Window,
    cx: &mut App,
) -> Option<Div> {
    if provider.variables.is_empty() {
        return None;
    }
    let mut block = col()
        .gap(px(6.))
        .child(dim(look, "Host variables on this Mac. Values are sent once and never shown."));
    for variable in &provider.variables {
        let name = &variable.name;
        let typed = state.values.get(name).is_some_and(|value| !value.is_empty());
        let field = inputs.variable(name, window, cx);
        block = block.child(
            row()
                .id(SharedString::from(format!("settings-provider-variable {name}")))
                .role(Role::Group)
                .aria_label(name.clone())
                .gap(px(8.))
                .child(mono(look, name).w(px(150.)).flex_shrink_0())
                .child(dim(look, if variable.set { "Set" } else { "Not set" }).w(px(52.)).flex_shrink_0())
                .child(div().flex_1().min_w_0().child(Input::new(&field).aria_label(format!("Value for {name}"))))
                .child(
                    button(
                        ui,
                        SharedString::from(format!("settings-provider-set {name}")),
                        if variable.set { "Replace" } else { "Set" },
                        send(P::SetVariable(name.clone())),
                        look,
                    )
                    .disabled(!typed),
                )
                .when(variable.set, |x| {
                    x.child(button(
                        ui,
                        SharedString::from(format!("settings-provider-clear {name}")),
                        "Clear",
                        send(P::ClearVariable(name.clone())),
                        look,
                    ))
                })
                .test_support(),
        );
    }
    Some(block)
}

/// Remove's in-place confirm, or why it can't remove yet.
fn removing(ui: &Rc<Ui>, provider: &ProviderEntry, look: Look) -> Div {
    let cancel = button(ui, "settings-provider-remove-cancel", "Cancel", send(P::CancelRemove), look);
    if provider.machines.is_empty() {
        return row()
            .gap(px(8.))
            .child(div().flex_1().min_w_0().child(dim(
                look,
                format!("Remove {}? Humata can't make, start or stop machines with it after this.", provider.label),
            )))
            .child(cancel)
            .child(
                widgets::primary_in(
                    look,
                    ui,
                    "settings-provider-remove-confirm",
                    "Remove",
                    send(P::ConfirmRemove(provider.id.clone())),
                )
                .bg(rgb(RED)),
            );
    }
    row()
        .gap(px(8.))
        .child(div().flex_1().min_w_0().child(alert(
            "settings-provider-remove-blocked",
            &format!(
                "{} still runs {}. Delete those machines first.",
                provider.label,
                and_list(&provider.machines)
            ),
        )))
        .child(cancel)
}

fn provider_row(
    ui: &Rc<Ui>,
    state: &ProvidersState,
    inputs: &ProvidersInputs,
    provider: &ProviderEntry,
    look: Look,
    window: &mut Window,
    cx: &mut App,
) -> Stateful<Div> {
    let id = &provider.id;
    let open = state.open.as_ref() == Some(id);
    let mut item = col()
        .id(SharedString::from(format!("settings-provider {id}")))
        .role(Role::Group)
        .aria_label(provider.label.clone())
        .border_b_1()
        .border_color(rgb(look.line))
        .child(
            row()
                .gap(px(16.))
                .px(px(14.))
                .py(px(10.))
                .child(col().flex_1().min_w_0().child(provider.label.clone()).child(dim(look, note(provider))))
                .child(
                    row()
                        .gap(px(8.))
                        .child(button(
                            ui,
                            SharedString::from(format!("settings-provider-commands {id}")),
                            "Commands",
                            send(P::ToggleCommands(id.clone())),
                            look,
                        ))
                        .when(!provider.built_in, |x| {
                            x.child(button(
                                ui,
                                SharedString::from(format!("settings-provider-remove {id}")),
                                "Remove",
                                send(P::Remove(id.clone())),
                                look,
                            ))
                        }),
                ),
        );
    if state.removing.as_ref() == Some(id) {
        item = item.child(div().px(px(14.)).pb(px(10.)).child(removing(ui, provider, look)));
    }
    if open {
        let mut details = col()
            .px(px(14.))
            .pb(px(12.))
            .gap(px(10.))
            .child(commands(look, provider));
        if let Some(params) = params(look, provider) {
            details = details.child(params);
        }
        if let Some(variables) = variables(ui, state, inputs, provider, look, window, cx) {
            details = details.child(variables);
        }
        item = item.child(details);
    }
    item
}

/// One provider in an import review: what it will run and what it reads.
fn reviewed(look: Look, state: &ProvidersState, plan: &ProviderImportPlan, provider: &ProviderEntry) -> Div {
    let mut block = col()
        .gap(px(6.))
        .px(px(14.))
        .py(px(10.))
        .border_b_1()
        .border_color(rgb(look.line))
        .child(provider.label.clone())
        .child(dim(look, note(provider)));
    if plan.replaces.contains(&provider.id) {
        let existing = state.provider(&provider.id).map_or(provider.id.as_str(), |p| p.label.as_str());
        block = block.child(dim(look, format!("Replaces {existing}")));
    }
    block = block.child(commands(look, provider));
    if !provider.variables.is_empty() {
        let names: Vec<&str> = provider.variables.iter().map(|v| v.name.as_str()).collect();
        block = block.child(dim(look, format!("Reads {}", names.join(", "))));
    }
    block
}

fn import_block(
    ui: &Rc<Ui>,
    state: &ProvidersState,
    draft: &ImportDraft,
    inputs: &ProvidersInputs,
    look: Look,
) -> Stateful<Div> {
    let mut block = col()
        .id("settings-provider-import")
        .role(Role::Group)
        .aria_label("Import provider")
        .gap(px(8.))
        .child(dim(look, "Paste the provider's TOML"))
        .child(
            div()
                .px(px(4.))
                .py(px(6.))
                .rounded(px(6.))
                .bg(rgb(look.card))
                .border_1()
                .border_color(rgb(look.line))
                .font_family("Menlo")
                .child(
                    Textarea::new(&inputs.paste)
                        .appearance(false)
                        .bordered(false)
                        .text_size(px(11.5))
                        .aria_label("Provider TOML"),
                ),
        );
    if let Some(problem) = &draft.problem {
        block = block.child(alert("settings-provider-import-problem", problem));
    }
    let cancel = button(ui, "settings-provider-import-cancel", "Cancel", send(P::CancelImport), look).disabled(draft.busy);
    match &draft.review {
        Some(plan) => {
            let mut review = card(look);
            for provider in &plan.providers {
                review = review.child(reviewed(look, state, plan, provider));
            }
            block.child(review).child(
                row()
                    .gap(px(8.))
                    .child(
                        widgets::primary_in(
                            look,
                            ui,
                            "settings-provider-import-confirm",
                            if draft.busy { "Importing…" } else { "Import" },
                            send(P::Import),
                        )
                        .disabled(draft.busy),
                    )
                    .child(cancel),
            )
        }
        None => block.child(
            row()
                .gap(px(8.))
                .child(
                    button(
                        ui,
                        "settings-provider-import-review",
                        if draft.busy { "Reviewing…" } else { "Review" },
                        send(P::Review),
                        look,
                    )
                    .disabled(draft.busy || draft.text.trim().is_empty()),
                )
                .child(cancel),
        ),
    }
}

/// The Providers block, above Scripts in Settings › Machines.
pub(super) fn providers_block(
    ui: &Rc<Ui>,
    state: &AppState,
    inputs: &ProvidersInputs,
    look: Look,
    window: &mut Window,
    cx: &mut App,
) -> Div {
    let providers = &state.settings.providers;
    inputs.sync(providers, window, cx);
    let mut block = col()
        .gap(px(8.))
        .child(div().font_weight(FontWeight::SEMIBOLD).child("Providers"))
        .child(dim(
            look,
            "Commands Humata runs on this Mac to make, start, stop and delete machines. An imported provider runs with your account's access, so read its commands before you add it.",
        ));
    if let Some(error) = &providers.error {
        block = block.child(alert("settings-providers-error", error));
    }
    match &providers.list {
        None => block = block.child(dim(look, "Loading providers")),
        Some(list) => {
            let mut rows = card(look);
            for provider in list {
                rows = rows.child(provider_row(ui, providers, inputs, provider, look, window, cx));
            }
            block = block.child(rows);
        }
    }
    match &providers.import {
        Some(draft) => block.child(import_block(ui, providers, draft, inputs, look)),
        None => block.child(row().child(button(
            ui,
            "settings-provider-import-open",
            "Import provider…",
            send(P::OpenImport),
            look,
        ))),
    }
}
