use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::integration::{ChangeStatus, DoctorReport};
use crate::keymap::managed_bindings;
use crate::{AppError, AppResult, ControllerConfig};

#[cfg(target_os = "windows")]
mod action_worker;
#[cfg(target_os = "windows")]
mod desktop;
#[cfg(target_os = "windows")]
mod keyboard;
#[cfg(target_os = "windows")]
mod pointer;

const DEFAULT_FOREGROUND_POLL_INTERVAL: Duration = Duration::from_millis(25);
const DEFAULT_ACTION_QUEUE_CAPACITY: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControllerOptions {
    /// Whether the native Windows Terminal should be opened before listening.
    pub launch_terminal: bool,
    /// Set only after integration diagnostics confirm that the action bridge is installed.
    pub bridge_ready: bool,
    pub foreground_poll_interval: Duration,
    pub action_queue_capacity: usize,
}

impl Default for ControllerOptions {
    fn default() -> Self {
        Self {
            launch_terminal: false,
            bridge_ready: false,
            foreground_poll_interval: DEFAULT_FOREGROUND_POLL_INTERVAL,
            action_queue_capacity: DEFAULT_ACTION_QUEUE_CAPACITY,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ControllerRunReport {
    pub dispatched_actions: u64,
    pub failed_actions: u64,
    pub dropped_actions: u64,
    pub last_dispatch_error: Option<String>,
    pub pane_geometry_errors: u64,
    pub last_pane_geometry_error: Option<String>,
}

/// Returns true only when every discovered Terminal channel has the complete,
/// conflict-free action bridge required before the hook may consume Prefix keys.
#[must_use]
pub fn bridge_is_ready(report: &DoctorReport) -> bool {
    let expected_bindings = managed_bindings().len();
    report.healthy
        && report.fragment.status == ChangeStatus::Unchanged
        && !report.targets.is_empty()
        && report.targets.iter().all(|target| {
            target.initialized
                && target.readable
                && target.valid_jsonc
                && target.conflicts.is_empty()
                && target.managed_binding_count == expected_bindings
        })
}

#[cfg(target_os = "windows")]
mod implementation {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc;
    use std::sync::{Arc, RwLock};
    use std::time::{Duration, Instant};

    use crate::model::{TerminalAction, WindowIdentity};
    use crate::pane_layout::{PaneDivider, ScreenPoint};
    use crate::platform::windows::{
        HookDecision, InputHook, MouseEventKind, PlatformError, RawInputEvent, SingleInstanceGuard,
        enable_per_monitor_dpi_awareness, foreground_hwnd, is_current_process_elevated,
        launch_windows_terminal, show_pane_resize_cursor,
    };
    use crate::prefix::{
        CancelReason, KeyDisposition, KeyTransition, PrefixCommand, PrefixMachine,
    };

    use super::action_worker::{ActionWorker, WorkerMessage};
    use super::desktop::{DesktopCache, DesktopSnapshot};
    use super::keyboard::KeyboardNormalizer;
    use super::pointer::PointerDragState;
    use super::{AppError, AppResult, ControllerConfig, ControllerOptions, ControllerRunReport};

    const MIN_FOREGROUND_POLL_INTERVAL: Duration = Duration::from_millis(5);
    const MAX_FOREGROUND_POLL_INTERVAL: Duration = Duration::from_secs(1);
    const MAX_ACTION_QUEUE_CAPACITY: usize = 1_024;

    pub(super) fn run(
        config: &ControllerConfig,
        options: ControllerOptions,
    ) -> AppResult<ControllerRunReport> {
        enable_per_monitor_dpi_awareness();
        validate_options(options)?;
        if !is_current_process_elevated().map_err(map_platform_error)? {
            return Err(AppError::ControllerRequiresElevation);
        }
        if !options.bridge_ready {
            return Err(AppError::InvalidConfiguration(
                "Windows Terminal action bridge is not ready; run `wter doctor` and `wter install` first"
                    .to_owned(),
            ));
        }

        let _single_instance = SingleInstanceGuard::acquire().map_err(map_platform_error)?;
        if options.launch_terminal {
            let _child = launch_windows_terminal().map_err(map_platform_error)?;
        }

        let desktop_cache =
            DesktopCache::start(options.foreground_poll_interval, config.mouse_resize)?;
        let cached_desktop = desktop_cache.shared();
        let dropped_actions = Arc::new(AtomicU64::new(0));
        let dropped_actions_for_hook = Arc::clone(&dropped_actions);

        let action_worker = ActionWorker::start(options.action_queue_capacity)?;
        let worker_sender = action_worker.sender();
        let (shutdown_sender, shutdown_receiver) = mpsc::sync_channel(1);

        let mut normalizer = KeyboardNormalizer::default();
        let prefix_runtime = config.prefix_config()?;
        let prefix_chord = prefix_runtime.prefix_chord;
        let mut prefix = PrefixMachine::new(prefix_runtime);
        let mut pending_shutdown_key = None;
        let mouse_resize_enabled = config.mouse_resize.enabled;
        let divider_hit_slop_pixels = i32::from(config.mouse_resize.divider_hit_slop_px);
        let mut pointer_drag = PointerDragState::default();
        let hook = InputHook::start(
            Box::new(move |raw_event| match raw_event {
                RawInputEvent::Keyboard(raw_event) => {
                    let terminal = cached_terminal_for_hook(&cached_desktop);
                    let event = normalizer.normalize(raw_event, terminal);
                    let outcome = prefix.handle_key_event(event, Instant::now());

                    if let Some(command) = outcome.command {
                        match command {
                            PrefixCommand::Dispatch { target, action } => {
                                let message = if action == TerminalAction::SendPrefixLiteral {
                                    WorkerMessage::SendLiteralPrefix {
                                        target,
                                        chord: prefix_chord,
                                    }
                                } else {
                                    WorkerMessage::Dispatch { target, action }
                                };
                                if worker_sender.try_send(message).is_err() {
                                    dropped_actions_for_hook.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                            PrefixCommand::Shutdown => {
                                pending_shutdown_key = Some(event.physical_key);
                            }
                        }
                    }

                    if pending_shutdown_key == Some(event.physical_key)
                        && event.transition == KeyTransition::Up
                    {
                        pending_shutdown_key = None;
                        let _ = shutdown_sender.try_send(());
                    }

                    match outcome.disposition {
                        KeyDisposition::PassThrough => HookDecision::Pass,
                        KeyDisposition::Consume => HookDecision::Consume,
                    }
                }
                RawInputEvent::Mouse(raw_event) => {
                    let point = ScreenPoint::new(raw_event.x, raw_event.y);
                    match raw_event.kind {
                        MouseEventKind::LeftDown => {
                            let _ = prefix.cancel(CancelReason::PointerInput);
                            let divider =
                                divider_for_hook(&cached_desktop, point, divider_hit_slop_pixels);
                            let decision = pointer_drag.begin(divider, point);
                            if let Some(axis) = decision.cursor_axis() {
                                let _ = show_pane_resize_cursor(axis);
                            }
                            hook_decision(decision.consumes())
                        }
                        MouseEventKind::Move => {
                            let decision = pointer_drag.move_to(foreground_hwnd(), point);
                            if let Some(resize) = decision.resize() {
                                let message = WorkerMessage::ResizePaneByPointer {
                                    target: resize.target,
                                    focus_point: resize.intent.focus_point,
                                    direction: resize.intent.direction,
                                    steps: resize.intent.steps,
                                    drag_sequence: resize.sequence,
                                };
                                if worker_sender.try_send(message).is_err() {
                                    dropped_actions_for_hook.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                            if let Some(axis) = decision.cursor_axis() {
                                let _ = show_pane_resize_cursor(axis);
                            }
                            // Consuming a move freezes the hardware cursor, and a
                            // frozen cursor stops reporting cumulative positions,
                            // so the drag delta would never advance. The initial
                            // button-down was already consumed, so passing moves
                            // through cannot start a text selection in the target.
                            HookDecision::Pass
                        }
                        MouseEventKind::LeftUp => hook_decision(pointer_drag.end().consumes()),
                    }
                }
            }),
            mouse_resize_enabled,
        );
        let hook = match hook {
            Ok(hook) => hook,
            Err(error) => {
                let _ = action_worker.stop();
                return Err(map_platform_error(error));
            }
        };

        shutdown_receiver.recv().map_err(|_| {
            AppError::Native("controller shutdown channel disconnected unexpectedly".to_owned())
        })?;

        let hook_result = hook.stop().map_err(map_platform_error);
        let cache_result = desktop_cache.stop();
        let worker_result = action_worker.stop();

        hook_result?;
        let cache_report = cache_result?;
        let report = worker_result?;

        Ok(ControllerRunReport {
            dispatched_actions: report.dispatched_actions,
            failed_actions: report.failed_actions,
            dropped_actions: dropped_actions.load(Ordering::Relaxed),
            last_dispatch_error: report.last_dispatch_error,
            pane_geometry_errors: cache_report.pane_geometry_errors,
            last_pane_geometry_error: cache_report.last_pane_geometry_error,
        })
    }

    fn validate_options(options: ControllerOptions) -> AppResult<()> {
        if !(MIN_FOREGROUND_POLL_INTERVAL..=MAX_FOREGROUND_POLL_INTERVAL)
            .contains(&options.foreground_poll_interval)
        {
            return Err(AppError::InvalidConfiguration(format!(
                "foreground poll interval must be between {} and {} milliseconds",
                MIN_FOREGROUND_POLL_INTERVAL.as_millis(),
                MAX_FOREGROUND_POLL_INTERVAL.as_millis()
            )));
        }
        if !(1..=MAX_ACTION_QUEUE_CAPACITY).contains(&options.action_queue_capacity) {
            return Err(AppError::InvalidConfiguration(format!(
                "action queue capacity must be between 1 and {MAX_ACTION_QUEUE_CAPACITY}"
            )));
        }
        Ok(())
    }

    fn map_platform_error(error: PlatformError) -> AppError {
        if matches!(&error, PlatformError::AlreadyRunning) {
            AppError::ControllerAlreadyRunning
        } else {
            AppError::Native(error.to_string())
        }
    }

    const fn hook_decision(consumes: bool) -> HookDecision {
        if consumes {
            HookDecision::Consume
        } else {
            HookDecision::Pass
        }
    }

    fn cached_terminal_for_hook(cache: &RwLock<DesktopSnapshot>) -> Option<WindowIdentity> {
        let current_hwnd = foreground_hwnd();
        let cached = cache.try_read().ok().and_then(|guard| guard.terminal);
        cached.filter(|identity| identity.hwnd == current_hwnd)
    }

    fn divider_for_hook(
        cache: &RwLock<DesktopSnapshot>,
        point: ScreenPoint,
        hit_slop_pixels: i32,
    ) -> Option<(WindowIdentity, PaneDivider)> {
        let current_hwnd = foreground_hwnd();
        let snapshot = cache.try_read().ok()?;
        let target = snapshot
            .terminal
            .filter(|identity| identity.hwnd == current_hwnd)?;
        let divider = snapshot.pane_layout.divider_at(point, hit_slop_pixels)?;
        Some((target, divider))
    }
}

pub fn run_controller(
    config: &ControllerConfig,
    options: ControllerOptions,
) -> AppResult<ControllerRunReport> {
    #[cfg(target_os = "windows")]
    {
        implementation::run(config, options)
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (config, options);
        Err(AppError::UnsupportedPlatform)
    }
}
