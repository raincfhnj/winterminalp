use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use crate::{AppError, AppResult};

use super::manifest::BackupManifest;

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(crate) struct FileSnapshot {
    pub bytes: Vec<u8>,
    pub sha256: String,
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    for &byte in digest.as_slice() {
        hex.push(HEX[usize::from(byte >> 4)] as char);
        hex.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    hex
}

pub(crate) fn read_snapshot(path: &Path) -> AppResult<FileSnapshot> {
    reject_symlink(path)?;
    let bytes =
        fs::read(path).map_err(|error| AppError::io("read integration file", path, error))?;
    let sha256 = sha256_hex(&bytes);
    Ok(FileSnapshot { bytes, sha256 })
}

pub(crate) fn read_optional_snapshot(path: &Path) -> AppResult<Option<FileSnapshot>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(AppError::Settings {
                    path: path.to_path_buf(),
                    message: "refusing to modify a symbolic link".to_owned(),
                });
            }
            read_snapshot(path).map(Some)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(AppError::io("inspect integration file", path, error)),
    }
}

pub(crate) fn create_backup(
    state_dir: &Path,
    label: &str,
    source_path: &Path,
    snapshot: &FileSnapshot,
) -> AppResult<BackupManifest> {
    let backup_dir = state_dir.join("backups");
    fs::create_dir_all(&backup_dir)
        .map_err(|error| AppError::io("create integration backup directory", &backup_dir, error))?;
    let safe_label: String = label
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect();
    let path = unique_path(&backup_dir, &format!("{safe_label}.settings"), "json");
    write_new_synced(&path, &snapshot.bytes, "write integration backup")?;
    let verified = read_snapshot(&path)?;
    if verified.sha256 != snapshot.sha256 {
        return Err(AppError::Settings {
            path,
            message: "backup verification checksum did not match source bytes".to_owned(),
        });
    }

    Ok(BackupManifest {
        source_path: source_path.to_path_buf(),
        backup_path: path,
        sha256: snapshot.sha256.clone(),
        byte_len: snapshot.bytes.len() as u64,
    })
}

/// Replaces a file only if its current bytes still match the snapshot used to plan the edit.
pub(crate) fn atomic_replace(
    path: &Path,
    expected_sha256: Option<&str>,
    replacement: &[u8],
) -> AppResult<String> {
    assert_compare_and_swap(path, expected_sha256)?;
    let parent = path.parent().ok_or_else(|| AppError::Settings {
        path: path.to_path_buf(),
        message: "target file has no parent directory".to_owned(),
    })?;
    fs::create_dir_all(parent)
        .map_err(|error| AppError::io("create integration target directory", parent, error))?;
    let temp_path = unique_path(parent, ".winterminalpp-tmp", "json");
    write_new_synced(&temp_path, replacement, "write integration temporary file")?;

    // A second check closes the potentially long window spent serializing and syncing the temp file.
    if let Err(error) = assert_compare_and_swap(path, expected_sha256) {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }

    if let Err(error) = replace_with_native_atomic_move(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }

    let expected_replacement_hash = sha256_hex(replacement);
    let verified = read_snapshot(path)?;
    if verified.sha256 != expected_replacement_hash || verified.bytes != replacement {
        return Err(AppError::Settings {
            path: path.to_path_buf(),
            message: "read-back verification failed after atomic replacement".to_owned(),
        });
    }
    Ok(expected_replacement_hash)
}

pub(crate) fn remove_if_hash(path: &Path, expected_sha256: &str) -> AppResult<bool> {
    let Some(snapshot) = read_optional_snapshot(path)? else {
        return Ok(false);
    };
    if snapshot.sha256 != expected_sha256 {
        return Ok(false);
    }
    fs::remove_file(path)
        .map_err(|error| AppError::io("remove managed integration file", path, error))?;
    Ok(true)
}

fn assert_compare_and_swap(path: &Path, expected_sha256: Option<&str>) -> AppResult<()> {
    let actual = read_optional_snapshot(path)?;
    let matches = match (expected_sha256, actual.as_ref()) {
        (None, None) => true,
        (Some(expected), Some(actual)) => expected == actual.sha256,
        _ => false,
    };
    if matches {
        return Ok(());
    }

    Err(AppError::SettingsConflict(format!(
        "{} changed after it was read; no bytes were replaced",
        path.display()
    )))
}

fn reject_symlink(path: &Path) -> AppResult<()> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| AppError::io("inspect integration file", path, error))?;
    if metadata.file_type().is_symlink() {
        return Err(AppError::Settings {
            path: path.to_path_buf(),
            message: "refusing to modify a symbolic link".to_owned(),
        });
    }
    Ok(())
}

fn write_new_synced(path: &Path, bytes: &[u8], operation: &'static str) -> AppResult<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| AppError::io(operation, path, error))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| AppError::io(operation, path, error))
}

fn unique_path(directory: &Path, stem: &str, extension: &str) -> PathBuf {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    directory.join(format!(
        "{stem}-{}-{now}-{sequence}.{extension}",
        std::process::id()
    ))
}

#[cfg(windows)]
fn replace_with_native_atomic_move(source: &Path, destination: &Path) -> AppResult<()> {
    use std::os::windows::ffi::OsStrExt;

    use windows::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    use windows::core::PCWSTR;

    let source_wide: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination_wide: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // SAFETY: both buffers are valid, immutable, NUL-terminated UTF-16 paths for this call.
    unsafe {
        MoveFileExW(
            PCWSTR(source_wide.as_ptr()),
            PCWSTR(destination_wide.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(|error| {
        AppError::Native(format!(
            "MoveFileExW failed from {} to {}: {error}",
            source.display(),
            destination.display()
        ))
    })
}

#[cfg(not(windows))]
fn replace_with_native_atomic_move(source: &Path, destination: &Path) -> AppResult<()> {
    fs::rename(source, destination)
        .map_err(|error| AppError::io("atomically replace integration file", destination, error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_and_swap_rejects_stale_snapshot() {
        let temp = tempfile::tempdir().expect("temporary directory should be created");
        let path = temp.path().join("settings.json");
        fs::write(&path, b"{\"value\":1}").expect("fixture should be written");
        let original = read_snapshot(&path).expect("fixture should be readable");
        fs::write(&path, b"{\"value\":2}").expect("fixture should be changed");

        let result = atomic_replace(&path, Some(&original.sha256), b"{\"value\":3}");

        assert!(matches!(result, Err(AppError::SettingsConflict(_))));
        assert_eq!(
            fs::read(&path).expect("fixture should remain readable"),
            b"{\"value\":2}"
        );
    }

    #[test]
    fn backup_preserves_the_exact_source_bytes() {
        let temp = tempfile::tempdir().expect("temporary directory should be created");
        let path = temp.path().join("settings.json");
        let bytes = b"\xef\xbb\xbf{\r\n  // user comment\r\n}\r\n";
        fs::write(&path, bytes).expect("fixture should be written");
        let snapshot = read_snapshot(&path).expect("fixture should be readable");

        let backup = create_backup(temp.path(), "stable", &path, &snapshot)
            .expect("backup should be created");

        assert_eq!(
            fs::read(&backup.backup_path).expect("backup should be readable"),
            bytes
        );
        assert_eq!(backup.sha256, sha256_hex(bytes));
    }
}
