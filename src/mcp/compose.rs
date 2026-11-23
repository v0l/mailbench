use super::mail::{MailTools, Res};
use super::prefs::AgentPrefs;
use serde_json::Value;

pub fn definitions() -> Vec<Value> {
    Vec::new()
}

impl MailTools {
    pub(super) fn create_draft(&self, _prefs: &AgentPrefs, _args: &Value) -> Res<Value> {
        Err("create_draft is not available yet".into())
    }

    pub(super) fn send_message(&self, _prefs: &AgentPrefs, _args: &Value) -> Res<Value> {
        Err("send_message is not available yet".into())
    }
}
