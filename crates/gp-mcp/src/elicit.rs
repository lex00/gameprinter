//! Questions to the person as MCP elicitation forms. When the client said
//! at its start that it shows forms (its `elicitation` capability), a tool
//! that needs the person's answer asks them itself, in the same words and
//! fields every time, instead of leaving the agent to put the question in
//! its own words: the yes to a call with no page open ([`yes_form`]), and
//! the path of their own ROM when a copy needs one ([`rom_form`]). The
//! answer comes from the person's own click in the client, not from the
//! agent's word. With no form to show ([`Answer::NoForm`]) the tools ask as
//! before.
//!
//! Tools run on a blocking thread; the server sets the asker for the call
//! with [`with`], and a tool asks with [`ask`].

use std::cell::RefCell;
use std::sync::Arc;

use serde_json::{Value, json};

/// What the person did with a form.
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    /// They filled it in and sent it: its fields.
    Sent(Value),
    /// They said no to it.
    Declined,
    /// They closed it without answering.
    Closed,
    /// There is no form to show: the client shows none, or the form
    /// couldn't reach it (the reason).
    NoForm(String),
}

/// Puts one form to the person and waits for their answer: its message and
/// its fields' JSON schema (a flat object of plain fields).
pub type Asker = dyn Fn(&str, Value) -> Answer + Send + Sync;

thread_local! {
    static ASKER: RefCell<Option<Arc<Asker>>> = const { RefCell::new(None) };
}

/// Runs `f` with `asker` as this thread's way to ask the person.
pub fn with<R>(asker: Option<Arc<Asker>>, f: impl FnOnce() -> R) -> R {
    let before = ASKER.with(|a| a.replace(asker));
    let out = f();
    ASKER.with(|a| *a.borrow_mut() = before);
    out
}

/// Whether this call's client shows forms.
pub fn can_ask() -> bool {
    ASKER.with(|a| a.borrow().is_some())
}

/// Asks the person with a form, when this call can show one.
pub fn ask(message: &str, schema: Value) -> Answer {
    let asker = ASKER.with(|a| a.borrow().clone());
    match asker {
        Some(a) => a(message, schema),
        None => Answer::NoForm("the client shows no forms".into()),
    }
}

/// The form for the person's yes to one call: the card's own words, and
/// one switch.
pub fn yes_form(says: &str) -> (String, Value) {
    (
        format!("{} Do you say yes to this?", says.trim()),
        json!({
            "type": "object",
            "properties": {
                "yes": {
                    "type": "boolean",
                    "title": "Yes, do it",
                    "description": "On: it runs once, as the words above say. Off, or no: it doesn't run.",
                    "default": false,
                },
            },
            "required": ["yes"],
        }),
    )
}

/// Whether a sent yes form says yes.
pub fn said_yes(fields: &Value) -> bool {
    fields["yes"] == json!(true)
}

/// The form asking for the path of the person's own ROM of `title`.
pub fn rom_form(title: &str) -> (String, Value) {
    (
        format!(
            "This copy needs your own ROM of {title}, which isn't in your ROM folder yet. Where is it on this computer? Game Printer copies it into your ROM folder (gp roms add) and reads the art and sound from it there; nothing from it leaves this computer."
        ),
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "title": "Path to the ROM file",
                    "description": "The full path of the file (a .nes file, or a MAME zip for an arcade game).",
                    "minLength": 1,
                },
            },
            "required": ["path"],
        }),
    )
}

/// The path a sent ROM form gives, trimmed of spaces and quotes.
pub fn sent_path(fields: &Value) -> Option<String> {
    let p = fields["path"]
        .as_str()?
        .trim()
        .trim_matches(|c| c == '"' || c == '\'');
    (!p.is_empty()).then(|| p.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A call asks with the asker it was given, and without one it has no
    /// form; the asker is put back after.
    #[test]
    fn a_call_asks_with_its_own_asker() {
        fn asker(m: &str, s: Value) -> Answer {
            Answer::Sent(json!({"asked": m, "fields": s["required"]}))
        }
        assert!(matches!(ask("q", json!({})), Answer::NoForm(_)));
        let (m, s) = yes_form("Publish it.");
        let got = with(Some(Arc::new(asker)), || ask(&m, s));
        let asked = "Publish it. Do you say yes to this?";
        let want = json!({"asked": asked, "fields": ["yes"]});
        assert_eq!(got, Answer::Sent(want));
        assert!(matches!(ask("q", json!({})), Answer::NoForm(_)));
        assert!(said_yes(&json!({"yes": true})));
        assert!(!said_yes(&json!({"yes": false})));
        assert_eq!(
            sent_path(&json!({"path": " '/r/t.nes' "})).as_deref(),
            Some("/r/t.nes")
        );
        assert_eq!(sent_path(&json!({"path": " "})), None);
    }
}
