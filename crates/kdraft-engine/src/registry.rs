//! The command registry: every user action is a named command with typed,
//! JSON-deserialisable arguments and a machine-readable description. GUI
//! and CLI execute commands by name.

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;

use crate::error::{EngineError, R};
use crate::session::Session;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CommandKind {
    /// Changes the document; one undo step.
    Edit,
    /// Changes only the selection; not an undo step.
    Selection,
    /// Opens, creates or saves a file; clears or keeps history.
    File,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ArgType {
    String,
    Number,
    Integer,
    Bool,
    /// A page or item id.
    Id,
    IdList,
    StringList,
    /// `[x, y]` in mm.
    Point,
    /// `[[x, y], ...]` in mm.
    PointList,
    Enum {
        values: &'static [&'static str],
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ArgSpec {
    pub name: &'static str,
    #[serde(rename = "type")]
    pub ty: ArgType,
    pub required: bool,
    pub description: &'static str,
}

/// Required argument.
pub fn req(name: &'static str, ty: ArgType, description: &'static str) -> ArgSpec {
    ArgSpec {
        name,
        ty,
        required: true,
        description,
    }
}

/// Optional argument.
pub fn opt(name: &'static str, ty: ArgType, description: &'static str) -> ArgSpec {
    ArgSpec {
        name,
        ty,
        required: false,
        description,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CommandSpec {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub kind: CommandKind,
    pub args: Vec<ArgSpec>,
    /// Presentation metadata (filled from [`crate::meta::meta`]).
    pub category: crate::meta::Category,
    pub icon: Option<&'static str>,
    pub shortcut: Option<&'static str>,
    /// Runs without further arguments: shown in menus and the palette.
    pub menu: bool,
    pub variants: &'static [crate::meta::Variant],
}

/// What a command did.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Outcome {
    /// Ids of pages and items that changed.
    pub changed: Vec<String>,
    /// Ids of pages and items that were created.
    pub created: Vec<String>,
    pub messages: Vec<String>,
}

impl Outcome {
    pub fn changed<I: IntoIterator<Item = S>, S: ToString>(ids: I) -> Outcome {
        Outcome {
            changed: ids.into_iter().map(|i| i.to_string()).collect(),
            ..Default::default()
        }
    }

    pub fn created<I: IntoIterator<Item = S>, S: ToString>(ids: I) -> Outcome {
        Outcome {
            created: ids.into_iter().map(|i| i.to_string()).collect(),
            ..Default::default()
        }
    }

    pub fn msg(mut self, m: impl Into<String>) -> Outcome {
        self.messages.push(m.into());
        self
    }
}

type Run = Box<dyn Fn(&mut Session, Value) -> R<Outcome> + Send + Sync>;
type Check = Box<dyn Fn(&Value) -> Result<(), String> + Send + Sync>;

struct Command {
    spec: CommandSpec,
    run: Run,
    check: Check,
}

pub struct Registry {
    commands: Vec<Command>,
}

impl Default for Registry {
    fn default() -> Self {
        Registry::new()
    }
}

fn normalise(v: Value) -> Value {
    if v.is_null() {
        Value::Object(Default::default())
    } else {
        v
    }
}

impl Registry {
    /// All built-in commands.
    pub fn new() -> Registry {
        let mut r = Registry {
            commands: Vec::new(),
        };
        crate::commands::register(&mut r);
        r
    }

    /// Adds a command whose arguments deserialise into `A`.
    pub fn add<A: DeserializeOwned + 'static>(
        &mut self,
        mut spec: CommandSpec,
        f: fn(&mut Session, A) -> R<Outcome>,
    ) {
        let name = spec.name;
        let m = crate::meta::meta(name);
        spec.category = m.category;
        spec.icon = m.icon;
        spec.shortcut = m.shortcut;
        spec.menu = m.menu;
        spec.variants = m.variants;
        let run: Run = Box::new(move |s, v| {
            let a: A = serde_json::from_value(normalise(v)).map_err(|e| EngineError::BadArgs {
                command: name.to_string(),
                message: e.to_string(),
            })?;
            f(s, a)
        });
        let check: Check = Box::new(|v| {
            serde_json::from_value::<A>(normalise(v.clone()))
                .map(|_| ())
                .map_err(|e| e.to_string())
        });
        self.commands.push(Command { spec, run, check });
    }

    pub fn specs(&self) -> impl Iterator<Item = &CommandSpec> {
        self.commands.iter().map(|c| &c.spec)
    }

    pub fn spec(&self, name: &str) -> Option<&CommandSpec> {
        self.find(name).ok().map(|c| &c.spec)
    }

    fn find(&self, name: &str) -> R<&Command> {
        self.commands
            .iter()
            .find(|c| c.spec.name == name)
            .ok_or_else(|| EngineError::UnknownCommand(name.to_string()))
    }

    /// Whether `args` deserialise for the command (no execution).
    pub fn check_args(&self, name: &str, args: &Value) -> R<()> {
        let c = self.find(name)?;
        (c.check)(args).map_err(|message| EngineError::BadArgs {
            command: name.to_string(),
            message,
        })
    }

    /// Executes a command. Edit commands become one undo step if the
    /// document changed; on error the session is left as it was.
    pub fn execute(&self, s: &mut Session, name: &str, args: Value) -> R<Outcome> {
        let c = self.find(name)?;
        if s.in_gesture() {
            s.commit_gesture();
        }
        let before = s.snapshot();
        match (c.run)(s, args) {
            Ok(out) => {
                match c.spec.kind {
                    CommandKind::Edit => {
                        s.after_edit(&before.doc);
                        s.record(c.spec.title, before);
                    }
                    CommandKind::Selection => {
                        if *s.doc() != *before.doc {
                            // Selection commands must not edit.
                            s.restore(&before);
                            return Err(EngineError::invalid(format!(
                                "{name} changed the document"
                            )));
                        }
                    }
                    CommandKind::File => {}
                }
                s.sanitize_selection();
                Ok(out)
            }
            Err(e) => {
                if c.spec.kind != CommandKind::File {
                    s.restore(&before);
                }
                Err(e)
            }
        }
    }

    /// Runs an edit command inside the current gesture: the document is
    /// first reset to the gesture's start, so repeated previews (a drag)
    /// replace each other and [`Session::commit_gesture`] records one step.
    pub fn preview(&self, s: &mut Session, name: &str, args: Value) -> R<Outcome> {
        let c = self.find(name)?;
        if c.spec.kind != CommandKind::Edit {
            return Err(EngineError::invalid(format!(
                "{name} is not an edit command"
            )));
        }
        s.gesture_reset()?;
        let base = s.doc_arc();
        let r = (c.run)(s, args);
        if r.is_err() {
            s.gesture_reset()?;
        } else {
            s.after_edit(&base);
            if !Arc::ptr_eq(&base, &s.doc_arc()) {
                s.touch();
            }
        }
        r
    }

    /// The command table as JSON (for `kdraft cmd list --json`).
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self.specs().collect::<Vec<_>>()).unwrap_or(Value::Null)
    }
}
