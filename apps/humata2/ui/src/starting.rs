//! The small window shown while the app starts: what it is doing, or why it
//! couldn't start, with Quit. It is drawn with the app's look, in the light
//! or dark the person chose last time.

use std::rc::Rc;

use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::{prelude::*, *};
use tca::Storage;

use crate::theme::{Look, RED};
use crate::widgets::{self, row};

/// The starting window's size.
pub const STARTING_WINDOW: (f32, f32) = (440., 220.);

/// The starting window's view. The caller says what is happening with
/// [`Starting::set_status`], or that start-up failed with [`Starting::fail`].
pub struct Starting {
    theme: humata2_settings::Theme,
    status: SharedString,
    failure: Option<SharedString>,
    on_retry: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
    _appearance: Subscription,
}

/// The theme saved by the last run; the system's when nothing readable was
/// saved, since the app itself then starts that way.
fn saved_theme(storage: &dyn Storage) -> humata2_settings::Theme {
    match humata2_app::saved::restore(storage) {
        Ok(Some(saved)) => saved.preferences.theme,
        Ok(None) | Err(_) => humata2_settings::Theme::System,
    }
}

fn apply(theme: humata2_settings::Theme, window: &mut Window, cx: &mut App) {
    match theme {
        humata2_settings::Theme::System => Theme::sync_system_appearance(Some(window), cx),
        humata2_settings::Theme::Light => Theme::change(ThemeMode::Light, Some(window), cx),
        humata2_settings::Theme::Dark => Theme::change(ThemeMode::Dark, Some(window), cx),
    }
    humata2_ui_core::sync_theme(cx);
}

impl Starting {
    /// The view for `window`, showing `status`, in the theme `storage` saved.
    pub fn new(
        storage: &dyn Storage,
        status: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let theme = saved_theme(storage);
        apply(theme, window, cx);
        let appearance = cx.observe_window_appearance(window, |this, window, cx| {
            if this.theme == humata2_settings::Theme::System {
                Theme::sync_system_appearance(Some(window), cx);
                humata2_ui_core::sync_theme(cx);
            }
        });
        Self {
            theme,
            status: status.into(),
            failure: None,
            on_retry: None,
            _appearance: appearance,
        }
    }

    /// Registers a callback invoked when the user clicks **Retry** after a
    /// startup failure.
    pub fn set_on_retry(&mut self, retry: impl Fn(&mut Window, &mut App) + 'static) {
        self.on_retry = Some(Rc::new(retry));
    }

    /// Shows `status` under "Starting Humata…".
    pub fn set_status(&mut self, status: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.status = status.into();
        cx.notify();
    }

    /// Shows why start-up failed, with Retry, Dismiss, and Quit, in place of the status.
    pub fn fail(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.failure = Some(text.into());
        cx.notify();
    }

    /// Clears the failure banner and returns to the starting status view.
    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.failure = None;
        cx.notify();
    }

    /// Clears the failure banner and invokes the registered retry handler, if any.
    pub fn retry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.failure = None;
        let handler = self.on_retry.clone();
        cx.notify();
        if let Some(handler) = handler {
            handler(window, cx);
        }
    }
}

impl Render for Starting {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let look = Look::of(cx);
        let page = div()
            .flex()
            .flex_col()
            .size_full()
            .items_center()
            .justify_center()
            .gap(px(8.))
            .px(px(24.))
            .bg(rgb(look.content))
            .text_color(rgb(look.text))
            .font_family(".SystemUIFont")
            .child(
                div()
                    .text_size(px(20.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Humata 2"),
            );
        match &self.failure {
            None => page
                .child(div().text_size(px(13.)).child("Starting Humata…"))
                .child(
                    div()
                        .id("starting-status")
                        .role(Role::Status)
                        .aria_label(self.status.clone())
                        .text_size(px(12.))
                        .text_color(rgb(look.secondary))
                        .child(self.status.clone()),
                ),
            Some(text) => page
                .child(
                    div()
                        .id("starting-failed")
                        .role(Role::Alert)
                        .aria_label(text.clone())
                        .text_size(px(12.))
                        .text_color(rgb(RED))
                        .child(text.clone()),
                )
                .child(
                    row()
                        .gap(px(8.))
                        .mt(px(8.))
                        .child(
                            widgets::push_primary_shape("starting-retry", "Retry", look)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.retry(window, cx);
                                })),
                        )
                        .child(
                            widgets::push_quiet_shape("starting-dismiss", "Dismiss", look)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.dismiss(cx);
                                })),
                        )
                        .child(
                            widgets::push_quiet_shape("starting-quit", "Quit", look)
                                .on_click(|_, _, cx| cx.quit()),
                        ),
                ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Storage, saved_theme};

    #[test]
    fn nothing_saved_follows_the_system() {
        let storage = tca::MemoryStorage::new();
        assert_eq!(saved_theme(&storage), humata2_settings::Theme::System);
    }

    #[test]
    fn a_saved_choice_is_followed() {
        let storage = std::sync::Arc::new(tca::MemoryStorage::new());
        let mut saved = humata2_app::saved::SavedUi::default();
        saved.preferences.theme = humata2_settings::Theme::Light;
        humata2_app::saved::Saver::new(storage.clone())
            .save(1, &saved)
            .unwrap_or_else(|error| panic!("memory storage refused a save: {error}"));
        assert_eq!(saved_theme(storage.as_ref()), humata2_settings::Theme::Light);
    }

    #[test]
    fn unreadable_saved_state_follows_the_system() {
        let storage = tca::MemoryStorage::new();
        storage
            .save(humata2_app::saved::KEY, b"not json")
            .unwrap_or_else(|error| panic!("memory storage refused a save: {error}"));
        assert_eq!(saved_theme(&storage), humata2_settings::Theme::System);
    }
}
