//! Getting a program's binary and putting it in place for `aso install`. The
//! engine that decides the target, obtains and checks the build and swaps it in
//! is air-interface's ([`air_interface::upgrade`]); this is the install's use of
//! it, and aso's record of what was installed.
//!
//! **The target.** A program's `version` in aso.toml is an optional pin. With no
//! pin the target is the newest tagged release the release source knows; with a
//! pin it is the pin, which may be older than the newest, and a newer release is
//! reported ("pinned at x (newest y)"), never installed. air_server publishes no
//! releases, so its `bin` is the operator's file and its `version` the
//! operator's word for it.

use std::path::Path;

use air_interface::exec::Exec;
use air_interface::files::{find_binary, sha256_file};
use air_interface::release::Source;
use air_interface::upgrade::{decide, obtain, swap_in};
use air_interface::version::same_version;
use air_interface::{Managed, record};

use crate::config::Aso;
use crate::program::Program;
use crate::state;

/// Record what is installed (version and SHA-256), or remove a record that no
/// longer describes the file when the version is not known.
pub fn record(
    aso: &Aso,
    program: Program,
    version: Option<&str>,
    installed: &Path,
) -> Result<(), String> {
    match version {
        Some(v) => state::record_installed(aso, program, v, installed),
        None => record::remove(&state::record_path(aso, program)),
    }
}

/// What `aso install` does for a binary: obtain the target and put it in place
/// unless that exact file is already installed at that version. Returns the
/// lines to print.
pub fn install_binary(
    aso: &Aso,
    exec: &dyn Exec,
    program: Program,
    source: &dyn Source<Program>,
) -> Result<Vec<String>, String> {
    let installed = program.installed_path(aso);
    let mut lines = Vec::new();
    // air_server's version is only the operator's word, and may be unset.
    let (new, version) = if program == Program::Air {
        let new = find_binary(&program.bin(aso)).ok_or_else(|| {
            format!(
                "cannot find air_server `{}` (air.bin in aso.toml names it): air_server publishes no release, so supply the binary",
                program.bin(aso)
            )
        })?;
        (new, program.pin(aso))
    } else {
        let d = decide(aso, program, source)?;
        if let Some(note) = &d.pin_note {
            lines.push(format!("WARNING: {note}"));
        }
        let version = d.target.clone();
        if installed.is_file()
            && state::recorded_version(aso, program, &installed)
                .is_some_and(|v| same_version(&v, &version))
        {
            lines.push(format!(
                "{}: {version} is installed already ({})",
                program.name(),
                installed.display()
            ));
            return Ok(lines);
        }
        (obtain(aso, exec, program, &d, source)?, Some(version))
    };
    if installed.is_file() && sha256_file(&new).ok() == sha256_file(&installed).ok() {
        lines.push(format!(
            "{}: {} is installed already",
            program.name(),
            installed.display()
        ));
        if let Some(v) = &version
            && state::recorded_version(aso, program, &installed).is_none()
        {
            record(aso, program, Some(v), &installed)?;
        }
        return Ok(lines);
    }
    swap_in(&new, &installed, "aso")?;
    record(aso, program, version.as_deref(), &installed)?;
    lines.push(format!(
        "{}: installed {} ({})",
        program.name(),
        installed.display(),
        version
            .as_deref()
            .unwrap_or("version unknown: set `version` in aso.toml to record it")
    ));
    Ok(lines)
}
