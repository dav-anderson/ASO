//! Running the installed fsp-mint's commands (`audit`, `export-esp`,
//! `service-key`). The long-running `serve` is the service manager's job
//! ([`crate::ctl`]), never aso's.

use crate::config::Aso;
use crate::exec::{Exec, Ran};
use crate::program::Program;
use crate::versions::find_binary;

/// The fsp-mint binary: the installed copy, else the file `fsp.bin` names.
pub fn binary(aso: &Aso) -> String {
    let installed = Program::Fsp.installed_path(aso);
    if installed.is_file() {
        return installed.display().to_string();
    }
    find_binary(&Program::Fsp.bin(aso))
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| Program::Fsp.bin(aso))
}

/// The environment fsp-mint reads: where its data and its settings file are, as
/// the service has them.
pub fn env(aso: &Aso) -> Vec<(String, String)> {
    vec![
        (
            "FSP_DATA_DIR".to_string(),
            aso.fsp_base().display().to_string(),
        ),
        (
            "FSP_CONFIG".to_string(),
            aso.fsp_config_file().display().to_string(),
        ),
    ]
}

/// Run `fsp-mint <args>`. `Err` only when it cannot be run.
pub fn run(aso: &Aso, exec: &dyn Exec, args: &[String]) -> Result<Ran, String> {
    exec.run(&binary(aso), args, &env(aso))
}
