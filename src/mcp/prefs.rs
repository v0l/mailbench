use crate::store::Store;
use serde::{Deserialize, Serialize};

const PREFS_ID: &str = "prefs";

fn default_port() -> u16 {
    super::DEFAULT_PORT
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentPrefs {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default)]
    pub allowed: Vec<String>,
    #[serde(default)]
    pub send: Vec<String>,
    #[serde(default)]
    pub nostr_send: bool,
    #[serde(default)]
    pub plain_text_only: bool,
}

impl Default for AgentPrefs {
    fn default() -> Self {
        AgentPrefs { enabled: false, port: super::DEFAULT_PORT, allowed: Vec::new(), send: Vec::new(), nostr_send: false, plain_text_only: false }
    }
}

impl AgentPrefs {
    pub fn allows(&self, account_id: &str) -> bool {
        self.allowed.iter().any(|a| a == account_id)
    }

    pub fn may_send(&self, account_id: &str, nostr: bool) -> bool {
        self.send.iter().any(|a| a == account_id) && (!nostr || self.nostr_send)
    }

    pub fn set_allowed(&mut self, account_id: &str, on: bool) {
        self.allowed.retain(|a| a != account_id);
        if on {
            self.allowed.push(account_id.to_string());
        } else {
            self.send.retain(|a| a != account_id);
        }
    }

    pub fn set_send(&mut self, account_id: &str, on: bool) {
        self.send.retain(|a| a != account_id);
        if on {
            self.send.push(account_id.to_string());
        }
    }
}

pub fn load(store: &Store) -> AgentPrefs {
    let mut prefs: AgentPrefs =
        store.get(super::token::KIND, PREFS_ID).ok().flatten().and_then(|raw| serde_json::from_slice(&raw).ok()).unwrap_or_default();
    prefs.plain_text_only = crate::prefs::load(store).plain_text_only;
    prefs
}

pub fn save(store: &Store, prefs: &AgentPrefs) -> Result<(), String> {
    let raw = serde_json::to_vec(prefs).map_err(|e| e.to_string())?;
    store.put(super::token::KIND, PREFS_ID, &raw).map_err(|e| e.to_string())
}
