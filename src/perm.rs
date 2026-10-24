//! File modes. aso runs on macOS and Linux; the mode calls exist on Unix only,
//! so they are here, behind one `cfg`, and the rest of the crate has none.

use std::path::Path;

/// Set the permission bits of `path`.
#[cfg(unix)]
pub fn set_mode(path: &Path, mode: u32) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .map_err(|e| format!("cannot set the mode of {}: {e}", path.display()))
}

/// Where there are no mode bits there is nothing to set.
#[cfg(not(unix))]
pub fn set_mode(_path: &Path, _mode: u32) -> Result<(), String> {
    Ok(())
}
