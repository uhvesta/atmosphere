//! Settings › Browser (BROWSER, frame `b1-settings-browser`):
//! which agents may use your browser, the table of detected browsers on this
//! Mac with Host & Extension status and one-click Set up host / Update host /
//! Uninstall / Install all, how to load the unpacked extension, and what you
//! are asked.

use crate::{
    bridge::Ui,
    chrome,
    settings::{button, card, dim},
    theme::{GREEN, GREY, Look, ORANGE},
    widgets::{col, row},
};
use gpui_kit::assets::IconName;
use gpui_kit::base::Disableable as _;
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState};
use humata2_clients::model::{BrowserFamily, DetectedBrowser};
use humata2_settings::{BrowserAction, SettingsAction as A};
use std::rc::Rc;

const DEFAULT_CHROMIUM_EXTENSION_PATH: &str = "apps/humata2/browser_extension/chromium";
const DEFAULT_FIREFOX_EXTENSION_PATH: &str = "apps/humata2/browser_extension/firefox";

fn send(ui: &Rc<Ui>, action: BrowserAction, cx: &mut App) {
    ui.send(AppAction::Settings(A::Browser(action)), cx);
}

fn action_link(
    ui: &Rc<Ui>,
    id: &'static str,
    label: &'static str,
    action: BrowserAction,
    look: Look,
) -> impl IntoElement {
    let ui = ui.clone();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .test_support()
        .cursor_pointer()
        .text_size(px(12.))
        .text_color(rgb(look.accent))
        .child(label)
        .on_click(move |_, _, cx| send(&ui, action.clone(), cx))
}

/// Formats Chromium browser display names for the extension card title, e.g.
/// `"Chrome, Brave and Edge"`.
fn chromium_title(browsers: &[DetectedBrowser]) -> String {
    let names: Vec<&str> = browsers
        .iter()
        .filter(|b| b.family == BrowserFamily::Chromium)
        .map(|b| b.display_name.as_str())
        .collect();
    match names.as_slice() {
        [] => "Chrome, Brave, Edge, Arc, Vivaldi and Chromium".to_owned(),
        [one] => (*one).to_owned(),
        [head @ .., last] => format!("{} and {last}", head.join(", ")),
    }
}

/// Resolves the extension folder path for `family` from the detected browsers,
/// falling back to the repo's default unpacked extension path.
fn extension_path_for(browsers: &[DetectedBrowser], family: BrowserFamily) -> String {
    browsers
        .iter()
        .find(|b| b.family == family && !b.extension_path.is_empty())
        .map(|b| b.extension_path.clone())
        .unwrap_or_else(|| match family {
            BrowserFamily::Chromium => DEFAULT_CHROMIUM_EXTENSION_PATH.to_owned(),
            BrowserFamily::Firefox => DEFAULT_FIREFOX_EXTENSION_PATH.to_owned(),
        })
}

fn status_dot(color: u32) -> Div {
    div().size(px(8.)).rounded_full().flex_shrink_0().bg(rgb(color))
}

fn browser_row(
    ui: &Rc<Ui>,
    browser: &DetectedBrowser,
    busy: bool,
    look: Look,
) -> impl IntoElement {
    let slug = browser.kind.slug();
    let kind = browser.kind;

    // Host column: Set up / Out of date (with second line) / Not set up.
    let host = if browser.host_installed && browser.host_matches_current {
        col().child(
            row()
                .gap(px(6.))
                .items_center()
                .child(chrome::icon(IconName::Check, GREEN, 12.))
                .child("Set up"),
        )
    } else if browser.host_installed {
        col()
            .gap(px(1.))
            .child(
                row()
                    .gap(px(6.))
                    .items_center()
                    .text_color(rgb(ORANGE))
                    .child(
                        div()
                            .w(px(12.))
                            .font_weight(FontWeight::BOLD)
                            .text_center()
                            .child("!"),
                    )
                    .child("Out of date"),
            )
            .child(dim(
                look,
                format!(
                    "Its file points at an older Humata, so {} can\u{2019}t connect.",
                    browser.display_name
                ),
            ))
    } else {
        col().child(
            div()
                .text_color(rgb(look.secondary))
                .child("Not set up"),
        )
    };

    // Extension column: only shown when host is installed and current.
    let extension = if browser.host_installed && browser.host_matches_current {
        if browser.connected {
            col().child(
                row()
                    .gap(px(6.))
                    .items_center()
                    .child(status_dot(GREEN))
                    .child("Connected"),
            )
        } else {
            col()
                .gap(px(1.))
                .child(
                    row()
                        .gap(px(6.))
                        .items_center()
                        .child(status_dot(GREY))
                        .child("Not connected"),
                )
                .child(dim(
                    look,
                    format!("Load the extension in {}, below.", browser.display_name),
                ))
        }
    } else {
        col()
    };

    // Action buttons for this row.
    let mut actions = row().gap(px(6.)).flex_shrink_0();
    if browser.host_installed && browser.host_matches_current {
        actions = actions.child(
            button(
                ui,
                SharedString::from(format!("settings-browser-uninstall-{slug}")),
                "Uninstall",
                AppAction::Settings(A::Browser(BrowserAction::Uninstall(kind))),
                look,
            )
            .disabled(busy),
        );
    } else if browser.host_installed {
        actions = actions
            .child(
                button(
                    ui,
                    SharedString::from(format!("settings-browser-install-{slug}")),
                    "Update host",
                    AppAction::Settings(A::Browser(BrowserAction::Install(kind))),
                    look,
                )
                .bg(rgb(look.accent))
                .text_color(rgb(0xffffff))
                .disabled(busy),
            )
            .child(
                button(
                    ui,
                    SharedString::from(format!("settings-browser-uninstall-{slug}")),
                    "Uninstall",
                    AppAction::Settings(A::Browser(BrowserAction::Uninstall(kind))),
                    look,
                )
                .disabled(busy),
            );
    } else {
        actions = actions.child(
            button(
                ui,
                SharedString::from(format!("settings-browser-install-{slug}")),
                "Set up host",
                AppAction::Settings(A::Browser(BrowserAction::Install(kind))),
                look,
            )
            .bg(rgb(look.accent))
            .text_color(rgb(0xffffff))
            .disabled(busy),
        );
    }

    row()
        .id(SharedString::from(format!("settings-browser-row-{slug}")))
        .test_support()
        .min_h(px(40.))
        .px(px(14.))
        .py(px(8.))
        .items_center()
        .border_b_1()
        .border_color(rgb(look.line))
        .child(
            div()
                .w(px(120.))
                .flex_shrink_0()
                .child(browser.display_name.clone()),
        )
        .child(div().w(px(280.)).flex_shrink_0().pr(px(16.)).child(host))
        .child(div().flex_1().min_w_0().pr(px(12.)).child(extension))
        .child(actions)
}

fn extension_row(
    ui: &Rc<Ui>,
    title: String,
    instructions: &'static str,
    path: String,
    copy_id: &'static str,
    reveal_id: &'static str,
    look: Look,
) -> Div {
    row()
        .gap(px(16.))
        .px(px(14.))
        .py(px(10.))
        .items_start()
        .border_b_1()
        .border_color(rgb(look.line))
        .child(
            col()
                .flex_1()
                .min_w_0()
                .gap(px(2.))
                .child(title)
                .child(dim(look, instructions))
                .child(
                    div()
                        .mt(px(2.))
                        .font_family("Menlo")
                        .text_size(px(11.5))
                        .child(path.clone()),
                ),
        )
        .child(
            row()
                .gap(px(14.))
                .flex_shrink_0()
                .pt(px(1.))
                .child(action_link(
                    ui,
                    copy_id,
                    "Copy path",
                    BrowserAction::CopyExtensionPath(path.clone()),
                    look,
                ))
                .child(action_link(
                    ui,
                    reveal_id,
                    "Reveal in Finder",
                    BrowserAction::RevealExtensionPath(path),
                    look,
                )),
        )
}

/// Settings › Browser body (frame `b1-settings-browser`).
pub(crate) fn body(ui: &Rc<Ui>, state: &AppState, look: Look) -> Div {
    let busy = state.settings.browser.busy;
    let browsers = state.settings.browser.browsers.as_deref();
    let has_browsers = browsers.is_some_and(|list| !list.is_empty());

    let mut page = col()
        .id("settings-browser-page")
        .test_support()
        .gap(px(8.))
        .child(dim(look, "Browser").font_weight(FontWeight::SEMIBOLD))
        .child(dim(
            look,
            "Claude Code, Codex, Copilot and agy can read tabs in your own browsers and, when you say yes, click, type and open pages in them. No other agent can.",
        ));

    // Browsers on this Mac + Install all button.
    let table_header = row()
        .mt(px(10.))
        .items_center()
        .child(
            dim(look, "Browsers on this Mac")
                .font_weight(FontWeight::SEMIBOLD)
                .flex_1(),
        )
        .child(
            button(
                ui,
                "settings-browser-install-all",
                "Install all",
                AppAction::Settings(A::Browser(BrowserAction::InstallAll)),
                look,
            )
            .disabled(busy || !has_browsers),
        );
    page = page.child(table_header);

    let mut table = card(look).child(
        row()
            .px(px(14.))
            .py(px(6.))
            .items_center()
            .border_b_1()
            .border_color(rgb(look.line))
            .child(div().w(px(120.)).flex_shrink_0().child(dim(look, "Browser")))
            .child(div().w(px(280.)).flex_shrink_0().child(dim(look, "Host")))
            .child(div().flex_1().child(dim(look, "Extension"))),
    );

    match browsers {
        None => {
            table = table.child(
                div()
                    .px(px(14.))
                    .py(px(10.))
                    .child(dim(look, "Loading browsers\u{2026}")),
            );
        }
        Some([]) => {
            table = table.child(
                div()
                    .px(px(14.))
                    .py(px(10.))
                    .child(dim(look, "No supported browsers found on this Mac.")),
            );
        }
        Some(list) => {
            for browser in list {
                table = table.child(browser_row(ui, browser, busy, look));
            }
        }
    }

    page = page.child(table).child(dim(
        look,
        "Set up host writes one file into that browser\u{2019}s own folder so it can find Humata. Uninstall removes that file and nothing else.",
    ));

    if let Some(error) = &state.settings.error {
        page = page.child(
            div()
                .id("settings-browser-error")
                .role(Role::Alert)
                .aria_label(error.clone())
                .text_size(px(11.5))
                .text_color(rgb(0xff453a))
                .child(error.clone())
                .test_support(),
        );
    }

    // Load the extension.
    let list = browsers.unwrap_or(&[]);
    let chromium_path = extension_path_for(list, BrowserFamily::Chromium);
    let firefox_path = extension_path_for(list, BrowserFamily::Firefox);
    let extension_card = card(look)
        .child(extension_row(
            ui,
            chromium_title(list),
            "Open the browser\u{2019}s Extensions page, turn on Developer mode, choose Load unpacked and pick this folder.",
            chromium_path,
            "settings-browser-copy-chromium",
            "settings-browser-reveal-chromium",
            look,
        ))
        .child(extension_row(
            ui,
            "Firefox".to_owned(),
            "Open about:debugging, choose This Firefox, then Load Temporary Add-on, and pick manifest.json in this folder. Firefox forgets it when it quits, so load it again after a restart.",
            firefox_path,
            "settings-browser-copy-firefox",
            "settings-browser-reveal-firefox",
            look,
        ));
    page = page
        .child(
            dim(look, "Load the extension")
                .font_weight(FontWeight::SEMIBOLD)
                .mt(px(12.)),
        )
        .child(extension_card);

    // What you are asked.
    let asked_card = card(look)
        .child(
            row()
                .px(px(14.))
                .py(px(10.))
                .gap(px(16.))
                .border_b_1()
                .border_color(rgb(look.line))
                .child(div().flex_1().child("Read a tab or list your tabs"))
                .child(
                    div()
                        .text_color(rgb(look.secondary))
                        .child("Asked in the Agent inbox \u{b7} you can allow reading for 30 min"),
                ),
        )
        .child(
            row()
                .px(px(14.))
                .py(px(10.))
                .gap(px(16.))
                .child(div().flex_1().child("Click, type or open a page"))
                .child(
                    div()
                        .text_color(rgb(look.secondary))
                        .child("Asked every time \u{b7} can\u{2019}t be allowed ahead"),
                ),
        );
    div().child(
        page.child(
            dim(look, "What you are asked")
                .font_weight(FontWeight::SEMIBOLD)
                .mt(px(12.)),
        )
        .child(asked_card),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use humata2_clients::model::BrowserKind;

    fn sample(kind: BrowserKind, family: BrowserFamily, name: &str, ext: &str) -> DetectedBrowser {
        DetectedBrowser {
            kind,
            family,
            display_name: name.to_owned(),
            binary_path: String::new(),
            native_messaging_dir: String::new(),
            manifest_path: String::new(),
            host_installed: false,
            host_matches_current: false,
            connected: false,
            extension_path: ext.to_owned(),
        }
    }

    #[core::prelude::v1::test]
    fn chromium_title_joins_detected_chromium_browsers() {
        assert_eq!(
            chromium_title(&[]),
            "Chrome, Brave, Edge, Arc, Vivaldi and Chromium"
        );
        assert_eq!(
            chromium_title(&[sample(BrowserKind::Chrome, BrowserFamily::Chromium, "Chrome", "")]),
            "Chrome"
        );
        assert_eq!(
            chromium_title(&[
                sample(BrowserKind::Chrome, BrowserFamily::Chromium, "Chrome", ""),
                sample(BrowserKind::Brave, BrowserFamily::Chromium, "Brave", ""),
                sample(BrowserKind::Edge, BrowserFamily::Chromium, "Edge", ""),
                sample(BrowserKind::Firefox, BrowserFamily::Firefox, "Firefox", ""),
            ]),
            "Chrome, Brave and Edge"
        );
    }

    #[core::prelude::v1::test]
    fn extension_path_prefers_daemon_path_and_falls_back_to_default() {
        let browsers = [
            sample(
                BrowserKind::Chrome,
                BrowserFamily::Chromium,
                "Chrome",
                "/opt/humata/ext/chromium",
            ),
            sample(BrowserKind::Firefox, BrowserFamily::Firefox, "Firefox", ""),
        ];
        assert_eq!(
            extension_path_for(&browsers, BrowserFamily::Chromium),
            "/opt/humata/ext/chromium"
        );
        assert_eq!(
            extension_path_for(&browsers, BrowserFamily::Firefox),
            DEFAULT_FIREFOX_EXTENSION_PATH
        );
    }
}
