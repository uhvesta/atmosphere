//! View snapshots: a drawn GPUI window as a tree of what a person can see
//! and use.
//!
//! Each line is an element marked with `.test_support()`, or a gpui-kit
//! component: its role and accessible label, then its value and states
//! (`selected`, `expanded` or `collapsed`, `checked`, `focused`,
//! `disabled`). Text recorded with `.test_text(..)` sits under the element
//! it is drawn in. Children are indented under their parents, in reading
//! order: top to bottom, then left to right.
//!
//! There are no sizes, positions, colours or fonts in it, so it reads the
//! same on every machine, and a change in it is a change in what the view
//! shows or how it behaves.
//!
//! ```text
//! Status "Count"
//!   text "2"
//! Button "Add" disabled
//! List "Items"
//!   ListItem "milk"
//!   ListItem "eggs" selected
//! ```

use std::cmp::Ordering;
use std::fmt::Write as _;

use gpui_kit::base::test_support::{TextSnapshot, texts};
use gpui_kit::test::{ElementSnapshot, TestQueryExt as _};
use gpui_kit::{Bounds, ElementId, Pixels, Window};
use thiserror::Error;

/// Why a window could not be written as a view snapshot.
#[derive(Debug, Error)]
pub enum ViewError {
    #[error(
        "the view snapshot can't tell whether {element} has keyboard focus: it can take focus, \
         but its focus handle was not observed. Call .test_support() before \
         .track_focus(&handle) on that element, then take the snapshot again"
    )]
    FocusBindingMissed { element: String },
}

/// The window's last drawn frame as a view snapshot. Draw a frame first,
/// with `window.render_frame(cx)`, so it shows the current state.
///
/// Only visible elements and text are included.
///
/// # Errors
///
/// [`ViewError::FocusBindingMissed`] when a focusable element's focus
/// can't be read.
pub fn capture(window: &Window) -> Result<String, ViewError> {
    let elements: Vec<ElementSnapshot> = window
        .elements()
        .into_iter()
        .filter(ElementSnapshot::visible)
        .collect();
    let texts: Vec<TextSnapshot> = texts(window)
        .into_iter()
        .filter(TextSnapshot::visible)
        .collect();
    Tree::new(&elements, &texts).write()
}

#[derive(Clone, Copy)]
enum Node {
    Element(usize),
    Text(usize),
}

struct Tree<'a> {
    elements: &'a [ElementSnapshot],
    texts: &'a [TextSnapshot],
    /// Children of each element, then (last) of the root.
    children: Vec<Vec<Node>>,
}

impl<'a> Tree<'a> {
    fn new(elements: &'a [ElementSnapshot], texts: &'a [TextSnapshot]) -> Self {
        let root = elements.len();
        let mut children = vec![Vec::new(); root + 1];
        for (index, element) in elements.iter().enumerate() {
            let parent = owner(elements, element.path(), Some(index)).unwrap_or(root);
            children[parent].push(Node::Element(index));
        }
        for (index, text) in texts.iter().enumerate() {
            let parent = owner(elements, text.path(), None);
            let repeats_label = parent.is_some_and(|parent| {
                elements[parent].label() == Some(text.text()) && !text.is_alternative()
            });
            if !repeats_label {
                children[parent.unwrap_or(root)].push(Node::Text(index));
            }
        }
        let mut tree = Self {
            elements,
            texts,
            children,
        };
        for index in 0..tree.children.len() {
            let mut nodes = std::mem::take(&mut tree.children[index]);
            nodes.sort_by(|a, b| reading_order(tree.bounds(*a), tree.bounds(*b)));
            tree.children[index] = nodes;
        }
        tree
    }

    fn bounds(&self, node: Node) -> Bounds<Pixels> {
        match node {
            Node::Element(index) => self.elements[index].bounds(),
            Node::Text(index) => self.texts[index].bounds(),
        }
    }

    fn write(&self) -> Result<String, ViewError> {
        let mut out = String::new();
        let root = self.elements.len();
        if self.children[root].is_empty() {
            out.push_str("(nothing observed: mark elements with .test_support())\n");
        }
        self.write_children(root, 0, &mut out)?;
        Ok(out)
    }

    fn write_children(&self, parent: usize, depth: usize, out: &mut String) -> Result<(), ViewError> {
        for node in &self.children[parent] {
            for _ in 0..depth {
                out.push_str("  ");
            }
            match *node {
                Node::Text(index) => {
                    let text = &self.texts[index];
                    let kind = if text.is_alternative() { "alt" } else { "text" };
                    let _ = writeln!(out, "{kind} {:?}", text.text());
                }
                Node::Element(index) => {
                    out.push_str(&describe(&self.elements[index])?);
                    out.push('\n');
                    self.write_children(index, depth + 1, out)?;
                }
            }
        }
        Ok(())
    }
}

/// The element whose path is the longest prefix of `path`: the closest
/// observed element that contains it. `except` is left out, so an element
/// is not its own owner.
fn owner(elements: &[ElementSnapshot], path: &[ElementId], except: Option<usize>) -> Option<usize> {
    elements
        .iter()
        .enumerate()
        .filter(|(index, element)| {
            Some(*index) != except
                && element.path().len() <= path.len()
                && path.starts_with(element.path())
                && (except.is_none() || element.path().len() < path.len())
        })
        .max_by_key(|(_, element)| element.path().len())
        .map(|(index, _)| index)
}

/// Top to bottom, then left to right. Ties keep the order GPUI gave.
fn reading_order(a: Bounds<Pixels>, b: Bounds<Pixels>) -> Ordering {
    a.origin
        .y
        .partial_cmp(&b.origin.y)
        .unwrap_or(Ordering::Equal)
        .then_with(|| a.origin.x.partial_cmp(&b.origin.x).unwrap_or(Ordering::Equal))
}

fn describe(element: &ElementSnapshot) -> Result<String, ViewError> {
    let mut line = match (element.role(), element.label()) {
        (Some(role), Some(label)) => format!("{role:?} {label:?}"),
        (Some(role), None) => format!("{role:?}"),
        (None, Some(label)) => format!("{label:?}"),
        (None, None) => format!("#{}", leaf(element)),
    };
    if let Some(value) = element.value() {
        let _ = write!(line, " value={value:?}");
    }
    if let Some(description) = element.description() {
        let _ = write!(line, " description={description:?}");
    }
    if let Some(placeholder) = element.placeholder() {
        let _ = write!(line, " placeholder={placeholder:?}");
    }
    if element.selected() == Some(true) {
        line.push_str(" selected");
    }
    match element.expanded() {
        Some(true) => line.push_str(" expanded"),
        Some(false) => line.push_str(" collapsed"),
        None => {}
    }
    match (element.checked(), element.indeterminate()) {
        (_, Some(true)) => line.push_str(" mixed"),
        (Some(true), _) => line.push_str(" checked"),
        (Some(false), _) => line.push_str(" unchecked"),
        (None, _) => {}
    }
    if focused(element)? {
        line.push_str(" focused");
    }
    if element.disabled() == Some(true) {
        line.push_str(" disabled");
    }
    Ok(line)
}

/// Whether the element has keyboard focus, without the panic
/// [`ElementSnapshot::focused`] raises for a missed focus binding: the
/// snapshot reports that as an error the test can show.
fn focused(element: &ElementSnapshot) -> Result<bool, ViewError> {
    // gpui-base marks a missed binding only in the snapshot's Debug output.
    if format!("{element:?}").contains("<binding missed>") {
        return Err(ViewError::FocusBindingMissed {
            element: describe_path(element),
        });
    }
    Ok(element.focused() == Some(true))
}

pub(crate) fn leaf(element: &ElementSnapshot) -> String {
    element.path().last().map(stable_id).unwrap_or_default()
}

/// An element's own id as a snapshot shows it. An id made from an entity
/// (gpui-component's text inputs are `("input", entity)`) leaves the
/// entity's number out: it counts the entities made before it, so it
/// changes whenever anything else makes one.
fn stable_id(id: &ElementId) -> String {
    match id {
        ElementId::View(_) => "view".to_owned(),
        // Entity ids carry a slot version in their high half, so they are
        // never below 2^32; the counters apps put in ids are.
        ElementId::NamedInteger(name, number) if *number >> 32 != 0 => name.to_string(),
        other => other.to_string(),
    }
}

fn describe_path(element: &ElementSnapshot) -> String {
    let path: Vec<String> = element.path().iter().map(ToString::to_string).collect();
    match (element.role(), element.label()) {
        (Some(role), Some(label)) => format!("the {role:?} {label:?} ({})", path.join(" > ")),
        _ => format!("the element at {}", path.join(" > ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::EntityId;

    #[test]
    fn ids_made_from_entities_leave_the_entity_out() {
        let entity = (1u64 << 32) | 37;
        assert_eq!(stable_id(&ElementId::NamedInteger("input".into(), entity)), "input");
        assert_eq!(stable_id(&ElementId::View(EntityId::from(entity))), "view");
        assert_eq!(stable_id(&ElementId::NamedInteger("row".into(), 3)), "row-3");
        assert_eq!(stable_id(&ElementId::Name("terminal-pane".into())), "terminal-pane");
    }
}
