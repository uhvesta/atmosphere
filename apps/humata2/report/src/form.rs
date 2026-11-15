//! The short form a person fills in after a recording or a snapshot.

use crate::event::Element;

/// What the person says about the report.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Form {
    /// What happened.
    pub happened: String,
    /// What they expected instead.
    pub expected: String,
    /// Comments pinned to elements they picked.
    pub comments: Vec<Comment>,
}

/// A comment pinned to an element, saved as the element's path in the view
/// tree, not as a picture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Comment {
    pub element: Element,
    pub text: String,
}
