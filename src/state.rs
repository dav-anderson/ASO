//! What aso remembers between commands, in `<aso folder>/state.json`, and what it
//! recorded when it installed a program (`installed/<program>.json`).
//!
//! `state.json` holds: the AIR server's name (read from air_server's first
//! start), the service DID (printed by `fsp-mint service-key`), whether step (d)
//! of the install was done or skipped, and the values of the settings that need
//! a restart as they were at the last start or restart (`applied`), which
//! `aso reload` compares with `aso.toml` to name what is waiting for a restart.
//! Every write replaces the whole file at once (a temporary file, then a rename).

use std::fs;
use std::path::{Path, PathBuf};

use air_interface::Managed;
use air_interface::files::replace_file;
use air_interface::record::{self, Installed};
use serde::{Deserialize, Serialize};

use crate::config::Aso;
use crate::program::Program;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub air_name: String,
    #[serde(default)]
    pub service_did: String,
    /// `""` (not done), `"done"` or `"skipped"`: install step (d).
    #[serde(default)]
    pub backup: String,
    #[serde(default)]
    pub applied: Vec<(String, String)>,
}

pub fn state_path(aso: &Aso) -> PathBuf {
    aso.dir.join("state.json")
}

pub fn read(aso: &Aso) -> Result<State, String> {
    let path = state_path(aso);
    match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| format!("{} is damaged: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(State::default()),
        Err(e) => Err(format!("cannot read {}: {e}", path.display())),
    }
}

pub fn write(aso: &Aso, state: &State) -> Result<(), String> {
    let text = serde_json::to_string_pretty(state).map_err(|e| e.to_string())?;
    replace_file(&state_path(aso), format!("{text}\n").as_bytes())
}

/// What aso recorded of a program it put in place: the version and the SHA-256
/// of the file ([`air_interface::record`]). fsp-mint (0.8.0 too) and air_server
/// print no version, so this record is the only way aso knows what they are.
pub fn record_path(aso: &Aso, program: Program) -> PathBuf {
    aso.dir
        .join("installed")
        .join(format!("{}.json", program.name()))
}

/// Write the record: `binary` is the file now installed and `version` what it is.
pub fn record_installed(
    aso: &Aso,
    program: Program,
    version: &str,
    binary: &Path,
) -> Result<(), String> {
    record::write(&record_path(aso, program), version, binary)
}

pub fn read_record(aso: &Aso, program: Program) -> Option<Installed> {
    record::read(&record_path(aso, program))
}

/// The version aso recorded for exactly this file, if it did.
pub fn recorded_version(aso: &Aso, program: Program, binary: &Path) -> Option<String> {
    record::recorded(&record_path(aso, program), binary)
}

/// Whether a restart of `program` applies the setting `key`: air_server's
/// settings are `air.*` (the AIR server's), fsp-mint's are `fsp.*` and the two
/// the generated fsp-mint settings come from (`air.name`, `air.host`); the
/// folders and accounts (`service.*`) belong to both.
pub fn owned_by(key: &str, program: Program) -> bool {
    match program {
        Program::Air => key.starts_with("air.") || key.starts_with("service."),
        Program::Fsp => {
            key.starts_with("fsp.")
                || key == "air.name"
                || key == "air.host"
                || key.starts_with("service.")
        }
        Program::Wallet => false,
    }
}

/// Record that `program` was started or restarted with the settings now in
/// `aso.toml`: the values of the restart-only settings it owns become the ones
/// `aso reload` compares against.
pub fn mark_applied(aso: &Aso, program: Program) -> Result<(), String> {
    let mut state = read(aso)?;
    for (key, value) in aso.restart_values() {
        if !owned_by(&key, program) {
            continue;
        }
        match state.applied.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = value,
            None => state.applied.push((key, value)),
        }
    }
    write(aso, &state)
}
