//! `aso versions`: the version of every program and library the service runs.
//!
//! - **aso**: its own version. aso links no family crate, so it has no library
//!   versions of its own.
//! - **ecash-wallet**: `ecash-wallet --version`, whose first line is the program
//!   and whose other lines are the libraries it was built with (sp-wire,
//!   mint-core, air), shown as `ecash-wallet/sp-wire` and so on. They are exact
//!   when it was built with `--locked`.
//! - **fsp-mint** (0.8.0 too) and **air_server** print no version. aso therefore keeps
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

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::config::Aso;
use crate::exec::Exec;
use crate::program::Program;
use crate::state;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Line {
    pub name: String,
    /// `x.y.z`, or `unknown`.
    pub version: String,
    /// Where the version came from, or why it is unknown.
    pub note: String,
}

fn known(name: &str, version: &str, note: &str) -> Line {
    Line {
        name: name.into(),
        version: version.into(),
        note: note.into(),
    }
}

fn unknown(name: &str, why: impl Into<String>) -> Line {
    known(name, "unknown", &why.into())
}

/// The program's own version from the first line of its `--version`:
/// `<name> <version>`.
pub fn program_version(name: &str, first_line: &str) -> Option<String> {
    let mut words = first_line.split_whitespace();
    (words.next()? == name).then(|| words.next().map(str::to_string))?
}

/// Whether two versions are the same number: `1.0` and `1.0.0` are.
pub fn same_version(a: &str, b: &str) -> bool {
    let parts = |v: &str| -> Option<Vec<u64>> {
        let mut n: Vec<u64> = v
            .split('.')
            .map(|p| p.parse().ok())
            .collect::<Option<Vec<_>>>()?;
        while n.len() < 3 {
            n.push(0);
        }
        Some(n)
    };
    match (parts(a), parts(b)) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

/// The version a program reports for the binary at `path`, or why it cannot.
/// ecash-wallet says so itself; fsp-mint and air_server only through aso's
/// record of that exact file.
pub fn reported(
    aso: &Aso,
    exec: &dyn Exec,
    program: Program,
    path: &Path,
) -> Result<String, String> {
    if program != Program::Wallet {
        return state::recorded_version(aso, program, path).ok_or_else(|| {
            format!(
                "aso has no record of the version of {} ({} prints no version)",
                path.display(),
                program.name()
            )
        });
    }
    let ran = exec.run(&path.display().to_string(), &["--version".to_string()], &[])?;
    if !ran.ok() {
        return Err(format!(
            "`{} --version` failed: {}",
            path.display(),
            ran.last_line()
        ));
    }
    let first = ran.stdout.lines().next().unwrap_or("");
    program_version(program.name(), first).ok_or_else(|| {
        format!(
            "`{} --version` printed `{first}`, which has no version",
            path.display()
        )
    })
}

/// The binary `spec` names (a path, or a name on PATH), as an absolute path.
pub fn find_binary(spec: &str) -> Option<PathBuf> {
    let found = if spec.contains('/') {
        let p = PathBuf::from(spec);
        p.is_file().then_some(p)
    } else {
        let path = std::env::var_os("PATH")?;
        std::env::split_paths(&path)
            .map(|d| d.join(spec))
            .find(|p| p.is_file())
    };
    found.and_then(|p| std::fs::canonicalize(p).ok())
}

fn recorded_line(aso: &Aso, program: Program, label: &str, binary: &Path) -> Line {
    let sha = match state::sha256_file(binary) {
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
    let mut out = vec![known(
        "aso",
        env!("CARGO_PKG_VERSION"),
        "this program (it links none of the family crates)",
    )];
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

/// Text for a person.
pub fn render(lines: &[Line]) -> String {
    let mut out = String::new();
    for l in lines {
        out.push_str(&format!("{:<32} {:<10} {}\n", l.name, l.version, l.note));
    }
    out
}
