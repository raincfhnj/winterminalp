use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Error, PartialEq, Eq)]
#[error("{message}")]
#[serde(rename_all = "camelCase")]
pub struct TerminalError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl TerminalError {
    pub(crate) fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            message: message.into(),
            retryable: false,
            pane_id: None,
            detail: None,
        }
    }

    pub(crate) fn for_pane(mut self, pane_id: &str) -> Self {
        self.pane_id = Some(pane_id.to_owned());
        self
    }

    pub(crate) fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub(crate) fn retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }

    pub(crate) fn invalid_pane_id(reason: impl Into<String>) -> Self {
        Self::new("PANE_NOT_FOUND", "The pane ID is invalid.").with_detail(reason)
    }

    pub(crate) fn unknown_profile(profile_id: &str) -> Self {
        Self::new(
            "INVALID_PROFILE",
            format!("Unknown terminal profile `{profile_id}`."),
        )
    }

    pub(crate) fn profile_unavailable(profile_id: &str) -> Self {
        Self::new(
            "INVALID_PROFILE",
            format!("Terminal profile `{profile_id}` is not available on this computer."),
        )
    }

    pub(crate) fn profile_mismatch(
        pane_id: &str,
        running_profile_id: &str,
        requested_profile_id: &str,
    ) -> Self {
        Self::new(
            "TERMINAL_ALREADY_RUNNING",
            "The pane is already running a different terminal profile.",
        )
        .for_pane(pane_id)
        .with_detail(format!(
            "running profile: {running_profile_id}; requested profile: {requested_profile_id}"
        ))
    }

    pub(crate) fn invalid_size(rows: u16, cols: u16, max_dimension: u16) -> Self {
        Self::new(
            "INVALID_TERMINAL_SIZE",
            "The terminal dimensions are outside the supported range.",
        )
        .with_detail(format!(
            "rows and columns must be between 1 and {max_dimension}; received {rows}x{cols}"
        ))
    }

    pub(crate) fn not_running(pane_id: &str) -> Self {
        Self::new(
            "TERMINAL_NOT_RUNNING",
            "The terminal process is not running.",
        )
        .for_pane(pane_id)
        .retryable(true)
    }

    pub(crate) fn already_running(pane_id: &str) -> Self {
        Self::new(
            "TERMINAL_ALREADY_RUNNING",
            "The terminal process is already running.",
        )
        .for_pane(pane_id)
    }

    pub(crate) fn state_unavailable(resource: &str, pane_id: Option<&str>) -> Self {
        let error = Self::new(
            "STATE_LOCK_FAILED",
            "Terminal state is temporarily unavailable.",
        )
        .with_detail(format!("the {resource} lock was poisoned"))
        .retryable(true);

        match pane_id {
            Some(pane_id) => error.for_pane(pane_id),
            None => error,
        }
    }

    pub(crate) fn start_failed(
        operation: &str,
        pane_id: Option<&str>,
        detail: impl Into<String>,
    ) -> Self {
        let error = Self::new(
            "TERMINAL_START_FAILED",
            format!("Terminal startup operation `{operation}` failed."),
        )
        .with_detail(detail)
        .retryable(true);

        match pane_id {
            Some(pane_id) => error.for_pane(pane_id),
            None => error,
        }
    }

    pub(crate) fn io_failed(
        operation: &str,
        pane_id: Option<&str>,
        detail: impl Into<String>,
    ) -> Self {
        let error = Self::new(
            "TERMINAL_IO_FAILED",
            format!("Terminal I/O operation `{operation}` failed."),
        )
        .with_detail(detail)
        .retryable(true);

        match pane_id {
            Some(pane_id) => error.for_pane(pane_id),
            None => error,
        }
    }

    pub(crate) fn worker_failed(pane_id: &str, detail: impl Into<String>) -> Self {
        Self::new(
            "TERMINAL_START_FAILED",
            "A terminal worker thread could not be started.",
        )
        .for_pane(pane_id)
        .with_detail(detail)
        .retryable(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_like_the_shared_application_error() {
        let error = TerminalError::not_running("pane-1");
        let json = serde_json::to_value(error).expect("terminal errors should serialize");

        assert_eq!(json["code"], "TERMINAL_NOT_RUNNING");
        assert_eq!(json["paneId"], "pane-1");
        assert_eq!(json["retryable"], true);
        assert!(json.get("detail").is_none());
    }
}
