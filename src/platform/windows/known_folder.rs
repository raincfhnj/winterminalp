use std::path::PathBuf;

use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{FOLDERID_Documents, KF_FLAG_DEFAULT, SHGetKnownFolderPath};

/// Resolves the per-user `Documents` known folder through the Shell API.
///
/// Returns `None` when the Shell API is unavailable so the caller can fall back
/// to environment variables.
#[must_use]
pub fn documents_directory() -> Option<PathBuf> {
    // SAFETY: the shell allocates the returned string and the flag value is
    // valid; the pointer is released with CoTaskMemFree exactly once.
    let pointer =
        unsafe { SHGetKnownFolderPath(&FOLDERID_Documents, KF_FLAG_DEFAULT, None) }.ok()?;
    let resolved = unsafe { pointer.to_string() }.ok().map(PathBuf::from);
    unsafe { CoTaskMemFree(Some(pointer.0 as *const core::ffi::c_void)) };
    resolved
}
