//! A text Rust builds for the interface (batch spec §F): a stable code the front end translates,
//! the values it fills in, and today's English sentence for a front end that does not know the
//! code. Paths and OS errors travel as params, verbatim: they are never translated.

use std::collections::BTreeMap;
use std::fmt;

use serde::Serialize;

/// On the wire: `{ "code": "msg.hook.notObject", "params": { "path": "…" }, "fallback": "…" }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Message {
    pub code: &'static str,
    pub params: BTreeMap<&'static str, String>,
    pub fallback: String,
}

impl Message {
    pub fn new(code: &'static str, fallback: impl Into<String>) -> Self {
        Self { code, params: BTreeMap::new(), fallback: fallback.into() }
    }

    /// Adds (or replaces) the value of `{key}`.
    pub fn with(mut self, key: &'static str, value: impl Into<String>) -> Self {
        self.params.insert(key, value.into());
        self
    }
}

impl fmt::Display for Message {
    /// The English sentence.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.fallback)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_message_serializes_as_code_params_and_fallback() {
        let m = Message::new("msg.hook.readFailed", r"Can't read C:\x\settings.json: Access is denied.")
            .with("path", r"C:\x\settings.json")
            .with("err", "Access is denied.");
        assert_eq!(
            serde_json::to_value(&m).unwrap(),
            json!({
                "code": "msg.hook.readFailed",
                "params": { "err": "Access is denied.", "path": r"C:\x\settings.json" },
                "fallback": r"Can't read C:\x\settings.json: Access is denied."
            })
        );
        assert_eq!(
            serde_json::to_value(Message::new("msg.hook.coucouRemoved", "x")).unwrap(),
            json!({ "code": "msg.hook.coucouRemoved", "params": {}, "fallback": "x" })
        );
    }

    #[test]
    fn display_is_the_fallback_and_params_are_kept_verbatim() {
        let m = Message::new("msg.codex.shortPathFailed", "Can't resolve: Zugriff verweigert (os error 5)")
            .with("err", "Zugriff verweigert (os error 5)");
        assert_eq!(m.to_string(), "Can't resolve: Zugriff verweigert (os error 5)");
        assert_eq!(m.params["err"], "Zugriff verweigert (os error 5)");
        // A later `with` of the same name replaces the value.
        assert_eq!(m.with("err", "b").params["err"], "b");
    }
}
