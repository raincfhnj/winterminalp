use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use tauri::State;
use tauri::ipc::Channel;

use crate::domain::{
    AppModel, AppSnapshot, Direction, DomainError, LayoutNode, PaneSnapshot, PaneStatus,
};
use crate::error::AppError;
use crate::terminal::{
    TerminalError, TerminalEvent, TerminalEventKind, TerminalManager, TerminalSink, TerminalStarted,
};

#[cfg(test)]
const DEFAULT_PROFILE_ID: &str = "powershell";

pub struct AppState {
    model: Arc<Mutex<AppModel>>,
    terminals: TerminalManager,
}

impl AppState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            model: Arc::new(Mutex::new(AppModel::new())),
            terminals: TerminalManager::new(),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

#[tauri::command]
pub fn get_app_snapshot(state: State<'_, AppState>) -> Result<AppSnapshot, AppError> {
    Ok(lock_model(&state)?.snapshot())
}

#[tauri::command]
pub fn split_pane(
    state: State<'_, AppState>,
    direction: Direction,
) -> Result<AppSnapshot, AppError> {
    lock_model(&state)?
        .split_active(direction)
        .map_err(domain_error)
}

#[tauri::command]
pub fn focus_pane(
    state: State<'_, AppState>,
    direction: Direction,
) -> Result<AppSnapshot, AppError> {
    lock_model(&state)?
        .focus_active(direction)
        .map_err(domain_error)
}

#[tauri::command]
pub fn resize_pane(
    state: State<'_, AppState>,
    direction: Direction,
    amount: f64,
) -> Result<AppSnapshot, AppError> {
    lock_model(&state)?
        .resize_active(direction, amount)
        .map_err(domain_error)
}

#[tauri::command]
pub fn toggle_zoom(state: State<'_, AppState>) -> Result<AppSnapshot, AppError> {
    lock_model(&state)?.toggle_zoom().map_err(domain_error)
}

#[tauri::command]
pub fn create_tab(state: State<'_, AppState>) -> Result<AppSnapshot, AppError> {
    lock_model(&state)?.create_tab().map_err(domain_error)
}

#[tauri::command]
pub fn activate_tab(state: State<'_, AppState>, tab_id: String) -> Result<AppSnapshot, AppError> {
    lock_model(&state)?
        .activate_tab(&tab_id)
        .map_err(domain_error)
}

#[tauri::command]
pub fn close_pane(state: State<'_, AppState>) -> Result<AppSnapshot, AppError> {
    let mutation = lock_model(&state)?
        .close_active_pane()
        .map_err(domain_error)?;
    terminate_removed(&state.terminals, &mutation.removed_pane_ids);
    Ok(mutation.snapshot)
}

#[tauri::command]
pub fn close_tab(state: State<'_, AppState>, tab_id: String) -> Result<AppSnapshot, AppError> {
    let mutation = lock_model(&state)?
        .close_tab(&tab_id)
        .map_err(domain_error)?;
    terminate_removed(&state.terminals, &mutation.removed_pane_ids);
    Ok(mutation.snapshot)
}

#[tauri::command]
pub fn start_terminal(
    state: State<'_, AppState>,
    pane_id: String,
    profile_id: Option<String>,
    cwd: Option<String>,
    rows: u16,
    cols: u16,
    channel: Channel<TerminalEvent>,
) -> Result<TerminalStarted, AppError> {
    let pane_profile_id = pane_profile_id(&lock_model(&state)?.snapshot(), &pane_id)?;
    let requested_profile_id = profile_id.unwrap_or_else(|| pane_profile_id.clone());
    if requested_profile_id != pane_profile_id {
        return Err(AppError::new(
            "INVALID_PROFILE",
            "The requested profile does not match this pane.",
        )
        .for_pane(&pane_id)
        .with_detail(format!(
            "pane profile: {pane_profile_id}; requested profile: {requested_profile_id}"
        )));
    }

    if !state
        .terminals
        .is_running(&pane_id)
        .map_err(terminal_error)?
    {
        set_pane_status(&state.model, &pane_id, PaneStatus::Starting, None)?;
    }

    let sink = terminal_sink(Arc::clone(&state.model), channel);
    let cwd = cwd.map(PathBuf::from);
    let started = match state.terminals.start_or_attach(
        &pane_id,
        &requested_profile_id,
        cwd.as_deref(),
        rows,
        cols,
        sink,
    ) {
        Ok(started) => started,
        Err(error) => {
            let mapped = terminal_start_error(error);
            record_terminal_failure(&state.model, &pane_id, &mapped);
            return Err(mapped);
        }
    };

    if let Err(error) = set_pane_status(&state.model, &pane_id, PaneStatus::Running, None) {
        let _ = state.terminals.terminate(&pane_id);
        return Err(error);
    }
    Ok(started)
}

#[tauri::command]
pub fn write_terminal(
    state: State<'_, AppState>,
    pane_id: String,
    data: String,
) -> Result<(), AppError> {
    ensure_pane_exists(&lock_model(&state)?.snapshot(), &pane_id)?;
    state
        .terminals
        .write(&pane_id, data.as_bytes())
        .map_err(terminal_error)
}

#[tauri::command]
pub fn resize_terminal(
    state: State<'_, AppState>,
    pane_id: String,
    rows: u16,
    cols: u16,
) -> Result<(), AppError> {
    ensure_pane_exists(&lock_model(&state)?.snapshot(), &pane_id)?;
    state
        .terminals
        .resize(&pane_id, rows, cols)
        .map_err(terminal_error)
}

#[tauri::command]
pub fn restart_terminal(
    state: State<'_, AppState>,
    pane_id: String,
    rows: u16,
    cols: u16,
    channel: Channel<TerminalEvent>,
) -> Result<TerminalStarted, AppError> {
    let profile_id = pane_profile_id(&lock_model(&state)?.snapshot(), &pane_id)?;
    if state
        .terminals
        .is_running(&pane_id)
        .map_err(terminal_error)?
    {
        return Err(AppError::new(
            "TERMINAL_ALREADY_RUNNING",
            "The terminal process is already running.",
        )
        .for_pane(&pane_id));
    }
    set_pane_status(&state.model, &pane_id, PaneStatus::Starting, None)?;

    let sink = terminal_sink(Arc::clone(&state.model), channel);
    let restart_result = match state
        .terminals
        .restart(&pane_id, rows, cols, Arc::clone(&sink))
    {
        Err(error) if error.code == "TERMINAL_NOT_RUNNING" => {
            state
                .terminals
                .start_or_attach(&pane_id, &profile_id, None, rows, cols, sink)
        }
        result => result,
    };
    let started = match restart_result {
        Ok(started) => started,
        Err(error) => {
            let mapped = terminal_start_error(error);
            record_terminal_failure(&state.model, &pane_id, &mapped);
            return Err(mapped);
        }
    };

    set_pane_status(&state.model, &pane_id, PaneStatus::Running, None)?;
    Ok(started)
}

fn lock_model(state: &AppState) -> Result<MutexGuard<'_, AppModel>, AppError> {
    state.model.lock().map_err(|_| state_lock_error())
}

fn set_pane_status(
    model: &Mutex<AppModel>,
    pane_id: &str,
    status: PaneStatus,
    message: Option<String>,
) -> Result<(), AppError> {
    model
        .lock()
        .map_err(|_| state_lock_error())?
        .set_pane_status(pane_id, status, message)
        .map(|_| ())
        .map_err(domain_error)
}

fn record_terminal_failure(model: &Mutex<AppModel>, pane_id: &str, error: &AppError) {
    let (status, message) = if error.code == "TERMINAL_ALREADY_RUNNING" {
        (PaneStatus::Running, None)
    } else {
        (PaneStatus::Error, Some(error.message.clone()))
    };
    let _ = set_pane_status(model, pane_id, status, message);
}

fn terminal_sink(model: Arc<Mutex<AppModel>>, channel: Channel<TerminalEvent>) -> TerminalSink {
    Arc::new(move |event| {
        if let Ok(mut model) = model.lock() {
            match event.kind {
                TerminalEventKind::Output => {}
                TerminalEventKind::Exited => {
                    let message = event
                        .exit_code
                        .map(|code| format!("Process exited with code {code}."));
                    let _ = model.set_pane_status(&event.pane_id, PaneStatus::Exited, message);
                }
                TerminalEventKind::Error => {
                    let message = event.data.clone().filter(|data| !data.is_empty());
                    let _ = model.set_pane_status(&event.pane_id, PaneStatus::Error, message);
                }
            }
        }
        let _ = channel.send(event);
    })
}

fn terminate_removed(terminals: &TerminalManager, pane_ids: &[String]) {
    for pane_id in pane_ids {
        if let Err(error) = terminals.terminate(pane_id) {
            eprintln!("failed to terminate PTY for pane {pane_id}: {error}");
        }
    }
}

fn pane_profile_id(snapshot: &AppSnapshot, pane_id: &str) -> Result<String, AppError> {
    snapshot
        .session
        .tabs
        .iter()
        .find_map(|tab| pane_in_layout(&tab.root, pane_id))
        .map(|pane| pane.profile_id.clone())
        .ok_or_else(|| pane_not_found(pane_id))
}

fn ensure_pane_exists(snapshot: &AppSnapshot, pane_id: &str) -> Result<(), AppError> {
    pane_profile_id(snapshot, pane_id).map(|_| ())
}

fn pane_in_layout<'a>(node: &'a LayoutNode, pane_id: &str) -> Option<&'a PaneSnapshot> {
    match node {
        LayoutNode::Pane { pane } if pane.id == pane_id => Some(pane),
        LayoutNode::Pane { .. } => None,
        LayoutNode::Split { first, second, .. } => {
            pane_in_layout(first, pane_id).or_else(|| pane_in_layout(second, pane_id))
        }
    }
}

fn state_lock_error() -> AppError {
    AppError::new(
        "STATE_LOCK_FAILED",
        "Application state is temporarily unavailable.",
    )
    .retryable(true)
}

fn pane_not_found(pane_id: &str) -> AppError {
    AppError::new("PANE_NOT_FOUND", "The requested pane does not exist.").for_pane(pane_id)
}

fn domain_error(error: DomainError) -> AppError {
    match error {
        DomainError::PaneNotFound(pane_id) => pane_not_found(&pane_id),
        DomainError::TabNotFound(tab_id) => AppError::new(
            "TAB_NOT_FOUND",
            format!("The requested tab `{tab_id}` does not exist."),
        ),
        DomainError::InvalidResizeAmount => AppError::new(
            "INVALID_AMOUNT",
            "The resize amount must be a finite number greater than zero.",
        ),
        DomainError::ActiveSessionNotFound | DomainError::ActiveTabNotFound => AppError::new(
            "INTERNAL_ERROR",
            "The active workspace state is inconsistent.",
        )
        .retryable(true),
    }
}

fn terminal_start_error(error: TerminalError) -> AppError {
    let mut mapped = terminal_error(error);
    if mapped.code == "TERMINAL_IO_FAILED" {
        mapped.code = "TERMINAL_START_FAILED".to_owned();
    }
    mapped
}

fn terminal_error(error: TerminalError) -> AppError {
    AppError {
        code: error.code,
        message: error.message,
        retryable: error.retryable,
        pane_id: error.pane_id,
        detail: error.detail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pane_lookup_traverses_the_serialized_layout_tree() {
        let mut model = AppModel::new();
        let snapshot = model
            .split_active(Direction::Right)
            .expect("split should succeed");
        for pane_id in model.pane_ids() {
            assert_eq!(
                pane_profile_id(&snapshot, &pane_id).expect("pane should exist"),
                DEFAULT_PROFILE_ID
            );
        }
        assert_eq!(
            pane_profile_id(&snapshot, "missing")
                .expect_err("unknown pane should fail")
                .code,
            "PANE_NOT_FOUND"
        );
    }

    #[test]
    fn domain_errors_map_to_stable_ipc_codes() {
        assert_eq!(
            domain_error(DomainError::InvalidResizeAmount).code,
            "INVALID_AMOUNT"
        );
        assert_eq!(
            domain_error(DomainError::TabNotFound("tab-1".to_owned())).code,
            "TAB_NOT_FOUND"
        );
        assert_eq!(
            domain_error(DomainError::PaneNotFound("pane-1".to_owned())).code,
            "PANE_NOT_FOUND"
        );
    }
}
