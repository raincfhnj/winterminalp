use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};

/// Opts the controller into per-monitor DPI awareness.
///
/// Low-level mouse hook coordinates are physical pixels, while UI Automation
/// virtualizes bounding rectangles for DPI-unaware callers. Without this the
/// inferred divider coordinates would not line up with the cursor on scaled
/// displays. The call is best-effort: it fails harmlessly when awareness was
/// already fixed by a manifest or a previous call.
pub fn enable_per_monitor_dpi_awareness() {
    // SAFETY: this takes a constant enum by value and has no pointer or
    // ownership requirements.
    let _ = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
}
