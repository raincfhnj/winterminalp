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
    let virtual_key = function_virtual_key(chord.function_key)?;
    send_key_chord(target, virtual_key, chord.ctrl, chord.alt, chord.shift)
}

/// Sends one arbitrary key chord to an unchanged foreground target.
///
/// Used for hidden bridge function keys. Only modifiers this call synthesizes
/// are released; modifiers already held by the user are preserved, extra held
/// modifiers fail closed, and a target key that is already physically held is
/// rejected before any input is injected.
fn send_key_chord(
    target: WindowIdentity,
    virtual_key: VIRTUAL_KEY,
    ctrl: bool,
    alt: bool,
    shift: bool,
) -> PlatformResult<InputDispatch> {
    send_chord(target, virtual_key, ctrl, alt, shift, true)
}

/// Sends one chord even when its target key is already physically held.
///
/// Literal prefix replay is triggered by the key-down of the very key it
/// re-injects, so the normal "target key already held" guard would always fail.
/// That physical key-down and its matching key-up are consumed by the
/// controller before reaching the target, so the injected chord is still the
/// only complete key transition the target observes.
pub fn send_literal_chord(
    target: WindowIdentity,
    virtual_key: VIRTUAL_KEY,
    ctrl: bool,
    alt: bool,
    shift: bool,
) -> PlatformResult<InputDispatch> {
    send_chord(target, virtual_key, ctrl, alt, shift, false)
}

fn send_chord(
    target: WindowIdentity,
    virtual_key: VIRTUAL_KEY,
    ctrl: bool,
    alt: bool,
    shift: bool,
    check_target_key: bool,
) -> PlatformResult<InputDispatch> {
    let snapshot = ModifierSnapshot::capture(virtual_key);
    let plan = plan_chord_events(virtual_key, ctrl, alt, shift, snapshot, check_target_key)?;

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

    Ok(InputDispatch { sent })
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
    target_key: bool,
}

impl ModifierSnapshot {
    fn capture(target_key: VIRTUAL_KEY) -> Self {
        Self {
            control: key_is_down(VK_CONTROL),
            alt: key_is_down(VK_MENU),
            shift: key_is_down(VK_SHIFT),
            windows: key_is_down(VK_LWIN) || key_is_down(VK_RWIN),
            target_key: key_is_down(target_key),
        }
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

fn plan_chord_events(
    virtual_key: VIRTUAL_KEY,
    ctrl: bool,
    alt: bool,
    shift: bool,
    snapshot: ModifierSnapshot,
    check_target_key: bool,
) -> PlatformResult<InputPlan> {
    if check_target_key && snapshot.target_key {
        return Err(PlatformError::TargetKeyHeld(virtual_key.0));
    }
    if snapshot.windows {
        return Err(PlatformError::UnexpectedModifierHeld(ModifierKey::Windows));
    }

    let mut events = Vec::with_capacity(8);
    let mut synthesized = Vec::with_capacity(3);
    plan_modifier(
        ctrl,
        snapshot.control,
        ModifierKey::Control,
        VK_LCONTROL,
        &mut events,
        &mut synthesized,
    )?;
    plan_modifier(
        alt,
        snapshot.alt,
        ModifierKey::Alt,
        VK_LMENU,
        &mut events,
        &mut synthesized,
    )?;
    plan_modifier(
        shift,
        snapshot.shift,
        ModifierKey::Shift,
        VK_LSHIFT,
        &mut events,
        &mut synthesized,
    )?;

    events.push(PlannedKeyEvent {
        virtual_key,
        transition: KeyTransition::Down,
    });
    events.push(PlannedKeyEvent {
        virtual_key,
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

    Ok(InputPlan { events })
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

    fn fkey(function_key: u8) -> VIRTUAL_KEY {
        function_virtual_key(function_key).expect("fixture function key should be valid")
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
        let plan = plan_chord_events(
            fkey(13),
            true,
            true,
            false,
            ModifierSnapshot {
                control: true,
                ..ModifierSnapshot::default()
            },
            true,
        )
        .expect("valid plan");

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
    fn plans_a_literal_character_chord() {
        let plan = plan_chord_events(
            VIRTUAL_KEY(u16::from(b'B')),
            true,
            false,
            false,
            ModifierSnapshot::default(),
            false,
        )
        .expect("valid plan");

        assert_eq!(
            plan.events,
            vec![
                PlannedKeyEvent {
                    virtual_key: VK_LCONTROL,
                    transition: KeyTransition::Down,
                },
                PlannedKeyEvent {
                    virtual_key: VIRTUAL_KEY(u16::from(b'B')),
                    transition: KeyTransition::Down,
                },
                PlannedKeyEvent {
                    virtual_key: VIRTUAL_KEY(u16::from(b'B')),
                    transition: KeyTransition::Up,
                },
                PlannedKeyEvent {
                    virtual_key: VK_LCONTROL,
                    transition: KeyTransition::Up,
                },
            ]
        );
    }

    #[test]
    fn literal_replay_allows_a_physically_held_target_key() {
        let held = ModifierSnapshot {
            target_key: true,
            ..ModifierSnapshot::default()
        };

        assert!(matches!(
            plan_chord_events(VIRTUAL_KEY(u16::from(b'B')), true, false, false, held, true),
            Err(PlatformError::TargetKeyHeld(_))
        ));
        assert!(
            plan_chord_events(
                VIRTUAL_KEY(u16::from(b'B')),
                true,
                false,
                false,
                held,
                false
            )
            .is_ok()
        );
    }

    #[test]
    fn refuses_extra_physical_modifier() {
        assert!(matches!(
            plan_chord_events(
                fkey(13),
                false,
                false,
                false,
                ModifierSnapshot {
                    shift: true,
                    ..ModifierSnapshot::default()
                },
                true
            ),
            Err(PlatformError::UnexpectedModifierHeld(ModifierKey::Shift))
        ));
    }

    #[test]
    fn cleanup_releases_only_keys_left_down_by_partial_insert() {
        let plan = plan_chord_events(
            fkey(13),
            true,
            true,
            false,
            ModifierSnapshot::default(),
            true,
        )
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
        let plan = plan_chord_events(
            fkey(24),
            false,
            false,
            false,
            ModifierSnapshot::default(),
            true,
        )
        .expect("valid plan");

        for input in plan.to_inputs() {
            // SAFETY: event_to_input initialized the active union member as a
            // keyboard INPUT, so reading `ki` is valid in this test.
            let keyboard = unsafe { input.Anonymous.ki };
            assert_eq!(keyboard.dwExtraInfo, CONTROLLER_INPUT_MARKER);
        }
    }
}
