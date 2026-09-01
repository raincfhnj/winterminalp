use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Up => "up",
            Self::Down => "down",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TerminalAction {
    SendPrefixLiteral,
    SplitPane { direction: Direction },
    FocusPane { direction: Direction },
    ResizePane { direction: Direction },
    NewTab,
    NextTab,
    PreviousTab,
    ActivateTab { index: u8 },
    ClosePane,
    TogglePaneZoom,
    RenameTab,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TerminalChannel {
    Stable,
    Preview,
    Canary,
    Unpackaged,
    Portable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowIdentity {
    pub hwnd: isize,
    pub process_id: u32,
    pub process_started_at_100ns: u64,
    pub channel: TerminalChannel,
}
