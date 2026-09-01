use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread::{self, JoinHandle};

use crate::keymap::binding_for_action;
use crate::model::{Direction, TerminalAction, WindowIdentity};
use crate::pane_layout::ScreenPoint;
use crate::platform::windows::{TerminalAccessibility, send_bridge_chord};
use crate::{AppError, AppResult};

pub(super) enum WorkerMessage {
    Dispatch {
        target: WindowIdentity,
        action: TerminalAction,
    },
    ResizePaneByPointer {
        target: WindowIdentity,
        focus_point: ScreenPoint,
        direction: Direction,
        steps: u8,
    },
    Stop,
}

#[derive(Default)]
pub(super) struct WorkerReport {
    pub(super) dispatched_actions: u64,
    pub(super) failed_actions: u64,
    pub(super) last_dispatch_error: Option<String>,
}

pub(super) struct ActionWorker {
    sender: SyncSender<WorkerMessage>,
    join: Option<JoinHandle<AppResult<WorkerReport>>>,
}

impl ActionWorker {
    pub(super) fn start(capacity: usize) -> AppResult<Self> {
        let (sender, receiver) = mpsc::sync_channel(capacity);
        let join = thread::Builder::new()
            .name("winterminal-action-worker".to_owned())
            .spawn(move || Ok(run(receiver)))
            .map_err(|error| {
                AppError::Native(format!("failed to spawn action worker thread: {error}"))
            })?;
        Ok(Self {
            sender,
            join: Some(join),
        })
    }

    pub(super) fn sender(&self) -> SyncSender<WorkerMessage> {
        self.sender.clone()
    }

    pub(super) fn stop(mut self) -> AppResult<WorkerReport> {
        let _ = self.sender.send(WorkerMessage::Stop);
        let Some(join) = self.join.take() else {
            return Ok(WorkerReport::default());
        };
        join.join()
            .map_err(|_| AppError::Native("action worker thread panicked".to_owned()))?
    }
}

impl Drop for ActionWorker {
    fn drop(&mut self) {
        if let Some(join) = self.join.take() {
            let _ = self.sender.send(WorkerMessage::Stop);
            let _ = join.join();
        }
    }
}

fn run(receiver: Receiver<WorkerMessage>) -> WorkerReport {
    let mut report = WorkerReport::default();
    let mut accessibility = None;
    while let Ok(message) = receiver.recv() {
        let result = match message {
            WorkerMessage::Dispatch { target, action } => dispatch_action(target, action),
            WorkerMessage::ResizePaneByPointer {
                target,
                focus_point,
                direction,
                steps,
            } => dispatch_pointer_resize(&mut accessibility, target, focus_point, direction, steps),
            WorkerMessage::Stop => break,
        };

        match result {
            Ok(dispatched) => report.dispatched_actions += dispatched,
            Err(error) => {
                report.failed_actions += 1;
                report.last_dispatch_error = Some(error);
            }
        }
    }
    report
}

fn dispatch_action(target: WindowIdentity, action: TerminalAction) -> Result<u64, String> {
    let binding = binding_for_action(action)
        .ok_or_else(|| format!("no bridge binding exists for {action:?}"))?;
    send_bridge_chord(target, binding.bridge_chord)
        .map(|_| 1)
        .map_err(|error| error.to_string())
}

fn dispatch_pointer_resize(
    accessibility: &mut Option<TerminalAccessibility>,
    target: WindowIdentity,
    focus_point: ScreenPoint,
    direction: Direction,
    steps: u8,
) -> Result<u64, String> {
    if steps == 0 {
        return Ok(0);
    }
    if accessibility.is_none() {
        *accessibility =
            Some(TerminalAccessibility::initialize().map_err(|error| error.to_string())?);
    }
    accessibility
        .as_ref()
        .expect("accessibility is initialized above")
        .focus_pane_at(target, focus_point)
        .map_err(|error| error.to_string())?;

    let action = TerminalAction::ResizePane { direction };
    let binding = binding_for_action(action)
        .ok_or_else(|| format!("no bridge binding exists for {action:?}"))?;
    for _ in 0..steps {
        send_bridge_chord(target, binding.bridge_chord).map_err(|error| error.to_string())?;
    }
    Ok(u64::from(steps))
}
