//! `aso audit`: the mint's supply audit and the wallet's balance, together.
//!
//! The mint audit is `fsp-mint audit`, which reads the data files, writes
//! nothing and works while `serve` runs; aso also runs it with `--json` to read
//! the verdict (`consistent`), because `fsp-mint audit` prints a WARNING and
//! still exits 0 when a key set has spent more than it issued. The wallet
//! balance is `ecash-wallet balance` (PSP, ESP by service and day, FSP). Nothing
//! is stopped, and nothing is written but the wallet's own start-up work: the
//! wallet picks up what is waiting for it, as it does at the start of any
//! command.

use crate::config::Aso;
use crate::exec::Exec;
use crate::fsp;
use crate::wallet;

/// What `aso audit` found.
#[derive(Debug, Default)]
pub struct Audit {
    pub lines: Vec<String>,
    /// False when the mint audit failed: the command exits non-zero.
    pub ok: bool,
}

/// The verdict in `fsp-mint audit --json`: `Ok(())` when `consistent` is true.
pub fn verdict(json: &str) -> Result<(), String> {
    let v: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| format!("`fsp-mint audit --json` did not print JSON: {e}"))?;
    match v.get("consistent").and_then(|c| c.as_bool()) {
        Some(true) => Ok(()),
        Some(false) => {
            let warnings: Vec<String> = v
                .get("warnings")
                .and_then(|w| w.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|w| w.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            Err(format!(
                "the mint audit is not consistent: {}",
                if warnings.is_empty() {
                    "see the audit above".to_string()
                } else {
                    warnings.join("; ")
                }
            ))
        }
        None => Err("`fsp-mint audit --json` has no `consistent` field".into()),
    }
}

pub fn run(aso: &Aso, exec: &dyn Exec) -> Audit {
    let mut out = Audit {
        lines: Vec::new(),
        ok: true,
    };
    out.lines.push("== mint audit (fsp-mint audit) ==".into());
    match fsp::run(aso, exec, &["audit".to_string()]) {
        Err(e) => {
            out.ok = false;
            out.lines.push(format!("FAIL: {e}"));
        }
        Ok(ran) if !ran.ok() => {
            out.ok = false;
            out.lines.push(format!(
                "FAIL: `fsp-mint audit` failed: {}",
                ran.last_line()
            ));
        }
        Ok(ran) => {
            out.lines.extend(ran.stdout.lines().map(str::to_string));
            match fsp::run(aso, exec, &["audit".to_string(), "--json".to_string()]) {
                Ok(j) if j.ok() => {
                    if let Err(why) = verdict(&j.stdout) {
                        out.ok = false;
                        out.lines.push(format!("FAIL: {why}"));
                    } else {
                        out.lines.push("mint audit: consistent".into());
                    }
                }
                Ok(j) => {
                    out.ok = false;
                    out.lines.push(format!(
                        "FAIL: `fsp-mint audit --json` failed: {}",
                        j.last_line()
                    ));
                }
                Err(e) => {
                    out.ok = false;
                    out.lines.push(format!("FAIL: {e}"));
                }
            }
        }
    }
    out.lines.push(String::new());
    out.lines
        .push("== wallet balance (ecash-wallet balance) ==".into());
    match wallet::run_ok(aso, exec, &["balance".to_string()]) {
        Ok(ran) => out.lines.extend(ran.stdout.lines().map(str::to_string)),
        Err(e) => out
            .lines
            .push(format!("the wallet balance could not be read: {e}")),
    }
    out
}
