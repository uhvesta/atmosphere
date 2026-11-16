//! The short form a person fills in after a recording or a snapshot.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

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
    /// Marks they drew on the window.
    pub drawing: Drawing,
}

/// A comment pinned to an element, saved as the element's path in the view
/// tree, not as a picture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Comment {
    pub element: Element,
    pub text: String,
    /// Where the element sat when the report was taken, in window points,
    /// when it is known; it places the comment on `marks.svg`.
    pub place: Option<Bounds>,
}

/// A box in window points.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// A size in window points.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

/// A position in window points.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

/// What a stroke looks like.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    /// A path that ends near its start.
    Loop,
    /// Any other path.
    Line,
    /// A path too short to be a line.
    Dot,
}

/// One mark drawn on the window.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stroke {
    pub shape: Shape,
    /// The path, in window points.
    pub points: Vec<Point>,
    /// The elements it covers, at most a few.
    pub covers: Vec<Element>,
    /// How many more elements it covers than are listed.
    #[serde(default)]
    pub more: usize,
}

impl Stroke {
    /// The stroke in one line, as the report and the prompt name it.
    pub fn describe(&self) -> String {
        let what = match self.shape {
            Shape::Loop => "a circle around",
            Shape::Line => "a line across",
            Shape::Dot => "a dot on",
        };
        let mut line = if self.covers.is_empty() {
            match self.points.first() {
                Some(point) => format!("{what} an unlabelled area near ({}, {})", point.x, point.y),
                None => format!("{what} an unlabelled area"),
            }
        } else {
            let covers: Vec<String> = self.covers.iter().map(ToString::to_string).collect();
            format!("{what} {}", covers.join(", "))
        };
        if self.more > 0 {
            let _ = write!(line, " and {} more", self.more);
        }
        line
    }
}

/// The marks drawn on the window and the window they were drawn on.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Drawing {
    pub window: Size,
    pub strokes: Vec<Stroke>,
}
