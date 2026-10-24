//! The automatic ESP redeem: what the service earns is paid out with no operator
//! command.
//!
//! One run does, in order:
//!
//! 1. `fsp-mint export-esp --out <hand-over folder> --serve-only`: fsp-mint,
//!    still serving, writes one hand-over file per redeemable-after day for the
//!    ESP it has kept and not yet exported. It marks each note exported, so a
//!    second run gets nothing it already gave. With `--serve-only` the command
//!    only asks the running serve and fails if none answers: it never takes the
//!    data folder's lock or opens its files, so the redeem job (root) cannot
//!    leave a file in fsp-mint's data folder that the service account cannot
//!    write. aso asks only an fsp-mint it recorded as 0.8.0 or later (older ones
//!    have no `--serve-only` and would do the export themselves).
//! 2. `ecash-wallet receive FILE` for every file in the hand-over folder (also
//!    one left by an earlier run whose `receive` failed), and the file is
//!    deleted as soon as the wallet has received it. A hand-over file holds the
//!    notes' spend secrets, so the folder is owner-only (0700) and fsp-mint
//!    writes the files 0600.
//! 3. `ecash-wallet redeem --service-key-file KEY --resume ID` for every
//!    redemption the wallet shows as sent and not answered (a lost reply).
//! 4. `ecash-wallet redeem --service-key-file KEY --memo PAYOUT`: every free ESP
//!    note of the service whose redeemable-after day has come. If the wallet says
//!    the outcome is unknown, the same request is sent again with `--resume ID`.
//! 5. `ecash-wallet redeem --collect`: asks the mint about every redemption
//!    queued for payout; completed ones end, declined ones give the notes back.
//!    A payout the operator has not made yet stays queued, and the next run asks
//!    again.
//!
//! **Trigger and payout (an ASK; this is the proposal).** `[redeem] payout` in
//! aso.toml (for example `btc:ADDRESS`) turns it on; unset, there is no
//! automatic redeem and no job is written. `[redeem] interval_hours` (24) is how
//! often the job runs. `aso install` asks for the payout.
//!
//! **fsp-mint is never stopped by any of this**: step 1 asks the running serve.

use std::fs;
use std::path::{Path, PathBuf};

use air_interface::Managed;
use air_interface::version::Tags;

use crate::config::Aso;
use crate::ctl;
use crate::exec::Exec;
use crate::fsp;
use crate::program::Program;
use crate::state;
use crate::wallet;

/// What `ecash-wallet redeem` says (redeem.rs:729 of ecash-wallet 0.7.0) when
/// there is nothing to redeem yet.
pub const NOTHING_TO_REDEEM: &str = "holds no free ESP for that service";
/// What `ecash-wallet redeem --collect` prints when nothing is queued.
pub const NOTHING_QUEUED: &str = "no redemption is waiting for a payout";

pub fn handover_dir(aso: &Aso) -> PathBuf {
    aso.data_dir().join("redeem").join("handover")
}

/// What one run did.
#[derive(Debug, Default)]
pub struct Run {
    pub lines: Vec<String>,
    /// False when a step failed; the next run tries again.
    pub ok: bool,
    /// The hand-over files the wallet received this run.
    pub received: usize,
}

/// The ids after each `--resume ` in `text`.
pub fn resume_ids(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("--resume ") {
        let after = &rest[at + "--resume ".len()..];
        let id: String = after.chars().take_while(|c| !c.is_whitespace()).collect();
        if !id.is_empty() && !out.contains(&id) {
            out.push(id);
        }
        rest = after;
    }
    out
}

fn make_private_dir(dir: &Path) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    air_interface::files::set_mode(dir, 0o700)
}

/// Why the installed fsp-mint must not be asked to export, if it must not: it
/// is older than 0.8.0 (no `export-esp --serve-only`), or aso has no record of
/// this exact binary and so cannot tell.
fn fsp_too_old(aso: &Aso) -> Option<String> {
    let (min, why) = Program::Fsp.min_version()?;
    let binary = Program::Fsp.installed_path(aso);
    match state::recorded_version(aso, Program::Fsp, &binary) {
        Some(v) if !Tags::Strict.is_newer(min, &v) => None,
        Some(v) => Some(format!(
            "export-esp skipped: the installed fsp-mint is {v}, and aso needs {min} or later (`{why}`), so that the export is always done by the running fsp-mint. Run `aso upgrade fsp-mint`"
        )),
        None => Some(format!(
            "export-esp skipped: aso has no record of {} (fsp-mint prints no version), so it cannot tell that it has `{why}` ({min} or later). Run `aso install`",
            binary.display()
        )),
    }
}

/// Run the chain once. `log` gets each line as it is made.
pub fn run(aso: &Aso, exec: &dyn Exec, log: &mut dyn FnMut(&str)) -> Run {
    let mut out = Run {
        ok: true,
        ..Run::default()
    };
    let mut say = |out: &mut Run, text: String| {
        log(&text);
        out.lines.push(text);
    };
    let Some(payout) = aso.opt("redeem.payout") else {
        say(
            &mut out,
            "the automatic redeem is off: redeem.payout is not set in aso.toml".into(),
        );
        return out;
    };
    let key = aso.service_key_file().display().to_string();
    if !aso.service_key_file().is_file() {
        out.ok = false;
        say(
            &mut out,
            format!("FAIL: the service key {key} does not exist: run `aso install` first"),
        );
        return out;
    }
    let dir = handover_dir(aso);
    if let Err(e) = make_private_dir(&dir) {
        out.ok = false;
        say(&mut out, format!("FAIL: {e}"));
        return out;
    }

    // 1. export (fsp-mint keeps serving). Only the running fsp-mint writes its
    // data folder: `--serve-only` asks it and fails if nothing answers, so
    // `export-esp` never does the export itself as whoever runs aso (root), which
    // would leave files the service account cannot write. An fsp-mint older than
    // 0.8.0 has no `--serve-only`, so it is not asked at all.
    let up = ctl::health(aso, Program::Fsp);
    let refusal = if !up.up {
        Some(format!(
            "export-esp skipped: fsp-mint is not answering its health check ({}). aso asks the running fsp-mint to export and never does the export itself",
            up.detail
        ))
    } else {
        fsp_too_old(aso)
    };
    if let Some(why) = refusal {
        out.ok = false;
        say(&mut out, why);
    } else {
        let export = fsp::run(
            aso,
            exec,
            &[
                "export-esp".to_string(),
                "--out".to_string(),
                dir.display().to_string(),
                "--serve-only".to_string(),
            ],
        );
        match export {
            Ok(ran) if ran.ok() => say(&mut out, "export-esp: done".into()),
            Ok(ran) => {
                out.ok = false;
                say(
                    &mut out,
                    format!("FAIL: export-esp failed: {}", ran.last_line()),
                );
            }
            Err(e) => {
                out.ok = false;
                say(&mut out, format!("FAIL: {e}"));
            }
        }
    }

    // 2. receive every file in the folder, then delete it.
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    files.sort();
    for file in files {
        let name = file.display().to_string();
        match wallet::run(aso, exec, &["receive".to_string(), name.clone()]) {
            Ok(ran) if ran.ok() => {
                out.received += 1;
                match fs::remove_file(&file) {
                    Ok(()) => say(&mut out, format!("receive: {name} taken in and deleted")),
                    Err(e) => {
                        out.ok = false;
                        say(
                            &mut out,
                            format!("FAIL: {name} was received but cannot be deleted: {e}"),
                        );
                    }
                }
            }
            Ok(ran) => {
                out.ok = false;
                say(
                    &mut out,
                    format!(
                        "FAIL: receive {name} failed: {} (the file is kept and tried again next run)",
                        ran.last_line()
                    ),
                );
            }
            Err(e) => {
                out.ok = false;
                say(&mut out, format!("FAIL: {e}"));
            }
        }
    }

    // 3. send again what the wallet shows as sent and unanswered.
    let mut resumed: Vec<String> = Vec::new();
    match wallet::balance(aso, exec) {
        Ok(b) => {
            for id in b.sent {
                resume(aso, exec, &key, &id, &mut out, &mut say);
                resumed.push(id);
            }
        }
        Err(e) => {
            out.ok = false;
            say(&mut out, format!("FAIL: {e}"));
        }
    }

    // 4. redeem what has matured.
    let ran = wallet::run(
        aso,
        exec,
        &[
            "redeem".to_string(),
            "--service-key-file".to_string(),
            key.clone(),
            "--memo".to_string(),
            payout,
        ],
    );
    match ran {
        Ok(r) if r.ok() => say(&mut out, "redeem: sent".into()),
        Ok(r) if r.stderr.contains(NOTHING_TO_REDEEM) => {
            say(&mut out, "redeem: nothing has matured yet".into())
        }
        Ok(r) => {
            out.ok = false;
            say(&mut out, format!("redeem: {}", r.last_line()));
            for id in resume_ids(&r.stderr) {
                if !resumed.contains(&id) {
                    resume(aso, exec, &key, &id, &mut out, &mut say);
                }
            }
        }
        Err(e) => {
            out.ok = false;
            say(&mut out, format!("FAIL: {e}"));
        }
    }

    // 5. collect.
    match wallet::run(aso, exec, &["redeem".to_string(), "--collect".to_string()]) {
        Ok(r) if r.ok() && r.stdout.contains(NOTHING_QUEUED) => {
            say(&mut out, "collect: no payout is waiting".into())
        }
        Ok(r) if r.ok() => say(
            &mut out,
            "collect: asked the mint about the queued payouts".into(),
        ),
        Ok(r) => {
            out.ok = false;
            say(
                &mut out,
                format!("FAIL: redeem --collect failed: {}", r.last_line()),
            );
        }
        Err(e) => {
            out.ok = false;
            say(&mut out, format!("FAIL: {e}"));
        }
    }
    if let Ok(b) = wallet::balance(aso, exec) {
        for id in b.queued {
            say(
                &mut out,
                format!("redemption {id}: payout queued at the mint; the next run asks again"),
            );
        }
    }
    out
}

fn resume(
    aso: &Aso,
    exec: &dyn Exec,
    key: &str,
    id: &str,
    out: &mut Run,
    say: &mut dyn FnMut(&mut Run, String),
) {
    let ran = wallet::run(
        aso,
        exec,
        &[
            "redeem".to_string(),
            "--service-key-file".to_string(),
            key.to_string(),
            "--resume".to_string(),
            id.to_string(),
        ],
    );
    match ran {
        Ok(r) if r.ok() => say(out, format!("redeem --resume {id}: sent again")),
        Ok(r) => {
            out.ok = false;
            say(
                out,
                format!(
                    "FAIL: redeem --resume {id}: {} (tried again next run)",
                    r.last_line()
                ),
            );
        }
        Err(e) => {
            out.ok = false;
            say(out, format!("FAIL: {e}"));
        }
    }
}
