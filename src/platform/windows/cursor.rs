use windows::Win32::UI::WindowsAndMessaging::{IDC_SIZENS, IDC_SIZEWE, LoadCursorW, SetCursor};

use crate::pane_layout::SplitAxis;

use super::error::{PlatformError, PlatformResult};

/// Displays the standard Windows resize cursor for a native pane divider.
///
/// The controller uses this only after it has captured a primary-button drag.
/// Ordinary hover moves always pass through to Windows Terminal so the cursor
/// hint can never block the terminal's pointer input.
pub fn show_pane_resize_cursor(axis: SplitAxis) -> PlatformResult<()> {
    let resource = match axis {
        SplitAxis::Vertical => IDC_SIZEWE,
        SplitAxis::Horizontal => IDC_SIZENS,
    };
    // SAFETY: predefined cursor resources are process-independent shared
    // handles and must not be destroyed by the caller.
    let cursor = unsafe { LoadCursorW(None, resource) }
        .map_err(|source| PlatformError::win32("LoadCursorW(resize)", source))?;
    // SAFETY: `cursor` is a live shared system cursor handle.
    unsafe {
        let _ = SetCursor(Some(cursor));
    }
    Ok(())
}
