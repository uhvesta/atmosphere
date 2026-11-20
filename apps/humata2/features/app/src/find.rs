//! Search and the Pinned section in the app (LD-23, LD-24, NV-20 to
//! NV-25): where a picked hit or a picked pin opens.

use humata2_clients::marks::Thing;
use humata2_find::{FindDelegate, PinnedDelegate};
use humata2_session::{Panel, SessionAction};
use tca::Effect;

use crate::{App, AppAction, AppState, Overlay, Route};

impl App {
    /// A picked hit opens its session, and the palette it was picked in
    /// closes. Back returns to where the person was.
    pub(super) fn find_delegate(&self, state: &mut AppState, delegate: &FindDelegate) -> Effect<AppAction> {
        match delegate {
            // A picked hit opens its session on a Conversation tab at the
            // hit's entry (D186). Navigating first, so Back returns to the search.
            FindDelegate::OpenTurn { session, turn, seq } => {
                state.overlays.retain(|overlay| overlay != &Overlay::Palette);
                let go = self.navigate(state, Route::Session(session.clone()));
                let open = self.child(
                    state,
                    AppAction::Session(SessionAction::OpenConversation {
                        session: session.clone(),
                        turn: *turn,
                        seq: *seq,
                    }),
                );
                Effect::merge([go, open])
            }
        }
    }

    /// A picked pin opens what it pins, as a sidebar row does: lateral, so
    /// Back is cleared.
    pub(super) fn pinned_delegate(&self, state: &mut AppState, delegate: &PinnedDelegate) -> Effect<AppAction> {
        let PinnedDelegate::Open(thing) = delegate;
        match thing {
            Thing::Session(id) => self.switch(state, Route::Session(id.clone())),
            Thing::Channel(id) => self.switch(state, Route::Channel(id.clone())),
            Thing::Machine(id) => self.switch(state, Route::Machine(id.clone())),
            Thing::PrReview(id) => {
                let go = self.switch(state, Route::Session(id.clone()));
                let panel = self.handle(
                    state,
                    AppAction::Session(SessionAction::SetPanel(Some(Panel::Changes))),
                );
                Effect::merge([go, panel])
            }
            // A pinned board item opens in the To-do list, whose key is
            // the item's id (D874).
            Thing::BoardItem(id) => {
                let go = self.switch(state, Route::Todo);
                let open = self.child(state, AppAction::Todo(humata2_todo_feature::TodoAction::Open(id.clone())));
                Effect::merge([go, open])
            }
        }
    }
}
