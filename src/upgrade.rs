//! `aso upgrade`: bring one or all of air_server, fsp-mint and ecash-wallet to
//! their target version, one at a time.
//!
//! **Brief downtime.** Every upgrade of a service is a restart of the program on
//! the service install: the old process stops, the new build starts. Nothing is
//! upgraded while it runs. [`Program::method`] says, for each program, what the
//! downtime is and why. (Upgrades with no downtime are not built; see ROADMAP
//! item 25.)
//!
//! **Keeping the downtime short.** Everything that can fail or take time happens
//! before the program is touched: the release is downloaded and its checksum
//! checked, the version is read, the binary is copied beside the installed one.
//! The stop and the start are then one service-manager call, with the two
//! renames that swap the binary just before it; aso then polls the health
//! listener every half second and reports how long the program was down.
//! If the new build does not come up healthy, the old binary is put back and
//! started.
//!
//! **The target** is decided in [`crate::binary::decide`]: the pin, else the
//! newest tagged release; never a release candidate.
//!
//! The order is air_server, then fsp-mint (the start order), then ecash-wallet.
//! The next program is not touched until the one before is healthy. A program
//! already at its target is left alone and nothing is stopped.

use std::time::{Duration, Instant};

use crate::binary::{self, Decision};
use crate::config::Aso;
use crate::ctl::{self, Action};
use crate::exec::Exec;
use crate::program::Program;
use crate::release::{self, Source};
use crate::state;
use crate::versions::{self, same_version};

/// How an upgrade is run.
pub struct Options<'a> {
    pub dry_run: bool,
    /// How long each program gets to be healthy after its restart.
    pub wait_secs: u64,
    /// Where releases come from.
    pub source: &'a dyn Source,
}

/// What `upgrade` did.
#[derive(Debug, Default)]
pub struct Upgraded {
    pub lines: Vec<String>,
    pub ok: bool,
    /// The programs that were changed, or, in a dry run, would be.
    pub changes: Vec<Program>,
}

/// Poll `healthy` until it says yes, or `wait_secs` run out.
fn wait_for(healthy: &dyn Fn(Program) -> bool, program: Program, wait_secs: u64) -> bool {
    let end = Instant::now() + Duration::from_secs(wait_secs);
    loop {
        if healthy(program) {
            return true;
        }
        if Instant::now() >= end {
            return false;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// Upgrade `programs`, in the order given, one at a time. With `dry_run`
/// nothing is changed and each program's plan is said. The first failure stops
/// the run. `healthy` says whether a service answers its health check.
pub fn upgrade(
    aso: &Aso,
    programs: &[Program],
    opts: &Options,
    exec: &dyn Exec,
    healthy: &dyn Fn(Program) -> bool,
) -> Upgraded {
    let mut out = Upgraded {
        lines: Vec::new(),
        ok: true,
        changes: Vec::new(),
    };
    let mut already = 0usize;
    for &program in programs {
        let name = program.name();
        let decision = match binary::decide(aso, program, opts.source) {
            Ok(d) => d,
            Err(why) => {
                out.lines.push(format!("{name}: skipped: {why}"));
                continue;
            }
        };
        match upgrade_one(aso, program, &decision, opts, exec, healthy) {
            Ok(done) => {
                out.lines.extend(done.lines);
                if done.changed {
                    out.changes.push(program);
                } else {
                    already += 1;
                }
            }
            Err(lines) => {
                out.lines.extend(lines);
                out.ok = false;
                out.lines
                    .push("stopped: the programs after this one were not touched".into());
                return out;
            }
        }
    }
    if out.ok && out.changes.is_empty() && already == programs.len() {
        out.lines
            .push("everything up to date: nothing was stopped".into());
    }
    out
}

struct One {
    lines: Vec<String>,
    /// False when the program was already at its target.
    changed: bool,
}

fn upgrade_one(
    aso: &Aso,
    program: Program,
    d: &Decision,
    opts: &Options,
    exec: &dyn Exec,
    healthy: &dyn Fn(Program) -> bool,
) -> Result<One, Vec<String>> {
    let name = program.name();
    let target = d.target.as_str();
    let fail = |why: String| vec![format!("FAIL: {name}: {why}")];
    let mut lines = Vec::new();
    if let Some(note) = &d.pin_note {
        lines.push(note.clone());
    }
    let installed = program.installed_path(aso);
    if !installed.is_file() {
        return Err(fail(format!(
            "{} does not exist: `aso upgrade` upgrades the service install, so run `aso install` first",
            installed.display()
        )));
    }
    let current = versions::reported(aso, exec, program, &installed).ok();
    if current.as_deref().is_some_and(|c| same_version(c, target)) {
        lines.push(format!("{name}: already {target}: nothing to do"));
        return Ok(One {
            lines,
            changed: false,
        });
    }
    let from = current.unwrap_or_else(|| "an unknown version".into());
    // A dry run downloads nothing: it says where the build would come from.
    let new = if opts.dry_run && d.from_release {
        None
    } else {
        match binary::obtain(aso, exec, program, d, opts.source) {
            Ok(n) => Some(n),
            Err(why) => {
                lines.push(format!("FAIL: {name}: {why}"));
                return Err(lines);
            }
        }
    };
    lines.push(format!("{name}: {from} -> {target}: {}", program.method()));
    if opts.dry_run {
        if new.is_none() {
            lines.push(format!(
                "{name}: the build would be downloaded from the release source and checked against its SHA-256 before anything is stopped"
            ));
        }
        lines.push(format!("{name}: dry run: nothing was changed"));
        return Ok(One {
            lines,
            changed: true,
        });
    }
    let Some(new) = new else {
        return Err(fail("no build to install".into()));
    };

    let previous = match binary::swap_in(&new, &installed) {
        Ok(p) => p,
        Err(e) => {
            lines.push(format!("FAIL: {name}: {e}"));
            return Err(lines);
        }
    };
    lines.push(format!(
        "{name}: installed {}{}",
        installed.display(),
        previous
            .as_ref()
            .map(|p| format!(", the old one is {}", p.display()))
            .unwrap_or_default()
    ));

    if program.is_service() {
        let began = Instant::now();
        let restarted = ctl::apply(aso, exec, program, Action::Restart);
        let went = restarted.is_ok() && wait_for(healthy, program, opts.wait_secs);
        if !went {
            // Put the old binary back and start it.
            let why = match &restarted {
                Err(e) => e.clone(),
                Ok(()) => format!(
                    "the new build did not become healthy within {} s",
                    opts.wait_secs
                ),
            };
            let back = binary::put_back(&installed);
            let again = back.is_ok() && ctl::apply(aso, exec, program, Action::Restart).is_ok();
            lines.push(match (back.is_ok(), again) {
                (true, true) => format!("FAIL: {name}: {why}; the old one is back and restarted"),
                (true, false) => format!(
                    "FAIL: {name}: {why}, and the old one is back in place but could not be restarted: restart {} yourself",
                    program.unit()
                ),
                (false, _) => format!(
                    "FAIL: {name}: {why}, and the old one could not be put back: it is {}",
                    binary::previous(&installed).display()
                ),
            });
            return Err(lines);
        }
        lines.push(format!(
            "{name}: was briefly down while the new version started: back, healthy, after {:.1} s",
            began.elapsed().as_secs_f64()
        ));
        if let Err(e) = state::mark_applied(aso, program) {
            lines.push(format!("WARNING: {name}: {e}"));
        }
    }
    let version = Some(target);
    if let Err(e) = binary::record(aso, program, version, &installed) {
        lines.push(format!("WARNING: {name}: {e}"));
    }
    lines.push(format!("{name}: now {target}"));
    if d.from_release {
        match release::prune(aso, program, &[target, &from]) {
            Ok(gone) if !gone.is_empty() => lines.push(format!(
                "{name}: removed old downloads: {}",
                gone.join(", ")
            )),
            Ok(_) => {}
            Err(e) => lines.push(format!("WARNING: {name}: {e}")),
        }
    }
    Ok(One {
        lines,
        changed: true,
    })
}
