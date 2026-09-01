use std::mem::size_of;

use windows::Win32::Foundation::{GetLastError, SetLastError, WIN32_ERROR};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT,
    KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY, VK_CONTROL, VK_F1, VK_LCONTROL, VK_LMENU, VK_LSHIFT,
    VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
};

use crate::keymap::BridgeChord;
use crate::model::WindowIdentity;

use super::error::{ModifierKey, PlatformError, PlatformResult};
use super::foreground::{foreground_hwnd, validate_window_identity};
use super::hook::CONTROLLER_INPUT_MARKER;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputDispatch {
    pub sent: u32,
    pub synthesized_modifiers: u8,
}

/// Sends a managed Windows Terminal bridge chord to an unchanged foreground
/// target.
///
/// This function never activates a window. It revalidates HWND, PID, process
/// creation time, and channel immediately before `SendInput`. Modifiers already
/// held by the user are never released; extra held modifiers fail closed.
pub fn send_bridge_chord(
    target: WindowIdentity,
    chord: BridgeChord,
) -> PlatformResult<InputDispatch> {
    let snapshot = ModifierSnapshot::capture(chord.function_key)?;
    let plan = plan_bridge_events(chord, snapshot)?;

    let actual_foreground = foreground_hwnd();
    if actual_foreground != target.hwnd {
        return Err(PlatformError::TargetNotForeground {
            expected: target.hwnd,
            actual: actual_foreground,
        });
    }
    validate_window_identity(target)?;

    let inputs = plan.to_inputs();
    let expected = input_count(inputs.len())?;
    let input_size = input_structure_size()?;
    // SAFETY: setting and immediately reading the calling thread's last-error
    // value does not dereference pointers or transfer ownership.
    unsafe { SetLastError(WIN32_ERROR(0)) };
    // SAFETY: `inputs` is a live contiguous INPUT slice and `input_size` is the
    // checked size of INPUT expected by SendInput.
    let sent = unsafe { SendInput(&inputs, input_size) };
    if sent != expected {
        // SAFETY: this reads the calling thread's last-error value immediately
        // after the failed/partial SendInput call.
        let os_error = unsafe { GetLastError() }.0;
        let cleanup = plan.cleanup_after(sent as usize);
        let cleanup_inputs = events_to_inputs(&cleanup);
        let cleanup_expected = input_count(cleanup_inputs.len())?;
        let cleanup_sent = if cleanup_inputs.is_empty() {
            0
        } else {
            // SAFETY: this only resets the calling thread's last-error value.
            unsafe { SetLastError(WIN32_ERROR(0)) };
            // SAFETY: `cleanup_inputs` is a live contiguous INPUT slice and the
            // structure size was checked above.
            unsafe { SendInput(&cleanup_inputs, input_size) }
        };

        return Err(PlatformError::InputInjectionIncomplete {
            sent,
            expected,
            cleanup_sent,
            cleanup_expected,
            os_error,
        });
    }

    Ok(InputDispatch {
        sent,
        synthesized_modifiers: plan.synthesized_modifiers,
    })
}

fn input_count(count: usize) -> PlatformResult<u32> {
    u32::try_from(count).map_err(|_| PlatformError::InputSequenceTooLarge(count))
}

fn input_structure_size() -> PlatformResult<i32> {
    let size = size_of::<INPUT>();
    i32::try_from(size).map_err(|_| PlatformError::InputStructureSizeTooLarge(size))
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct ModifierSnapshot {
    control: bool,
    alt: bool,
    shift: bool,
    windows: bool,
    function_key: bool,
}

impl ModifierSnapshot {
    fn capture(function_key: u8) -> PlatformResult<Self> {
        let function_key = function_virtual_key(function_key)?;
        Ok(Self {
            control: key_is_down(VK_CONTROL),
            alt: key_is_down(VK_MENU),
            shift: key_is_down(VK_SHIFT),
            windows: key_is_down(VK_LWIN) || key_is_down(VK_RWIN),
            function_key: key_is_down(function_key),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PlannedKeyEvent {
    virtual_key: VIRTUAL_KEY,
    transition: KeyTransition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyTransition {
    Down,
    Up,
}

#[derive(Debug, PartialEq, Eq)]
struct InputPlan {
    events: Vec<PlannedKeyEvent>,
    synthesized_modifiers: u8,
}

impl InputPlan {
    fn to_inputs(&self) -> Vec<INPUT> {
        events_to_inputs(&self.events)
    }

    fn cleanup_after(&self, sent: usize) -> Vec<PlannedKeyEvent> {
        let mut down = Vec::with_capacity(4);
        for event in self.events.iter().take(sent.min(self.events.len())) {
            match event.transition {
                KeyTransition::Down => {
                    if !down.contains(&event.virtual_key) {
                        down.push(event.virtual_key);
                    }
                }
                KeyTransition::Up => {
                    if let Some(index) = down
                        .iter()
                        .rposition(|virtual_key| *virtual_key == event.virtual_key)
                    {
                        down.remove(index);
                    }
                }
            }
        }

        down.into_iter()
            .rev()
            .map(|virtual_key| PlannedKeyEvent {
                virtual_key,
                transition: KeyTransition::Up,
            })
            .collect()
    }
}

fn plan_bridge_events(chord: BridgeChord, snapshot: ModifierSnapshot) -> PlatformResult<InputPlan> {
    let function_key = function_virtual_key(chord.function_key)?;
    if snapshot.function_key {
        return Err(PlatformError::FunctionKeyHeld(chord.function_key));
    }
    if snapshot.windows {
        return Err(PlatformError::UnexpectedModifierHeld(ModifierKey::Windows));
    }

    let mut events = Vec::with_capacity(8);
    let mut synthesized = Vec::with_capacity(3);
    plan_modifier(
        chord.ctrl,
        snapshot.control,
        ModifierKey::Control,
        VK_LCONTROL,
        &mut events,
        &mut synthesized,
    )?;
    plan_modifier(
        chord.alt,
        snapshot.alt,
        ModifierKey::Alt,
        VK_LMENU,
        &mut events,
        &mut synthesized,
    )?;
    plan_modifier(
        chord.shift,
        snapshot.shift,
        ModifierKey::Shift,
        VK_LSHIFT,
        &mut events,
        &mut synthesized,
    )?;

    events.push(PlannedKeyEvent {
        virtual_key: function_key,
        transition: KeyTransition::Down,
    });
    events.push(PlannedKeyEvent {
        virtual_key: function_key,
        transition: KeyTransition::Up,
    });
    events.extend(
        synthesized
            .iter()
            .rev()
            .copied()
            .map(|virtual_key| PlannedKeyEvent {
                virtual_key,
                transition: KeyTransition::Up,
            }),
    );

    Ok(InputPlan {
        events,
        synthesized_modifiers: synthesized.len() as u8,
    })
}

fn plan_modifier(
    required: bool,
    held: bool,
    name: ModifierKey,
    virtual_key: VIRTUAL_KEY,
    events: &mut Vec<PlannedKeyEvent>,
    synthesized: &mut Vec<VIRTUAL_KEY>,
) -> PlatformResult<()> {
    match (required, held) {
        (false, true) => Err(PlatformError::UnexpectedModifierHeld(name)),
        (true, false) => {
            events.push(PlannedKeyEvent {
                virtual_key,
                transition: KeyTransition::Down,
            });
            synthesized.push(virtual_key);
            Ok(())
        }
        _ => Ok(()),
    }
}

fn function_virtual_key(function_key: u8) -> PlatformResult<VIRTUAL_KEY> {
    if !(13..=24).contains(&function_key) {
        return Err(PlatformError::InvalidBridgeFunctionKey(function_key));
    }

    Ok(VIRTUAL_KEY(VK_F1.0 + u16::from(function_key - 1)))
}

fn key_is_down(virtual_key: VIRTUAL_KEY) -> bool {
    // SAFETY: GetAsyncKeyState accepts any virtual-key code by value and has no
    // pointer or ownership requirements.
    unsafe { GetAsyncKeyState(i32::from(virtual_key.0)) < 0 }
}

fn events_to_inputs(events: &[PlannedKeyEvent]) -> Vec<INPUT> {
    events.iter().copied().map(event_to_input).collect()
}

fn event_to_input(event: PlannedKeyEvent) -> INPUT {
    let flags = match event.transition {
        KeyTransition::Down => KEYBD_EVENT_FLAGS::default(),
        KeyTransition::Up => KEYEVENTF_KEYUP,
    };

    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: event.virtual_key,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: CONTROLLER_INPUT_MARKER,
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(ctrl: bool, alt: bool, shift: bool, function_key: u8) -> BridgeChord {
        BridgeChord {
            ctrl,
            alt,
            shift,
            function_key,
        }
    }

    #[test]
    fn validates_bridge_function_key_range() {
        assert!(matches!(
            function_virtual_key(12),
            Err(PlatformError::InvalidBridgeFunctionKey(12))
        ));
        assert_eq!(function_virtual_key(13).expect("F13").0, 0x7c);
        assert_eq!(function_virtual_key(24).expect("F24").0, 0x87);
    }

    #[test]
    fn only_releases_modifiers_synthesized_by_this_dispatch() {
        let plan = plan_bridge_events(
            chord(true, true, false, 13),
            ModifierSnapshot {
                control: true,
                ..ModifierSnapshot::default()
            },
        )
        .expect("valid plan");

        assert_eq!(plan.synthesized_modifiers, 1);
        assert_eq!(
            plan.events,
            vec![
                PlannedKeyEvent {
                    virtual_key: VK_LMENU,
                    transition: KeyTransition::Down,
                },
                PlannedKeyEvent {
                    virtual_key: VIRTUAL_KEY(0x7c),
                    transition: KeyTransition::Down,
                },
                PlannedKeyEvent {
                    virtual_key: VIRTUAL_KEY(0x7c),
                    transition: KeyTransition::Up,
                },
                PlannedKeyEvent {
                    virtual_key: VK_LMENU,
                    transition: KeyTransition::Up,
                },
            ]
        );
    }

    #[test]
    fn refuses_extra_physical_modifier() {
        assert!(matches!(
            plan_bridge_events(
                chord(false, false, false, 13),
                ModifierSnapshot {
                    shift: true,
                    ..ModifierSnapshot::default()
                }
            ),
            Err(PlatformError::UnexpectedModifierHeld(ModifierKey::Shift))
        ));
    }

    #[test]
    fn cleanup_releases_only_keys_left_down_by_partial_insert() {
        let plan = plan_bridge_events(chord(true, true, false, 13), ModifierSnapshot::default())
            .expect("valid plan");

        assert_eq!(
            plan.cleanup_after(3),
            vec![
                PlannedKeyEvent {
                    virtual_key: VIRTUAL_KEY(0x7c),
                    transition: KeyTransition::Up,
                },
                PlannedKeyEvent {
                    virtual_key: VK_LMENU,
                    transition: KeyTransition::Up,
                },
                PlannedKeyEvent {
                    virtual_key: VK_LCONTROL,
                    transition: KeyTransition::Up,
                },
            ]
        );
        assert!(plan.cleanup_after(plan.events.len()).is_empty());
    }

    #[test]
    fn every_input_carries_controller_marker() {
        let plan = plan_bridge_events(chord(false, false, false, 24), ModifierSnapshot::default())
            .expect("valid plan");

        for input in plan.to_inputs() {
            // SAFETY: event_to_input initialized the active union member as a
            // keyboard INPUT, so reading `ki` is valid in this test.
            let keyboard = unsafe { input.Anonymous.ki };
            assert_eq!(keyboard.dwExtraInfo, CONTROLLER_INPUT_MARKER);
        }
    }
}
