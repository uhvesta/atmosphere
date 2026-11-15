use tca::Effect;

use crate::saved::SavedUi;
use crate::{App, AppAction, AppState, DRAFT_SAVE, Load};

impl App {
    /// Saves where the person is if it changed: at once for a structural
    /// change, within a second for drafts.
    pub(super) fn persist(&self, state: &mut AppState) -> Effect<AppAction> {
        if state.load != Load::Ready {
            return Effect::none();
        }
        let now = state.saved();
        if now == state.last_saved {
            return Effect::none();
        }
        if now.structure() == state.last_saved.structure() {
            if state.save_due {
                return Effect::none();
            }
            state.save_due = true;
            let sleep = self.clients.clock.sleep(DRAFT_SAVE);
            return Effect::future(async move {
                sleep.await;
                AppAction::SaveDue
            });
        }
        self.save(state, now)
    }

    pub(super) fn save(&self, state: &mut AppState, now: SavedUi) -> Effect<AppAction> {
        state.save_generation += 1;
        let generation = state.save_generation;
        state.last_saved = now.clone();
        let saver = self.saver.clone();
        Effect::future(async move {
            AppAction::Saved(
                saver
                    .save(generation, &now)
                    .err()
                    .map(|error| error.to_string()),
            )
        })
    }

    pub(super) fn save_if_due(&self, state: &mut AppState) -> Effect<AppAction> {
        state.save_due = false;
        let now = state.saved();
        if now == state.last_saved {
            return Effect::none();
        }
        self.save(state, now)
    }

    pub(super) fn save_finished(
        &self,
        state: &mut AppState,
        error: Option<String>,
    ) -> Effect<AppAction> {
        let clock = self.clients.clock.clone();
        match error {
            Some(error) => state.toast(error, &clock),
            None => Effect::none(),
        }
    }
}
