//! A toy app to take reports of: a list with a "New item" field and an Add
//! button, a masked "Token" field, and a terminal. It renders its state,
//! view and layout as text the way the app's snapshots do, and a
//! [`Driver`] records what is done to it.

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

use humata2_report::{
    Button, Captured, Context, Daemon, Element, FlowAction, Gesture, Limits, Machine, Pasted,
    Recording, Session, Trees, WindowSize,
};

/// 2026-11-14 10:30 UTC.
pub const STARTED: u64 = 1_794_652_200_000;

pub fn context() -> Context {
    Context {
        started_unix_ms: STARTED,
        app_version: "0.1.0 (1)".to_owned(),
        os: "macOS 26.7".to_owned(),
        window: WindowSize {
            width: 800,
            height: 600,
        },
        daemons: vec![Daemon {
            machine: "studio".to_owned(),
            version: "0.1.0".to_owned(),
        }],
        machines: vec![Machine {
            name: "studio".to_owned(),
            kind: "local".to_owned(),
            state: "online".to_owned(),
        }],
        sessions: vec![Session {
            id: "s-1".to_owned(),
            title: "fix the list".to_owned(),
            channel: "web".to_owned(),
            machine: "studio".to_owned(),
            agent: "shell".to_owned(),
        }],
    }
}

pub fn draft_field() -> Element {
    Element::new("TextInput", "New item").inside("Window", "Toy")
}

pub fn add_button() -> Element {
    Element::new("Button", "Add").inside("Window", "Toy")
}

pub fn token_field() -> Element {
    Element::new("PasswordInput", "Token").inside("Window", "Toy")
}

pub fn terminal() -> Element {
    Element::new("Terminal", "shell").inside("Window", "Toy")
}

pub fn item(label: &str) -> Element {
    Element::new("ListItem", label)
        .inside("List", "Items")
        .inside("Window", "Toy")
}

/// The app's state, as its reducer holds it. The terminal's text isn't in
/// it: the terminal holds that.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct State {
    draft: String,
    items: Vec<String>,
    token: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Toy {
    state: State,
    terminal: Vec<String>,
}

/// What an input does to the toy, however it arrived.
enum Input<'a> {
    Text { into: &'a str, text: &'a str },
    Enter { on: &'a str },
    Backspace { on: &'a str },
    Click { role: &'a str, label: &'a str },
    Nothing,
}

impl Toy {
    pub fn state(&self) -> String {
        // Ends in a newline, as a report stores its trees.
        format!("{:#?}\n", self.state)
    }

    pub fn view(&self) -> String {
        let mut view = String::from("Window \"Toy\"\n");
        view.push_str(&format!("  TextInput \"New item\" value={:?}\n", self.state.draft));
        view.push_str("  Button \"Add\"\n");
        let dots = "•".repeat(self.state.token.chars().count());
        view.push_str(&format!("  PasswordInput \"Token\" value={dots:?}\n"));
        view.push_str("  List \"Items\"\n");
        for item in &self.state.items {
            view.push_str(&format!("    ListItem {item:?}\n"));
        }
        view.push_str("  Terminal \"shell\"\n");
        for line in &self.terminal {
            view.push_str(&format!("    text {line:?}\n"));
        }
        view
    }

    pub fn layout(&self) -> String {
        let mut layout = String::from("window 800x600, test fonts\n");
        layout.push_str("Window \"Toy\" 0,0 800x600\n");
        layout.push_str("  TextInput \"New item\" 8,8 600x24\n");
        layout.push_str("  Button \"Add\" 616,8 64x24\n");
        layout.push_str("  PasswordInput \"Token\" 8,40 600x24\n");
        layout.push_str("  List \"Items\" 8,72 784x200\n");
        for (row, item) in self.state.items.iter().enumerate() {
            layout.push_str(&format!("    ListItem {item:?} 8,{} 784x20\n", 72 + row * 20));
        }
        layout.push_str("  Terminal \"shell\" 8,280 784x312\n");
        for (row, line) in self.terminal.iter().enumerate() {
            layout.push_str(&format!("    text {line:?} 8,{} 784x16\n", 280 + row * 16));
        }
        layout
    }

    pub fn trees(&self) -> Trees {
        Trees {
            state: self.state(),
            view: self.view(),
            layout: self.layout(),
        }
    }

    /// Plays a flow step, as a flow would: text goes to the "New item"
    /// field, which a flow focuses.
    pub fn play(&mut self, action: &FlowAction) {
        let _ = match action {
            FlowAction::Type(text) | FlowAction::Paste(text) => self.input(Input::Text {
                into: "New item",
                text,
            }),
            FlowAction::Key(key) if key == "enter" => self.input(Input::Enter { on: "New item" }),
            FlowAction::Key(key) if key == "backspace" => {
                self.input(Input::Backspace { on: "New item" })
            }
            FlowAction::Click(target) => self.input(Input::Click {
                role: &target.role,
                label: &target.label,
            }),
            FlowAction::Key(_) | FlowAction::Drag { .. } => None,
        };
    }

    /// Does what `gesture` does, and returns the reducer action it sent.
    fn gesture(&mut self, gesture: &Gesture) -> Option<String> {
        let label = |target: &Option<Element>| {
            target
                .as_ref()
                .and_then(|target| target.label.clone())
                .unwrap_or_default()
        };
        let input = match gesture {
            Gesture::Type { text, target }
            | Gesture::Paste {
                target,
                content: Pasted::Text(text),
            } => {
                let into = label(target);
                return self.input(Input::Text { into: &into, text });
            }
            Gesture::Key { key, target } => {
                let on = label(target);
                return match key.as_str() {
                    "enter" => self.input(Input::Enter { on: &on }),
                    "backspace" => self.input(Input::Backspace { on: &on }),
                    _ => None,
                };
            }
            Gesture::Click {
                target,
                button: Button::Left,
            } => {
                let role = target.role.clone();
                let label = target.label.clone().unwrap_or_default();
                return self.input(Input::Click {
                    role: &role,
                    label: &label,
                });
            }
            _ => Input::Nothing,
        };
        self.input(input)
    }

    fn input(&mut self, input: Input<'_>) -> Option<String> {
        match input {
            Input::Text { into: "New item", text } => {
                self.state.draft.push_str(text);
                Some(format!("DraftChanged({:?})", self.state.draft))
            }
            Input::Text { into: "Token", text } => {
                self.state.token.push_str(text);
                Some(format!("TokenChanged({:?})", self.state.token))
            }
            Input::Text { into: "shell", text } => {
                match self.terminal.last_mut() {
                    Some(line) => line.push_str(text),
                    None => self.terminal.push(text.to_owned()),
                }
                None
            }
            Input::Enter { on: "New item" } | Input::Click { role: "Button", label: "Add" } => {
                if self.state.draft.is_empty() {
                    return None;
                }
                let draft = std::mem::take(&mut self.state.draft);
                self.state.items.push(draft);
                Some("AddTapped".to_owned())
            }
            Input::Enter { on: "shell" } => {
                self.terminal.push(String::new());
                None
            }
            Input::Backspace { on: "New item" } => {
                self.state.draft.pop();
                Some(format!("DraftChanged({:?})", self.state.draft))
            }
            Input::Click { role: "ListItem", label } => {
                let at = self.state.items.iter().position(|item| item == label)?;
                self.state.items.remove(at);
                Some(format!("Removed({at})"))
            }
            _ => None,
        }
    }
}

/// Plays gestures on a [`Toy`] and records each, with the action it sent
/// and the trees after it, 100 ms apart.
pub struct Driver {
    pub toy: Toy,
    pub recording: Recording,
    pub now: u64,
    /// Every state, view and layout the toy had, oldest first.
    pub history: Vec<Trees>,
}

impl Driver {
    pub fn start() -> Self {
        Self::with_limits(Limits::default())
    }

    pub fn with_limits(limits: Limits) -> Self {
        let toy = Toy::default();
        Self {
            recording: Recording::start(context(), toy.trees(), limits),
            history: vec![toy.trees()],
            toy,
            now: 0,
        }
    }

    /// Does `gesture` and records it and what followed.
    pub fn act(&mut self, gesture: Gesture) {
        self.now += 100;
        let at = self.now;
        let action = self.toy.gesture(&gesture);
        self.recording.interaction(at, gesture);
        if let Some(action) = action {
            self.recording.action(at + 1, &action, &self.toy.state());
        }
        self.recording.view(at + 2, &self.toy.view());
        self.recording.layout(at + 3, &self.toy.layout());
        self.history.push(self.toy.trees());
    }

    pub fn type_into(&mut self, target: Element, text: &str) {
        self.act(Gesture::Type {
            text: text.to_owned(),
            target: Some(target),
        });
    }

    pub fn key(&mut self, target: Element, key: &str) {
        self.act(Gesture::Key {
            key: key.to_owned(),
            target: Some(target),
        });
    }

    pub fn click(&mut self, target: Element) {
        self.act(Gesture::Click {
            target,
            button: Button::Left,
        });
    }

    pub fn stop(self) -> Captured {
        let at = self.now + 100;
        self.recording.stop(at)
    }
}

/// Every regular file in `dir`, by name, with its text.
pub fn files(dir: &Path) -> Vec<(String, String)> {
    let mut files: Vec<(String, String)> = fs::read_dir(dir)
        .expect("the report folder can be read")
        .map(|entry| entry.expect("a folder entry").path())
        .filter(|path| path.is_file())
        .map(|path: PathBuf| {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_owned();
            let text = fs::read_to_string(&path).expect("report files are text");
            (name, text)
        })
        .collect();
    files.sort();
    files
}

/// Fails if any file in `dir` holds `length` characters of `secret` in a
/// row.
pub fn assert_no_piece_of(dir: &Path, secret: &str, length: usize) {
    let chars: Vec<char> = secret.chars().collect();
    let pieces: Vec<String> = chars
        .windows(length.min(chars.len()))
        .map(|piece| piece.iter().collect())
        .collect();
    for (name, text) in files(dir) {
        for piece in &pieces {
            assert!(
                !text.contains(piece.as_str()),
                "{name} holds {piece:?}, part of {secret:?}:\n{text}"
            );
        }
    }
}
