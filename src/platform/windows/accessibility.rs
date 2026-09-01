use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomation2, IUIAutomationElement, TreeScope_Descendants,
};
use windows::core::Interface;

use crate::model::WindowIdentity;
use crate::pane_layout::{PaneGeometry, ScreenPoint, ScreenRect};

use super::error::{PlatformError, PlatformResult};
use super::foreground::{foreground_hwnd, validate_window_identity};

const UI_AUTOMATION_CONNECTION_TIMEOUT_MS: u32 = 250;
const UI_AUTOMATION_TRANSACTION_TIMEOUT_MS: u32 = 250;
const TERMINAL_CONTROL_CLASS_NAME: &str = "TermControl";

/// Thread-affine UI Automation client used only outside low-level hook callbacks.
///
/// Windows Terminal remains the layout authority. This adapter reads disposable
/// `TermControl` rectangles and can focus one native pane before dispatching a
/// documented `resizePane` action; it never persists or reconstructs a pane tree.
pub struct TerminalAccessibility {
    automation: IUIAutomation,
    _apartment: ComApartment,
}

impl TerminalAccessibility {
    pub fn initialize() -> PlatformResult<Self> {
        let apartment = ComApartment::initialize()?;
        // SAFETY: COM is initialized on this thread, CUIAutomation is an
        // in-process COM class, and no aggregation is requested.
        let automation = unsafe {
            CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
        }
        .map_err(|source| PlatformError::win32("CoCreateInstance(CUIAutomation)", source))?;

        if let Ok(automation_v2) = automation.cast::<IUIAutomation2>() {
            // SAFETY: the interface was queried from the live automation
            // object and the timeout values are finite milliseconds.
            unsafe {
                let _ = automation_v2.SetConnectionTimeout(UI_AUTOMATION_CONNECTION_TIMEOUT_MS);
                let _ = automation_v2.SetTransactionTimeout(UI_AUTOMATION_TRANSACTION_TIMEOUT_MS);
            }
        }

        Ok(Self {
            automation,
            _apartment: apartment,
        })
    }

    pub fn pane_geometries(&self, hwnd: isize) -> PlatformResult<Vec<PaneGeometry>> {
        let elements = self.terminal_elements(hwnd)?;
        let mut panes = Vec::with_capacity(elements.len());
        for element in elements {
            // SAFETY: `element` is a live UI Automation proxy used on the COM
            // apartment where it was obtained.
            let bounds = unsafe { element.CurrentBoundingRectangle() }.map_err(|source| {
                PlatformError::win32("IUIAutomationElement::CurrentBoundingRectangle", source)
            })?;
            let has_keyboard_focus = unsafe { element.CurrentHasKeyboardFocus() }
                .map_err(|source| {
                    PlatformError::win32("IUIAutomationElement::CurrentHasKeyboardFocus", source)
                })?
                .as_bool();
            let bounds = ScreenRect::new(bounds.left, bounds.top, bounds.right, bounds.bottom);
            if bounds.is_valid() {
                panes.push(PaneGeometry {
                    bounds,
                    has_keyboard_focus,
                });
            }
        }
        Ok(panes)
    }

    pub fn focus_pane_at(&self, target: WindowIdentity, point: ScreenPoint) -> PlatformResult<()> {
        let actual = foreground_hwnd();
        if actual != target.hwnd {
            return Err(PlatformError::TargetNotForeground {
                expected: target.hwnd,
                actual,
            });
        }
        validate_window_identity(target)?;
        let element = self
            .terminal_elements(target.hwnd)?
            .into_iter()
            .find_map(|element| {
                // A transient provider failure for one stale element should
                // not prevent a later live TermControl from being selected.
                let bounds = unsafe { element.CurrentBoundingRectangle() }.ok()?;
                ScreenRect::new(bounds.left, bounds.top, bounds.right, bounds.bottom)
                    .contains(point)
                    .then_some(element)
            })
            .ok_or(PlatformError::PaneNotFoundAt {
                hwnd: target.hwnd,
                x: point.x,
                y: point.y,
            })?;

        // SAFETY: `element` is a TermControl descendant of the revalidated
        // target HWND and is used on its creating COM apartment.
        unsafe { element.SetFocus() }
            .map_err(|source| PlatformError::win32("IUIAutomationElement::SetFocus", source))
    }

    fn terminal_elements(&self, hwnd: isize) -> PlatformResult<Vec<IUIAutomationElement>> {
        // SAFETY: the caller supplies an opaque top-level HWND that is checked
        // by UI Automation; invalid or stale handles become a structured error.
        let root = unsafe { self.automation.ElementFromHandle(isize_to_hwnd(hwnd)) }
            .map_err(|source| PlatformError::win32("IUIAutomation::ElementFromHandle", source))?;
        // SAFETY: the condition belongs to this automation client.
        let condition = unsafe { self.automation.CreateTrueCondition() }
            .map_err(|source| PlatformError::win32("IUIAutomation::CreateTrueCondition", source))?;
        // SAFETY: `root` and `condition` are live proxies on this apartment;
        // descendants are read-only accessibility elements.
        let descendants = unsafe { root.FindAll(TreeScope_Descendants, &condition) }
            .map_err(|source| PlatformError::win32("IUIAutomationElement::FindAll", source))?;
        let length = unsafe { descendants.Length() }
            .map_err(|source| PlatformError::win32("IUIAutomationElementArray::Length", source))?;
        let mut elements = Vec::new();
        for index in 0..length {
            let element = unsafe { descendants.GetElement(index) }.map_err(|source| {
                PlatformError::win32("IUIAutomationElementArray::GetElement", source)
            })?;
            let class_name = unsafe { element.CurrentClassName() }.map_err(|source| {
                PlatformError::win32("IUIAutomationElement::CurrentClassName", source)
            })?;
            if class_name == TERMINAL_CONTROL_CLASS_NAME {
                elements.push(element);
            }
        }
        Ok(elements)
    }
}

struct ComApartment;

impl ComApartment {
    fn initialize() -> PlatformResult<Self> {
        // SAFETY: this is called once for a newly spawned worker thread and is
        // balanced by CoUninitialize when the apartment guard is dropped.
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
            .ok()
            .map_err(|source| PlatformError::win32("CoInitializeEx", source))?;
        Ok(Self)
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        // SAFETY: this guard is dropped on the same thread where initialization
        // succeeded, after the automation field has been released.
        unsafe { CoUninitialize() };
    }
}

fn isize_to_hwnd(hwnd: isize) -> HWND {
    HWND(hwnd as *mut core::ffi::c_void)
}
