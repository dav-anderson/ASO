//! `aso versions`: the version of every program and library the service runs.
//!
//! - **aso**: its own version. aso links no family crate, so it has no library
//!   versions of its own.
//! - **air-minterface**: the version of the shared code aso uses to install,
//!   upgrade, schedule and log the programs (the same code spi uses).
//! - **ecash-wallet**: `ecash-wallet --version`, whose first line is the program
//!   and whose other lines are the libraries it was built with (sp-wire,
//!   mint-core, air), shown as `ecash-wallet/sp-wire` and so on. They are exact
//!   when it was built with `--locked`.
//! - **fsp-mint** (0.10.0 too) and **air_server** print no version. aso therefore keeps
//!   its own record: when `aso install` or `aso upgrade` puts a binary in place
//!   it writes the version (the release's, or the one `fsp.version` /
//!   `air.version` names) and the SHA-256 of the file to
//!   `<aso folder>/installed/<program>.json`. A binary whose hash matches the
//!   record has that version; one that does not is `unknown`; with no record the
//!   line says why. The libraries fsp-mint and air_server were built with cannot
//!   be read from them: those lines are `unknown`, with that reason (upstream:
//!   `--version` in fsp-mint, ROADMAP item 15 for version over AIR).
//!
//! Every line is a version or `unknown` with the reason. Nothing is started,
//! stopped or written.

use std::path::Path;

use air_minterface::exec::Exec;
use air_minterface::files::{find_binary, sha256_file};
use air_minterface::version::{Line, known, program_version, unknown};
use air_minterface::{Managed, version};

use crate::config::Aso;
use crate::program::Program;
use crate::state;

pub use air_minterface::version::render;

fn recorded_line(aso: &Aso, program: Program, label: &str, binary: &Path) -> Line {
    let sha = match sha256_file(binary) {
        Ok(s) => s,
        Err(e) => return unknown(label, e),
    };
    let name = program.name();
    match (state::read_record(aso, program), program.pin(aso)) {
        (Some(r), _) if r.sha256 == sha => known(
            label,
            &r.version,
            &format!("recorded by aso for this exact binary ({name} prints no version)"),
        ),
        (Some(r), _) => unknown(
            label,
            format!(
                "{} is not the binary aso recorded as {} (its SHA-256 differs), and {name} prints no version",
                binary.display(),
                r.version
            ),
        ),
        (None, Some(v)) => known(
            label,
            &v,
            &format!(
                "named in aso.toml, not checked: aso has no record of this binary and {name} prints no version (`aso install` records it)"
            ),
        ),
        (None, None) => unknown(
            label,
            format!(
                "{name} prints no version, aso has no record of installing it and its `version` is not set in aso.toml"
            ),
        ),
    }
}

/// Run `<bin> --version` and turn the answer into lines: the program, then each
/// library it was built with.
fn wallet_lines(exec: &dyn Exec, label: &str, bin: &str) -> Vec<Line> {
    let ran = match exec.run(bin, &["--version".to_string()], &[]) {
        Ok(r) => r,
        Err(e) => return vec![unknown(label, e)],
    };
    if !ran.ok() {
        return vec![unknown(
            label,
            format!("`{bin} --version` failed ({})", ran.last_line()),
        )];
    }
    let mut lines = ran.stdout.lines();
    let first = lines.next().unwrap_or("");
    let Some(version) = program_version("ecash-wallet", first) else {
        return vec![unknown(
            label,
            format!("`{bin} --version` printed `{first}`, not `ecash-wallet <version>`"),
        )];
    };
    let mut out = vec![known(label, &version, &format!("`{bin} --version`"))];
    for lib in lines {
        let mut words = lib.split_whitespace();
        if let (Some(n), Some(v)) = (words.next(), words.next()) {
            out.push(known(
                &format!("{label}/{n}"),
                v,
                "built in (from the program's Cargo.lock)",
            ));
        }
    }
    out
}

/// A service program's lines: its version, and the libraries it was built with,
/// which cannot be read from it.
fn service_lines(aso: &Aso, program: Program, label: &str, binary: &Path) -> Vec<Line> {
    let mut out = vec![recorded_line(aso, program, label, binary)];
    let libs: &[&str] = match program {
        Program::Fsp => &["sp-wire", "mint-core", "air"],
        _ => &["air"],
    };
    for lib in libs {
        out.push(unknown(
            &format!("{label}/{lib}"),
            format!(
                "{} prints no version, so the {lib} it was built with cannot be read from it",
                program.name()
            ),
        ));
    }
    out
}

/// Every program and library, in order.
pub fn versions(aso: &Aso, exec: &dyn Exec) -> Vec<Line> {
    let mut out = vec![
        known(
            "aso",
            env!("CARGO_PKG_VERSION"),
            "this program (it links none of the family crates)",
        ),
        version::crate_line(),
    ];
    for program in [Program::Fsp, Program::Wallet, Program::Air] {
        let name = program.name();
        let installed = program.installed_path(aso);
        let (label, path) = if installed.is_file() {
            (name.to_string(), Some(installed))
        } else {
            (
                format!("{name} (not installed)"),
                find_binary(&program.bin(aso)),
            )
        };
        match path {
            None => out.push(unknown(
                &label,
                format!(
                    "cannot find {} or `{}` ({}.bin)",
                    program.installed_path(aso).display(),
                    program.bin(aso),
                    program.section()
                ),
            )),
            Some(p) if program == Program::Wallet => {
                out.extend(wallet_lines(exec, &label, &p.display().to_string()));
            }
            Some(p) => out.extend(service_lines(aso, program, &label, &p)),
        }
    }
    out
}
