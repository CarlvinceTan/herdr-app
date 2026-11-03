//! Agents come back after their terminal is lost (a reboot, a core crash).
//!
//! The core gives a kept terminal that lost its program a shell in its last
//! folder, never the old program. So while an agent runs, Herdr notes it on its
//! terminal as a core label (labels outlive the program): which agent, and the
//! shell it ran in. A terminal whose shell changed under a noted agent was lost,
//! and the agent's resume command (`programs.json`) is typed into the new shell.
//! An agent that says which conversation it's in (the `herdr.session` label, from
//! Claude Code's session hook) comes back to exactly that one.

use serde_json::{Value, json};

/// What runs, as `{"key": "claude", "shell": <pid of the terminal's program>}`.
pub const NOTE: &str = "herdr.relaunch";
/// The agent's own id for its conversation, set by the agent (a hook).
pub const SESSION: &str = "herdr.session";

#[derive(Debug, PartialEq)]
pub enum Step {
    /// Note this label value: an agent runs in front of this shell.
    Note(String),
    /// The agent was quit, or something else runs now: drop the note.
    Forget,
    /// The shell is new and the agent is gone: type this line, noted as `note`.
    Start { line: String, note: String },
}

/// What to do for one listed terminal (as `terminals_listed` keeps it: with
/// `program`, `labels`, `keep`, `pid`). `None`: nothing changes.
pub fn step(rec: &Value) -> Option<Step> {
    if rec["state"] != "live" || rec["keep"] != true {
        return None;
    }
    let pid = rec["pid"].as_u64()?;
    let program = &rec["program"];
    let key = program["key"].as_str().unwrap_or_default();
    let noted = rec.pointer(&format!("/labels/{NOTE}")).and_then(Value::as_str).and_then(|s| serde_json::from_str::<Value>(s).ok());
    let noted_key = noted.as_ref().and_then(|n| n["key"].as_str()).unwrap_or_default().to_owned();
    let noted_shell = noted.as_ref().and_then(|n| n["shell"].as_u64());
    let note = |key: &str| json!({ "key": key, "shell": pid }).to_string();

    if program["agent"] == true {
        if key.is_empty() || (noted_key == key && noted_shell == Some(pid)) {
            return None;
        }
        return Some(Step::Note(note(key)));
    }
    noted.as_ref()?;
    if program["shell"] != true {
        // The noted agent starting (it hasn't said it's an agent yet) stays noted.
        return (key != noted_key).then_some(Step::Forget);
    }
    if noted_shell == Some(pid) {
        return Some(Step::Forget);
    }
    let id = rec.pointer(&format!("/labels/{SESSION}")).and_then(Value::as_str).filter(|id| plain(id));
    let argv = crate::programs::resume(&noted_key, id)?;
    Some(Step::Start { line: argv.iter().map(|a| word(a)).collect::<Vec<_>>().join(" "), note: note(&noted_key) })
}

/// An id that can go on a command line as it is.
fn plain(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
}

/// One word as a shell reads it: bare when plain, else single-quoted.
fn word(a: &str) -> String {
    if !a.is_empty() && a.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+=,:@%~".contains(c)) {
        a.to_owned()
    } else {
        format!("'{}'", a.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(program: Value, pid: u64, labels: Value) -> Value {
        json!({ "state": "live", "keep": true, "pid": pid, "program": program, "labels": labels })
    }
    fn claude() -> Value { json!({ "key": "claude", "agent": true, "shell": false }) }
    fn shell() -> Value { json!({ "key": "shell", "agent": false, "shell": true }) }
    fn noted(shell: u64) -> String { json!({ "key": "claude", "shell": shell }).to_string() }

    #[test]
    fn a_running_agent_is_noted_once() {
        assert_eq!(step(&rec(claude(), 7, json!({}))), Some(Step::Note(noted(7))));
        assert_eq!(step(&rec(claude(), 7, json!({ NOTE: noted(7) }))), None);
    }

    #[test]
    fn quitting_in_the_same_shell_forgets_it() {
        assert_eq!(step(&rec(shell(), 7, json!({ NOTE: noted(7) }))), Some(Step::Forget));
        assert_eq!(step(&rec(shell(), 7, json!({}))), None);
    }

    #[test]
    fn a_new_shell_starts_the_agent_again_in_its_conversation() {
        let lost = rec(shell(), 9, json!({ NOTE: noted(7), SESSION: "4f1c-aa" }));
        assert_eq!(step(&lost), Some(Step::Start { line: "claude --resume 4f1c-aa".into(), note: noted(9) }));
        let unsaid = rec(shell(), 9, json!({ NOTE: noted(7) }));
        assert_eq!(step(&unsaid), Some(Step::Start { line: "claude --continue".into(), note: noted(9) }));
        // An id that isn't plain never reaches the command line.
        let odd = rec(shell(), 9, json!({ NOTE: noted(7), SESSION: "x; rm -rf ~" }));
        assert_eq!(step(&odd), Some(Step::Start { line: "claude --continue".into(), note: noted(9) }));
    }

    #[test]
    fn something_else_in_front_forgets_it_but_the_agent_starting_does_not() {
        let other = json!({ "key": "vim", "agent": false, "shell": false });
        assert_eq!(step(&rec(other, 9, json!({ NOTE: noted(7) }))), Some(Step::Forget));
        let starting = json!({ "key": "claude", "agent": false, "shell": false });
        assert_eq!(step(&rec(starting, 9, json!({ NOTE: noted(9) }))), None);
    }

    #[test]
    fn only_kept_live_terminals_with_a_program() {
        let mut r = rec(claude(), 7, json!({}));
        r["keep"] = json!(false);
        assert_eq!(step(&r), None);
        let mut r = rec(claude(), 7, json!({}));
        r["pid"] = Value::Null;
        assert_eq!(step(&r), None);
    }

    #[test]
    fn words_are_quoted_when_needed() {
        assert_eq!(word("--continue"), "--continue");
        assert_eq!(word("it's"), "'it'\\''s'");
    }
}
