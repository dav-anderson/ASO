//! Every file aso generates from `aso.toml` and what it knows, in one place:
//! `aso install` writes them and `aso reload` writes them again. A file is
//! replaced only when its text changed, whole and at once.

use std::fs;
use std::path::{Path, PathBuf};

use crate::config::Aso;
use crate::generate;
use crate::program::Program;
use crate::service;
use crate::state::{self, State};

/// Every generated file, as `(path, text)`: fsp-mint's and ecash-wallet's
/// settings, the env files, the service definitions, the logs job, the nginx
/// file (when `air.public_name` is set) and the redeem job (when
/// `redeem.payout` is set).
pub fn all(
    aso: &Aso,
    state: &State,
    exe: &Path,
    config: &Path,
) -> Result<Vec<(PathBuf, String)>, String> {
    let mut files = vec![
        (aso.fsp_config_file(), generate::fsp_toml(aso, state)?),
        (aso.wallet_config_file(), generate::wallet_toml(aso)?),
    ];
    files.extend(service::service_files(aso, exe, Program::Air)?);
    files.extend(service::service_files(aso, exe, Program::Fsp)?);
    files.extend(service::logs_files(aso, exe)?);
    if let Some(nginx) = service::nginx_file(aso)? {
        files.push(nginx);
    }
    if aso.opt("redeem.payout").is_some() {
        files.extend(service::redeem_files(aso, exe, config)?);
    }
    Ok(files)
}

/// The redeem job's files, whether or not `redeem.payout` is set now.
pub fn redeem_job_files(aso: &Aso, exe: &Path, config: &Path) -> Result<Vec<PathBuf>, String> {
    Ok(service::redeem_files(aso, exe, config)?
        .into_iter()
        .map(|(p, _)| p)
        .collect())
}

/// Write `text` to `path` (mode 0644: nothing aso generates holds a secret)
/// unless the file already has exactly that text. Returns whether it wrote.
pub fn write_if_changed(path: &Path, text: &str) -> Result<bool, String> {
    if fs::read_to_string(path).is_ok_and(|have| have == text) {
        return Ok(false);
    }
    state::replace_file(path, text.as_bytes())?;
    crate::perm::set_mode(path, 0o644)?;
    Ok(true)
}

/// Write the files that changed. Returns the paths written.
pub fn write_changed(files: &[(PathBuf, String)]) -> Result<Vec<PathBuf>, String> {
    let mut written = Vec::new();
    for (path, text) in files {
        if write_if_changed(path, text)? {
            written.push(path.clone());
        }
    }
    Ok(written)
}
