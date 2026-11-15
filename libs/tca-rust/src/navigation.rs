//! Navigation as state: a stack of screens, a sheet or popover that may be
//! presented, and alerts, with reducers that run the presented features
//! and dismiss them, cancelling their effects.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::effect::{CancelId, Effect};
use crate::reducer::Reducer;

/// Gives every presentation and stack in the process scopes of its own, so
/// two sheets' effects never share a cancel id.
static SCOPES: AtomicU64 = AtomicU64::new(0);

fn next_scope() -> u64 {
    SCOPES.fetch_add(1, Ordering::Relaxed)
}

/// A feature that may be presented, such as a sheet, a popover or an
/// alert. It's equal to another when both present equal states, or
/// neither does.
pub struct PresentationState<T> {
    presented: Option<T>,
    /// Names this presentation's effects; new for each present.
    scope: u64,
}

impl<T> PresentationState<T> {
    /// Nothing presented.
    pub fn none() -> Self {
        Self {
            presented: None,
            scope: next_scope(),
        }
    }

    /// `state` presented.
    pub fn presenting(state: T) -> Self {
        let mut presentation = Self::none();
        presentation.present(state);
        presentation
    }

    /// Presents `state`, in place of what was presented. Dismiss first, with
    /// [`dismiss`](Self::dismiss), to stop what was presented's effects.
    pub fn present(&mut self, state: T) {
        self.presented = Some(state);
        self.scope = next_scope();
    }

    /// Dismisses what's presented, returning the effect that cancels its
    /// effects.
    pub fn dismiss<A: Send + 'static>(&mut self) -> Effect<A> {
        match self.presented.take() {
            Some(_) => Effect::cancel(self.cancel_id()),
            None => Effect::none(),
        }
    }

    /// Whether something is presented.
    pub fn is_presented(&self) -> bool {
        self.presented.is_some()
    }

    /// What's presented.
    pub fn get(&self) -> Option<&T> {
        self.presented.as_ref()
    }

    /// What's presented, to change.
    pub fn get_mut(&mut self) -> Option<&mut T> {
        self.presented.as_mut()
    }

    fn cancel_id(&self) -> CancelId {
        CancelId::from(format!("presentation:{}", self.scope))
    }
}

impl<T> Default for PresentationState<T> {
    fn default() -> Self {
        Self::none()
    }
}

impl<T> From<Option<T>> for PresentationState<T> {
    fn from(state: Option<T>) -> Self {
        let mut presentation = Self::none();
        if let Some(state) = state {
            presentation.present(state);
        }
        presentation
    }
}

impl<T: Clone> Clone for PresentationState<T> {
    fn clone(&self) -> Self {
        Self {
            presented: self.presented.clone(),
            scope: self.scope,
        }
    }
}

impl<T: PartialEq> PartialEq for PresentationState<T> {
    fn eq(&self, other: &Self) -> bool {
        self.presented == other.presented
    }
}

impl<T: Eq> Eq for PresentationState<T> {}

impl<T: fmt::Debug> fmt::Debug for PresentationState<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("PresentationState")
            .field(&self.presented)
            .finish()
    }
}

/// An action for a presented feature, or to dismiss it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PresentationAction<A> {
    /// For the feature presented. Dropped if none is.
    Presented(A),
    /// Dismisses the feature, cancelling its effects. A presented feature
    /// sends it itself by returning [`Effect::dismiss`].
    Dismiss,
}

/// Runs `child` on what `state` presents, and dismisses it. Its effects
/// are cancelled once it's dismissed, its cancel ids kept apart from
/// everyone else's, and an [`Effect::dismiss`] it returns comes back as
/// [`PresentationAction::Dismiss`]. Run this, then handle what the parent
/// itself wants of the child's actions.
pub fn if_presented<R>(
    child: &R,
    state: &mut PresentationState<R::State>,
    action: PresentationAction<R::Action>,
) -> Effect<PresentationAction<R::Action>>
where
    R: Reducer,
    R::Action: Send + 'static,
{
    match action {
        PresentationAction::Presented(action) => {
            let id = state.cancel_id();
            let Some(presented) = state.presented.as_mut() else {
                return Effect::none();
            };
            let dismissed: Arc<dyn Fn() -> PresentationAction<R::Action> + Send + Sync> =
                Arc::new(|| PresentationAction::Dismiss);
            child
                .reduce(presented, action)
                .scoped(&id.to_string())
                .cancellable(id, false)
                .map(PresentationAction::Presented)
                .on_dismiss(&dismissed)
        }
        PresentationAction::Dismiss => state.dismiss(),
    }
}

/// Names a screen in a [`StackState`]. Ids count up from 0 in each stack
/// and are never used twice in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StackElementId(pub u64);

impl fmt::Display for StackElementId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// A stack of screens, the first at the bottom, each with an id that stays
/// the same while it's there. Two stacks are equal when they hold equal
/// screens under the same ids.
pub struct StackState<T> {
    elements: Vec<(StackElementId, T)>,
    next: u64,
    /// Names this stack's effects apart from other stacks'.
    scope: u64,
}

impl<T> StackState<T> {
    /// An empty stack.
    pub fn new() -> Self {
        Self {
            elements: Vec::new(),
            next: 0,
            scope: next_scope(),
        }
    }

    /// Puts `state` on top, returning its id.
    pub fn push(&mut self, state: T) -> StackElementId {
        let id = StackElementId(self.next);
        self.next += 1;
        self.elements.push((id, state));
        id
    }

    /// Takes the screen `id` and every screen above it off, returning the
    /// effect that cancels their effects.
    pub fn pop_from<A: Send + 'static>(&mut self, id: StackElementId) -> Effect<A> {
        let Some(at) = self.elements.iter().position(|(each, _)| *each == id) else {
            return Effect::none();
        };
        let gone: Vec<StackElementId> = self.elements.drain(at..).map(|(id, _)| id).collect();
        Effect::merge(gone.into_iter().map(|id| Effect::cancel(self.cancel_id(id))))
    }

    /// Takes the top screen off, returning the effect that cancels its
    /// effects.
    pub fn pop<A: Send + 'static>(&mut self) -> Effect<A> {
        match self.elements.last().map(|(id, _)| *id) {
            Some(id) => self.pop_from(id),
            None => Effect::none(),
        }
    }

    /// Takes every screen off, returning the effect that cancels their
    /// effects.
    pub fn pop_all<A: Send + 'static>(&mut self) -> Effect<A> {
        match self.elements.first().map(|(id, _)| *id) {
            Some(id) => self.pop_from(id),
            None => Effect::none(),
        }
    }

    /// The screen `id`, if it's there.
    pub fn get(&self, id: StackElementId) -> Option<&T> {
        self.elements
            .iter()
            .find(|(each, _)| *each == id)
            .map(|(_, state)| state)
    }

    /// The screen `id`, to change.
    pub fn get_mut(&mut self, id: StackElementId) -> Option<&mut T> {
        self.elements
            .iter_mut()
            .find(|(each, _)| *each == id)
            .map(|(_, state)| state)
    }

    /// The top screen and its id.
    pub fn last(&self) -> Option<(StackElementId, &T)> {
        self.elements.last().map(|(id, state)| (*id, state))
    }

    /// The screens from the bottom up, with their ids.
    pub fn iter(&self) -> impl Iterator<Item = (StackElementId, &T)> + '_ {
        self.elements.iter().map(|(id, state)| (*id, state))
    }

    /// The ids from the bottom up.
    pub fn ids(&self) -> impl Iterator<Item = StackElementId> + '_ {
        self.elements.iter().map(|(id, _)| *id)
    }

    pub fn len(&self) -> usize {
        self.elements.len()
    }

    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    fn cancel_id(&self, id: StackElementId) -> CancelId {
        CancelId::from(format!("stack:{}:{}", self.scope, id.0))
    }
}

impl<T> Default for StackState<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> FromIterator<T> for StackState<T> {
    fn from_iter<I: IntoIterator<Item = T>>(states: I) -> Self {
        let mut stack = Self::new();
        for state in states {
            stack.push(state);
        }
        stack
    }
}

impl<T: Clone> Clone for StackState<T> {
    fn clone(&self) -> Self {
        Self {
            elements: self.elements.clone(),
            next: self.next,
            scope: self.scope,
        }
    }
}

impl<T: PartialEq> PartialEq for StackState<T> {
    fn eq(&self, other: &Self) -> bool {
        self.elements == other.elements
    }
}

impl<T: Eq> Eq for StackState<T> {}

impl<T: fmt::Debug> fmt::Debug for StackState<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map()
            .entries(self.elements.iter().map(|(id, state)| (id, state)))
            .finish()
    }
}

/// An action for a stack: push a screen, pop one with those above it, or
/// one for a screen on it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StackAction<S, A> {
    Push(S),
    /// Pops the screen and those above it, cancelling their effects. A
    /// screen sends it for itself by returning [`Effect::dismiss`].
    PopFrom(StackElementId),
    /// For the screen with this id. Dropped if it's gone.
    Element(StackElementId, A),
}

/// Runs `child` on the screen of `stack` an action is for, and pushes and
/// pops screens. Each screen's effects are kept apart from the others' and
/// cancelled when it's popped, and an [`Effect::dismiss`] it returns pops
/// it. Run this, then handle what the parent itself wants of the actions.
pub fn for_each_in_stack<R>(
    child: &R,
    stack: &mut StackState<R::State>,
    action: StackAction<R::State, R::Action>,
) -> Effect<StackAction<R::State, R::Action>>
where
    R: Reducer,
    R::State: Send + 'static,
    R::Action: Send + 'static,
{
    match action {
        StackAction::Push(state) => {
            stack.push(state);
            Effect::none()
        }
        StackAction::PopFrom(id) => stack.pop_from(id),
        StackAction::Element(id, action) => {
            let cancel_id = stack.cancel_id(id);
            let Some(element) = stack.get_mut(id) else {
                return Effect::none();
            };
            let dismissed: Arc<dyn Fn() -> StackAction<R::State, R::Action> + Send + Sync> =
                Arc::new(move || StackAction::PopFrom(id));
            child
                .reduce(element, action)
                .scoped(&cancel_id.to_string())
                .cancellable(cancel_id, false)
                .map(move |action| StackAction::Element(id, action))
                .on_dismiss(&dismissed)
        }
    }
}

/// What an alert's button is for, which a view shows by its look and
/// place.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonRole {
    #[default]
    Default,
    /// Leaves without doing anything.
    Cancel,
    /// Does something that can't be undone, such as deleting.
    Destructive,
}

/// A button on an alert: what it says and the action it sends, if any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ButtonState<A> {
    pub label: String,
    pub role: ButtonRole,
    /// What it sends, as [`PresentationAction::Presented`]; with none it
    /// only dismisses.
    pub action: Option<A>,
}

impl<A> ButtonState<A> {
    pub fn new(label: impl Into<String>, action: A) -> Self {
        Self {
            label: label.into(),
            role: ButtonRole::Default,
            action: Some(action),
        }
    }

    /// A button that dismisses the alert and does nothing more.
    pub fn cancel(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            role: ButtonRole::Cancel,
            action: None,
        }
    }

    pub fn destructive(label: impl Into<String>, action: A) -> Self {
        Self {
            label: label.into(),
            role: ButtonRole::Destructive,
            action: Some(action),
        }
    }

    /// What tapping it sends.
    pub fn tapped(&self) -> PresentationAction<A>
    where
        A: Clone,
    {
        match &self.action {
            Some(action) => PresentationAction::Presented(action.clone()),
            None => PresentationAction::Dismiss,
        }
    }
}

/// An alert or confirmation as state: a title, a message and its buttons.
/// Present it in a [`PresentationState`] and handle its taps with
/// [`alert`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlertState<A> {
    pub title: String,
    pub message: Option<String>,
    pub buttons: Vec<ButtonState<A>>,
}

impl<A> AlertState<A> {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            message: None,
            buttons: Vec::new(),
        }
    }

    pub fn message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }

    pub fn button(mut self, button: ButtonState<A>) -> Self {
        self.buttons.push(button);
        self
    }
}

/// Dismisses the alert `state` presents for a tap on it, returning the
/// action of the button tapped, if it had one, for the parent to handle.
pub fn alert<A>(
    state: &mut PresentationState<AlertState<A>>,
    action: PresentationAction<A>,
) -> Option<A> {
    let presented = state.presented.take();
    match action {
        PresentationAction::Presented(action) => presented.map(|_| action),
        PresentationAction::Dismiss => None,
    }
}
