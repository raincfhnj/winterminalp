use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TerminalEventKind {
    Output,
    Exited,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TerminalEvent {
    pub pane_id: String,
    pub sequence: u64,
    pub kind: TerminalEventKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TerminalStarted {
    pub pane_id: String,
    pub profile_id: String,
    pub process_id: Option<u32>,
    pub attached: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TerminalProfile {
    pub id: String,
    pub name: String,
    pub program: String,
    pub arguments: Vec<String>,
    pub default_directory: Option<String>,
    pub environment: BTreeMap<String, String>,
    pub icon: String,
    pub theme: String,
    pub cursor_style: String,
    pub is_default: bool,
    pub available: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_event_uses_the_documented_camel_case_contract() {
        let event = TerminalEvent {
            pane_id: "pane-1".to_owned(),
            sequence: 7,
            kind: TerminalEventKind::Exited,
            data: None,
            exit_code: Some(0),
        };

        let json = serde_json::to_value(event).expect("terminal events should serialize");

        assert_eq!(json["paneId"], "pane-1");
        assert_eq!(json["sequence"], 7);
        assert_eq!(json["kind"], "exited");
        assert!(json.get("data").is_none());
        assert_eq!(json["exitCode"], 0);
    }
}
