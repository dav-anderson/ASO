//! Getting a program's binary and putting it in place: used by `aso install` and
//! `aso upgrade`.
//!
//! **The target.** A program's `version` in aso.toml is an optional pin. With no
//! pin the target is the newest tagged release the release source knows; with a
//! pin it is the pin, which may be older than the newest, and a newer release is
//! reported ("pinned at x (newest y)"), never installed. air_server publishes no
//! releases, so its `bin` is the operator's file and its `version` the
//! operator's word for it.
//!
//! **The build.** When the release source has the target, aso downloads the
//! release binary, checks its SHA-256 against the one the release publishes and
//! makes it executable; a download that fails the check is deleted and nothing is
//! changed. Otherwise the build is the file `bin` names. aso never builds from
//! source, and never installs a release candidate.
//!
//! **Putting it in place** is two renames in `bin_dir`: the new file is copied
//! beside the installed one as `<name>.new`, the installed one becomes
//! `<name>.previous`, and `<name>.new` becomes the installed one. Both steps are
//! atomic on one filesystem, and everything that can fail (the download, the
//! checks, the copy) happens before the first rename and before any program is
//! stopped.

use std::fs;
use std::path::{Path, PathBuf};

use crate::config::Aso;
use crate::exec::Exec;
use crate::program::Program;
use crate::release::{self, Source};
use crate::state;
use crate::versions::{self, find_binary, same_version};

/// What aso will install for a program, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decision {
    pub target: String,
    /// "pinned at x (newest y)", when a newer release exists than the pin.
    pub pin_note: Option<String>,
    /// Whether the build is downloaded from the release source (else `bin`).
    pub from_release: bool,
}

/// The target of one program, or why there is none.
pub fn decide(aso: &Aso, program: Program, source: &dyn Source) -> Result<Decision, String> {
    let name = program.name();
    let pin = program.pin(aso);
    let newest = if program.publishes() {
        release::newest_of(source, program)
    } else {
        Ok(None)
    };
    match (pin, newest) {
        (Some(pin), Ok(Some(newest))) => Ok(Decision {
            pin_note: release::is_newer(&newest, &pin)
                .then(|| format!("{name}: pinned at {pin} (newest {newest})")),
            target: pin,
            from_release: true,
        }),
        (Some(pin), _) => Ok(Decision {
            target: pin,
            pin_note: None,
            from_release: false,
        }),
        (None, Ok(Some(newest))) => Ok(Decision {
            target: newest,
            pin_note: None,
            from_release: true,
        }),
        (None, Ok(None)) if !program.publishes() => Err(format!(
            "aso.toml names no version for {name} (set `version` under [{}]: {name} publishes no releases, so the version is your word for the file `bin` names)",
            program.section()
        )),
        (None, Ok(None)) => Err(format!(
            "aso.toml names no version for {name}, and the release source lists no release for it (set `version` under [{}] to use the file `bin` names)",
            program.section()
        )),
        (None, Err(e)) => Err(format!(
            "aso.toml names no version for {name}, and the newest release could not be found: {e}"
        )),
    }
}

/// The build to install: downloaded and verified, or the one `bin` names. The
/// result is a file that is not the installed one.
pub fn obtain(
    aso: &Aso,
    exec: &dyn Exec,
    program: Program,
    d: &Decision,
    source: &dyn Source,
) -> Result<PathBuf, String> {
    let name = program.name();
    let target = &d.target;
    let installed = program.installed_path(aso);
    let new = if d.from_release {
        let dir = release::releases_dir(aso, program, target);
        let got = source
            .fetch(program, target, &dir)
            .map_err(|e| format!("cannot get {name} {target} from the release source: {e}"))?;
        let have = state::sha256_file(&got.file)?;
        if !have.eq_ignore_ascii_case(&got.sha256) {
            let _ = fs::remove_file(&got.file);
            return Err(format!(
                "the download of {name} {target} failed its checksum (the release says {}, the file is {have}): it was deleted and nothing was changed",
                got.sha256
            ));
        }
        release::make_executable(&got.file)?;
        got.file
    } else {
        let Some(new) = find_binary(&program.bin(aso)) else {
            return Err(format!(
                "cannot find the build `{}` ({}.bin in aso.toml names it)",
                program.bin(aso),
                program.section()
            ));
        };
        if fs::canonicalize(&installed).ok().as_deref() == Some(new.as_path()) {
            return Err(format!(
                "`{}.bin` names the installed file {} itself: point it at the new build",
                program.section(),
                installed.display()
            ));
        }
        new
    };
    // The new build must be what is wanted. ecash-wallet says what it is.
    // fsp-mint and air_server cannot, so they are taken as named (a release's
    // checksum was checked above), unless it is the very file that is installed.
    if program == Program::Wallet {
        match versions::reported(aso, exec, program, &new) {
            Ok(found) if same_version(&found, target) => {}
            Ok(found) => {
                return Err(format!(
                    "{} is {found}, but {} names {target}: nothing was changed",
                    new.display(),
                    if d.from_release {
                        "the release"
                    } else {
                        "aso.toml"
                    }
                ));
            }
            Err(e) => return Err(format!("nothing was changed: {e}")),
        }
    } else if !d.from_release
        && installed.is_file()
        && state::sha256_file(&new).ok() == state::sha256_file(&installed).ok()
    {
        return Err(format!(
            "the new build {} is the same file as the installed one, not {target}",
            new.display()
        ));
    }
    Ok(new)
}

fn staged(installed: &Path) -> PathBuf {
    let name = installed
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    installed.with_file_name(format!("{name}.new"))
}

pub fn previous(installed: &Path) -> PathBuf {
    let name = installed
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    installed.with_file_name(format!("{name}.previous"))
}

/// Copy `new` beside `installed`, keep the installed one as `.previous`, and put
/// the copy in its place. Returns the `.previous` file when there was an
/// installed one to keep.
pub fn swap_in(new: &Path, installed: &Path) -> Result<Option<PathBuf>, String> {
    let staged = staged(installed);
    let previous = previous(installed);
    if let Some(dir) = installed.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    fs::copy(new, &staged)
        .map_err(|e| format!("cannot write {}: {e} (run aso as root)", staged.display()))?;
    release::make_executable(&staged).inspect_err(|_| {
        let _ = fs::remove_file(&staged);
    })?;
    let had = installed.is_file();
    if had && let Err(e) = fs::rename(installed, &previous) {
        let _ = fs::remove_file(&staged);
        return Err(format!(
            "cannot keep {} as {}: {e}",
            installed.display(),
            previous.display()
        ));
    }
    if let Err(e) = fs::rename(&staged, installed) {
        if had {
            let _ = fs::rename(&previous, installed);
        }
        let _ = fs::remove_file(&staged);
        return Err(format!("cannot put {} in place: {e}", installed.display()));
    }
    Ok(had.then_some(previous))
}

/// Put the `.previous` file back as the installed one.
pub fn put_back(installed: &Path) -> Result<(), String> {
    let previous = previous(installed);
    fs::rename(&previous, installed).map_err(|e| {
        format!(
            "cannot put {} back as {}: {e}",
            previous.display(),
            installed.display()
        )
    })
}

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
        None => match fs::remove_file(state::record_path(aso, program)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("cannot remove the old record: {e}")),
        },
    }
}

/// What `aso install` does for a binary: obtain the target and put it in place
/// unless that exact file is already installed at that version. Returns the
/// lines to print.
pub fn install_binary(
    aso: &Aso,
    exec: &dyn Exec,
    program: Program,
    source: &dyn Source,
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
    if installed.is_file() && state::sha256_file(&new).ok() == state::sha256_file(&installed).ok() {
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
    swap_in(&new, &installed)?;
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
