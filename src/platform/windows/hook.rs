use std::cell::{Cell, UnsafeCell};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT, LLKHF_ALTDOWN,
    LLKHF_EXTENDED, LLKHF_INJECTED, LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT, PM_NOREMOVE, PeekMessageW,
    PostThreadMessageW, SetWindowsHookExW, TranslateMessage, WH_KEYBOARD_LL, WH_MOUSE_LL,
    WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_QUIT, WM_SYSKEYDOWN,
    WM_SYSKEYUP,
};
use windows::core::Owned;

use super::error::{PlatformError, PlatformResult};

/// `dwExtraInfo` marker attached to every key synthesized by this controller.
pub const CONTROLLER_INPUT_MARKER: usize = 0x5754_5050;

static ACTIVE_HOOK_STATE: AtomicPtr<HookState> = AtomicPtr::new(ptr::null_mut());

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyTransition {
    Down,
    Up,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawKeyEvent {
    pub virtual_key: u32,
    pub scan_code: u32,
    pub transition: KeyTransition,
    pub is_system_key: bool,
    pub is_extended: bool,
    pub is_alt_down: bool,
    pub injected: bool,
    pub timestamp_ms: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseEventKind {
    Move,
    LeftDown,
    LeftUp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawMouseEvent {
    pub x: i32,
    pub y: i32,
    pub kind: MouseEventKind,
    pub injected: bool,
    pub timestamp_ms: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawInputEvent {
    Keyboard(RawKeyEvent),
    Mouse(RawMouseEvent),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookDecision {
    Pass,
    Consume,
}

/// Handler called synchronously on the dedicated hook thread.
///
/// It must not perform I/O, wait, pump messages, or run unbounded work. Use a
/// bounded `try_send` to hand an intent to another thread. Injected events are
/// always passed through before this handler is invoked.
pub type RawInputHandler = Box<dyn FnMut(RawInputEvent) -> HookDecision + Send + 'static>;

/// RAII handle for process-global low-level keyboard and optional mouse hooks.
///
/// The callback and Win32 message loop run on a dedicated thread. Dropping the
/// handle posts `WM_QUIT`, joins that thread, and lets `Owned<HHOOK>` call
/// `UnhookWindowsHookEx` on the installer thread.
pub struct InputHook {
    thread_id: u32,
    join: Option<JoinHandle<PlatformResult<()>>>,
}

impl InputHook {
    pub fn start(handler: RawInputHandler, include_mouse: bool) -> PlatformResult<Self> {
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let join = thread::Builder::new()
            .name("winterminalp-input-hook".to_owned())
            .spawn(move || hook_thread_main(handler, include_mouse, ready_tx))
            .map_err(|source| PlatformError::HookThreadSpawn { source })?;

        match ready_rx.recv() {
            Ok(Ok(thread_id)) => Ok(Self {
                thread_id,
                join: Some(join),
            }),
            Ok(Err(error)) => {
                let _ = join.join();
                Err(error)
            }
            Err(_) => match join.join() {
                Ok(Err(error)) => Err(error),
                Ok(Ok(())) => Err(PlatformError::HookStartupTerminated),
                Err(_) => Err(PlatformError::HookThreadPanicked),
            },
        }
    }

    #[must_use]
    pub const fn thread_id(&self) -> u32 {
        self.thread_id
    }

    pub fn stop(mut self) -> PlatformResult<()> {
        self.shutdown()
    }

    fn shutdown(&mut self) -> PlatformResult<()> {
        let Some(join) = self.join.take() else {
            return Ok(());
        };

        // SAFETY: `thread_id` belongs to the live hook thread whose message
        // queue is created before start returns; parameters contain no pointers.
        let post_result =
            unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) }
                .map_err(|source| PlatformError::win32("PostThreadMessageW(WM_QUIT)", source));

        let thread_result = join.join().map_err(|_| PlatformError::HookThreadPanicked)?;
        thread_result?;
        post_result
    }
}

impl Drop for InputHook {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

struct HookState {
    enabled: Cell<bool>,
    handler: UnsafeCell<RawInputHandler>,
}

impl HookState {
    fn new(handler: RawInputHandler) -> Self {
        Self {
            enabled: Cell::new(true),
            handler: UnsafeCell::new(handler),
        }
    }

    fn handle(&self, event: RawInputEvent) -> HookDecision {
        if !self.enabled.get() {
            return HookDecision::Pass;
        }

        // SAFETY: Windows invokes WH_KEYBOARD_LL on the installer thread. The
        // handler is never accessed outside that callback thread, and handlers
        // are forbidden from pumping messages (which would permit reentrancy).
        let result = catch_unwind(AssertUnwindSafe(|| unsafe {
            (&mut *self.handler.get())(event)
        }));
        match result {
            Ok(decision) => decision,
            Err(_) => {
                self.enabled.set(false);
                HookDecision::Pass
            }
        }
    }
}

struct ActiveHookState {
    pointer: *mut HookState,
    _state: Box<HookState>,
}

impl ActiveHookState {
    fn install(handler: RawInputHandler) -> PlatformResult<Self> {
        let mut state = Box::new(HookState::new(handler));
        let pointer = state.as_mut() as *mut HookState;
        ACTIVE_HOOK_STATE
            .compare_exchange(
                ptr::null_mut(),
                pointer,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .map_err(|_| PlatformError::HookAlreadyActive)?;

        Ok(Self {
            pointer,
            _state: state,
        })
    }
}

impl Drop for ActiveHookState {
    fn drop(&mut self) {
        let _ = ACTIVE_HOOK_STATE.compare_exchange(
            self.pointer,
            ptr::null_mut(),
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
}

fn hook_thread_main(
    handler: RawInputHandler,
    include_mouse: bool,
    ready: mpsc::SyncSender<PlatformResult<u32>>,
) -> PlatformResult<()> {
    let mut initial_message = MSG::default();
    // SAFETY: `initial_message` is a valid writable MSG and PM_NOREMOVE only
    // ensures this thread owns a message queue before its id is published.
    unsafe {
        let _ = PeekMessageW(&mut initial_message, None, 0, 0, PM_NOREMOVE);
    }

    let active_state = match ActiveHookState::install(handler) {
        Ok(state) => state,
        Err(error) => {
            let _ = ready.send(Err(error));
            return Ok(());
        }
    };

    let keyboard_hook = match install_keyboard_hook() {
        Ok(hook) => hook,
        Err(error) => {
            drop(active_state);
            let _ = ready.send(Err(error));
            return Ok(());
        }
    };
    let mouse_hook = if include_mouse {
        match install_mouse_hook() {
            Ok(hook) => Some(hook),
            Err(error) => {
                drop(keyboard_hook);
                drop(active_state);
                let _ = ready.send(Err(error));
                return Ok(());
            }
        }
    } else {
        None
    };
    // SAFETY: GetCurrentThreadId takes no arguments and has no ownership rules.
    let thread_id = unsafe { GetCurrentThreadId() };
    if ready.send(Ok(thread_id)).is_err() {
        return Ok(());
    }

    let result = run_message_loop();
    drop(mouse_hook);
    drop(keyboard_hook);
    drop(active_state);
    result
}

fn install_keyboard_hook() -> PlatformResult<Owned<HHOOK>> {
    // SAFETY: None requests the module handle for the current process; the
    // returned borrowed handle remains loaded for the process lifetime.
    let module = unsafe { GetModuleHandleW(None) }
        .map_err(|source| PlatformError::win32("GetModuleHandleW", source))?;
    let instance = HINSTANCE(module.0);
    // SAFETY: the callback uses the required system ABI, the module remains
    // loaded, and thread id 0 requests the documented global low-level hook.
    let hook = unsafe {
        SetWindowsHookExW(
            WH_KEYBOARD_LL,
            Some(low_level_keyboard_proc),
            Some(instance),
            0,
        )
    }
    .map_err(|source| PlatformError::win32("SetWindowsHookExW(WH_KEYBOARD_LL)", source))?;

    // SAFETY: SetWindowsHookExW returned a valid hook handle and this function
    // transfers its sole ownership to Owned for RAII unhooking.
    Ok(unsafe { Owned::new(hook) })
}

fn install_mouse_hook() -> PlatformResult<Owned<HHOOK>> {
    // SAFETY: None requests the module handle for the current process; the
    // returned borrowed handle remains loaded for the process lifetime.
    let module = unsafe { GetModuleHandleW(None) }
        .map_err(|source| PlatformError::win32("GetModuleHandleW", source))?;
    let instance = HINSTANCE(module.0);
    // SAFETY: the callback uses the required system ABI, the module remains
    // loaded, and thread id 0 requests the documented global low-level hook.
    let hook =
        unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(low_level_mouse_proc), Some(instance), 0) }
            .map_err(|source| PlatformError::win32("SetWindowsHookExW(WH_MOUSE_LL)", source))?;

    // SAFETY: SetWindowsHookExW returned a valid hook handle and this function
    // transfers its sole ownership to Owned for RAII unhooking.
    Ok(unsafe { Owned::new(hook) })
}

fn run_message_loop() -> PlatformResult<()> {
    let mut message = MSG::default();
    loop {
        // SAFETY: `message` is a valid writable MSG owned by this hook thread.
        let result = unsafe {
            windows::Win32::UI::WindowsAndMessaging::GetMessageW(&mut message, None, 0, 0)
        };
        match result.0 {
            -1 => {
                return Err(PlatformError::win32(
                    "GetMessageW",
                    windows::core::Error::from_thread(),
                ));
            }
            0 => return Ok(()),
            // SAFETY: `message` was populated successfully by GetMessageW and
            // remains valid for translation and dispatch.
            _ => unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            },
        }
    }
}

unsafe extern "system" fn low_level_keyboard_proc(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if code != HC_ACTION as i32 || lparam.0 == 0 {
        // SAFETY: forwarding the original callback parameters is required by
        // the hook contract when this callback does not handle the event.
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }

    // SAFETY: for HC_ACTION Windows documents lparam as a valid pointer to a
    // KBDLLHOOKSTRUCT for the duration of this callback.
    let data = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
    let Some(event) = raw_key_event_from_hook(wparam.0 as u32, data) else {
        // SAFETY: forwarding the unchanged parameters preserves the hook chain.
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    };

    // Never feed synthetic input back into the prefix state machine. In
    // particular, this prevents the synthetic high-function-key bridge from
    // recursively firing.
    if event.injected {
        // SAFETY: forwarding the unchanged parameters preserves the hook chain.
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }

    let state = ACTIVE_HOOK_STATE.load(Ordering::Acquire);
    if state.is_null() {
        // SAFETY: forwarding the unchanged parameters preserves the hook chain.
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }

    // SAFETY: ActiveHookState publishes this pointer before hook installation,
    // clears it only after the hook is dropped, and callbacks are serialized on
    // the installer thread.
    let decision = unsafe { (&*state).handle(RawInputEvent::Keyboard(event)) };
    match decision {
        // SAFETY: forwarding the unchanged parameters preserves the hook chain.
        HookDecision::Pass => unsafe { CallNextHookEx(None, code, wparam, lparam) },
        HookDecision::Consume => LRESULT(1),
    }
}

unsafe extern "system" fn low_level_mouse_proc(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if code != HC_ACTION as i32 || lparam.0 == 0 {
        // SAFETY: forwarding the original callback parameters is required by
        // the hook contract when this callback does not handle the event.
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }

    // SAFETY: for HC_ACTION Windows documents lparam as a valid pointer to an
    // MSLLHOOKSTRUCT for the duration of this callback.
    let data = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
    let Some(event) = raw_mouse_event_from_hook(wparam.0 as u32, data) else {
        // SAFETY: forwarding the unchanged parameters preserves the hook chain.
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    };
    if event.injected {
        // SAFETY: forwarding the unchanged parameters preserves the hook chain.
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }

    let state = ACTIVE_HOOK_STATE.load(Ordering::Acquire);
    if state.is_null() {
        // SAFETY: forwarding the unchanged parameters preserves the hook chain.
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }

    // SAFETY: ActiveHookState publishes this pointer before hook installation,
    // clears it only after both hooks are dropped, and callbacks are serialized
    // on the installer thread.
    let decision = unsafe { (&*state).handle(RawInputEvent::Mouse(event)) };
    match decision {
        // SAFETY: forwarding the unchanged parameters preserves the hook chain.
        HookDecision::Pass => unsafe { CallNextHookEx(None, code, wparam, lparam) },
        HookDecision::Consume => LRESULT(1),
    }
}

fn raw_key_event_from_hook(message: u32, data: &KBDLLHOOKSTRUCT) -> Option<RawKeyEvent> {
    let (transition, is_system_key) = match message {
        WM_KEYDOWN => (KeyTransition::Down, false),
        WM_KEYUP => (KeyTransition::Up, false),
        WM_SYSKEYDOWN => (KeyTransition::Down, true),
        WM_SYSKEYUP => (KeyTransition::Up, true),
        _ => return None,
    };

    Some(RawKeyEvent {
        virtual_key: data.vkCode,
        scan_code: data.scanCode,
        transition,
        is_system_key,
        is_extended: data.flags.contains(LLKHF_EXTENDED),
        is_alt_down: data.flags.contains(LLKHF_ALTDOWN),
        injected: data.flags.contains(LLKHF_INJECTED),
        timestamp_ms: data.time,
    })
}

fn raw_mouse_event_from_hook(message: u32, data: &MSLLHOOKSTRUCT) -> Option<RawMouseEvent> {
    let kind = match message {
        WM_MOUSEMOVE => MouseEventKind::Move,
        WM_LBUTTONDOWN => MouseEventKind::LeftDown,
        WM_LBUTTONUP => MouseEventKind::LeftUp,
        _ => return None,
    };

    Some(RawMouseEvent {
        x: data.pt.x,
        y: data.pt.y,
        kind,
        injected: data.flags & LLMHF_INJECTED != 0,
        timestamp_ms: data.time,
    })
}

#[cfg(test)]
mod tests {
    use windows::Win32::UI::WindowsAndMessaging::{KBDLLHOOKSTRUCT_FLAGS, LLKHF_INJECTED};

    use super::*;

    #[test]
    fn decodes_physical_key_transition() {
        let event = raw_key_event_from_hook(
            WM_KEYDOWN,
            &KBDLLHOOKSTRUCT {
                vkCode: 0x42,
                scanCode: 0x30,
                flags: KBDLLHOOKSTRUCT_FLAGS::default(),
                time: 123,
                dwExtraInfo: 0,
            },
        )
        .expect("known keyboard message");

        assert_eq!(event.transition, KeyTransition::Down);
        assert_eq!(event.virtual_key, 0x42);
        assert!(!event.injected);
        assert!(!event.is_system_key);
    }

    #[test]
    fn marks_injected_system_key_release() {
        let event = raw_key_event_from_hook(
            WM_SYSKEYUP,
            &KBDLLHOOKSTRUCT {
                vkCode: 0x7c,
                scanCode: 0,
                flags: LLKHF_INJECTED,
                time: 456,
                dwExtraInfo: CONTROLLER_INPUT_MARKER,
            },
        )
        .expect("known keyboard message");

        assert_eq!(event.transition, KeyTransition::Up);
        assert!(event.injected);
        assert!(event.is_system_key);
    }

    #[test]
    fn ignores_non_keyboard_messages() {
        assert!(
            raw_key_event_from_hook(0xffff, &KBDLLHOOKSTRUCT::default()).is_none(),
            "unknown messages must pass through"
        );
    }

    #[test]
    fn decodes_mouse_drag_events_and_injection_flags() {
        let event = raw_mouse_event_from_hook(
            WM_LBUTTONDOWN,
            &MSLLHOOKSTRUCT {
                pt: windows::Win32::Foundation::POINT { x: 320, y: 240 },
                mouseData: 0,
                flags: LLMHF_INJECTED,
                time: 789,
                dwExtraInfo: 123,
            },
        )
        .expect("known mouse message");

        assert_eq!(event.kind, MouseEventKind::LeftDown);
        assert_eq!((event.x, event.y), (320, 240));
        assert!(event.injected);
    }

    #[test]
    fn ignores_unneeded_mouse_messages() {
        assert!(
            raw_mouse_event_from_hook(0xffff, &MSLLHOOKSTRUCT::default()).is_none(),
            "unknown mouse messages must pass through"
        );
    }
}
