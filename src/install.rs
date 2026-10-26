//! `aso install`: the guided install, as four steps, and the smart rescan.
//!
//! a. **AIR service**: air_server (the operator's binary) behind nginx.
//! b. **FSP**: the service key, and `fsp-mint serve`.
//! c. **Wallet**: ecash-wallet installed and **funded**, by `buy-psp` then
//!    `claim-psp`, or by `handoff import` with the DID key file.
//! d. **Backups** (optional): ESP exchanged at another service and issued as
//!    FSP there, for the services that hold backups.
//!
//! **Smart rescan.** Every run starts with [`crate::status::collect`], which
//! checks what exists. A step whose facts all hold is not done again; for a step
//! that is not complete the run says what is missing and builds only that. Every
//! action is idempotent besides: a binary already installed is left, a file
//! with the text it should have is not written, a service that is running is not
//! restarted by the manager's `enable --now`. Funding is never repeated: the
//! wallet is bought into only when a funding option is given and the wallet is
//! not funded yet.
//!
//! **The DID key is the operator's to back up, offline.** `fsp-mint service-key`
//! makes `service_key.hex`; aso says where it is, before the service starts, and
//! makes no copy of it.

use std::fs;
use std::path::{Path, PathBuf};

use air_minterface::files::{backup_and_replace, set_mode};
use air_minterface::release::Source;

use crate::ask::Ask;
use crate::binary;
use crate::config::Aso;
use crate::ctl::{self, Action, Job};
use crate::edit;
use crate::exec::Exec;
use crate::files;
use crate::fsp;
use crate::generate;
use crate::outcome::Outcome;
use crate::program::Program;
use crate::service;
use crate::state;
use crate::status::{self, Step};
use crate::wallet;

/// What a command needs to act on the machine.
pub struct Ctx<'a> {
    pub aso: &'a Aso,
    pub exec: &'a dyn Exec,
    pub source: &'a dyn Source<Program>,
    /// The `aso` binary the units run.
    pub exe: PathBuf,
    /// The `aso.toml` this run loaded.
    pub config: PathBuf,
    /// Now, in seconds since the Unix epoch.
    pub now: i64,
}

/// How step c funds the wallet.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Funding {
    /// Check the balance and say what to do.
    #[default]
    None,
    /// `buy-psp --mint-key KEY`: prints the payload to pay once.
    Buy { mint_key: String },
    /// `claim-psp`: claim what was paid for, once the payment has its
    /// confirmations.
    Claim,
    /// `handoff import INPUT [--did-key-file FILE]`.
    Handoff {
        input: String,
        did_key_file: Option<String>,
    },
}

/// What `aso install` was told.
#[derive(Clone, Debug, Default)]
pub struct Options {
    pub funding: Funding,
    /// Step d: `(service DID, PSP amount)`.
    pub backup: Option<(String, String)>,
    /// Step d: skip it, and remember that.
    pub skip_backup: bool,
}

/// What to tell the operator about the DID key, before the service starts.
pub fn did_key_notice(aso: &Aso) -> Vec<String> {
    vec![
        format!(
            "BACK UP THE DID KEY, OFFLINE: {} (made by `fsp-mint service-key`).",
            aso.service_key_file().display()
        ),
        "  It is the key of the service's DID. Whoever holds it can redeem the ESP this service earns, and if it is lost the service and its ESP are lost. aso makes no copy of it: copy it to paper or to a drive that is not on this machine and keep that somewhere safe. This is the operator's job.".to_string(),
    ]
}

/// The AIR server's name in air_server's log: the first token that is 66 hex
/// characters starting with 02 or 03 (a compressed public key).
pub fn air_name_in(log: &str) -> Option<String> {
    log.split(|c: char| !c.is_ascii_alphanumeric())
        .find(|w| {
            w.len() == 66
                && (w.starts_with("02") || w.starts_with("03"))
                && w.chars().all(|c| c.is_ascii_hexdigit())
        })
        .map(str::to_lowercase)
}

/// Write every generated file whose text changed and tell the service manager.
/// A timed job whose files are among them is loaded (again) here, by whichever
/// call wrote them: the logs job, on from the first install (an install made
/// before it existed gets it at the next `aso install` or `aso reload`), and
/// the redeem job when `redeem.payout` is set. Loading the redeem job only in
/// [`reconcile_redeem`] missed it when an earlier call had written its files
/// (step a of an install, with the payout already in `aso.toml`). Returns the
/// paths written.
pub fn refresh(ctx: &Ctx) -> Result<Vec<PathBuf>, String> {
    let aso = ctx.aso;
    let st = state::read(aso)?;
    let all = files::all(aso, &st, &ctx.exe, &ctx.config)?;
    let written = files::write_changed(&all)?;
    if !written.is_empty() {
        ctl::define(aso, ctx.exec)?;
    }
    let logs: Vec<PathBuf> = service::logs_files(aso, &ctx.exe)?
        .into_iter()
        .map(|(p, _)| p)
        .collect();
    let mut jobs = vec![(service::LOGS_UNIT, logs)];
    if aso.opt("redeem.payout").is_some() {
        jobs.push((
            service::REDEEM_UNIT,
            files::redeem_job_files(aso, &ctx.exe, &ctx.config)?,
        ));
    }
    for (unit, paths) in jobs {
        if paths.iter().any(|p| written.contains(p)) {
            if !aso.mac {
                ctl::job(aso, ctx.exec, unit, Job::Enable)?;
            }
            ctl::job(aso, ctx.exec, unit, Job::Reload)?;
        }
    }
    Ok(written)
}

/// Make the redeem job match `aso.toml`: with `redeem.payout` set, say so when
/// [`refresh`] wrote (and so loaded) its files; unset, it is unloaded and its
/// files removed. Touches the timer only; no service is stopped.
pub fn reconcile_redeem(ctx: &Ctx, written: &[PathBuf]) -> Result<Vec<String>, String> {
    let aso = ctx.aso;
    let paths = files::redeem_job_files(aso, &ctx.exe, &ctx.config)?;
    let mut lines = Vec::new();
    if aso.opt("redeem.payout").is_some() {
        if paths.iter().any(|p| written.contains(p)) {
            lines.push(format!(
                "automatic redeem: on, every {} h",
                aso.whole("redeem.interval_hours")
            ));
        }
    } else if paths.iter().any(|p| p.is_file()) {
        ctl::job(aso, ctx.exec, service::REDEEM_UNIT, Job::Disable)?;
        for p in paths.iter().filter(|p| p.is_file()) {
            fs::remove_file(p).map_err(|e| format!("cannot remove {}: {e}", p.display()))?;
        }
        ctl::define(aso, ctx.exec)?;
        lines.push("automatic redeem: off (redeem.payout is not set); its job was removed".into());
    }
    Ok(lines)
}

fn chown(ctx: &Ctx, path: &Path) -> Result<(), String> {
    if ctx.aso.mac {
        return Ok(());
    }
    let owner = format!(
        "{}:{}",
        ctx.aso.text("service.user"),
        ctx.aso.text("service.group")
    );
    let ran = ctx.exec.run(
        "chown",
        &["-R".to_string(), owner, path.display().to_string()],
        &[],
    )?;
    if ran.ok() {
        Ok(())
    } else {
        Err(format!(
            "chown of {} failed: {}",
            path.display(),
            ran.last_line()
        ))
    }
}

/// The service account (Linux; on macOS the services run as the account that
/// ran `sudo aso install`) and the folders, with their owners and modes.
pub fn prepare(ctx: &Ctx) -> Result<Vec<String>, String> {
    let aso = ctx.aso;
    let mut lines = Vec::new();
    let (user, group) = (aso.text("service.user"), aso.text("service.group"));
    if !aso.mac && user != "root" {
        let known = ctx.exec.run("id", &["-u".to_string(), user.clone()], &[])?;
        if !known.ok() {
            let has_group = ctx
                .exec
                .run("getent", &["group".to_string(), group.clone()], &[])?;
            if !has_group.ok() {
                run_ok(ctx, "groupadd", &["--system", &group])?;
            }
            run_ok(
                ctx,
                "useradd",
                &[
                    "--system",
                    "--gid",
                    &group,
                    "--no-create-home",
                    "--home-dir",
                    &aso.data_dir().display().to_string(),
                    "--shell",
                    "/usr/sbin/nologin",
                    &user,
                ],
            )?;
            lines.push(format!("made the account {user} (group {group})"));
        }
    }
    for dir in [
        aso.bin_dir(),
        aso.etc_dir(),
        aso.data_dir(),
        aso.data_dir().join("air"),
        aso.fsp_base(),
        aso.log_dir(),
        aso.dir.join("installed"),
        aso.dir.join("releases"),
    ] {
        fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    // The wallet and the redeem hand-over belong to whoever runs aso and the
    // redeem job (root), and to no one else.
    for dir in [
        aso.data_dir().join("wallet"),
        aso.data_dir().join("redeem"),
        crate::redeem::handover_dir(aso),
    ] {
        fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        set_mode(&dir, 0o700)?;
    }
    for dir in [aso.data_dir().join("air"), aso.fsp_base(), aso.log_dir()] {
        chown(ctx, &dir)?;
    }
    Ok(lines)
}

fn run_ok(ctx: &Ctx, bin: &str, args: &[&str]) -> Result<(), String> {
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    let ran = ctx.exec.run(bin, &args, &[])?;
    if ran.ok() {
        Ok(())
    } else {
        Err(format!(
            "`{}` failed: {}",
            crate::exec::command_line(bin, &args),
            ran.last_line()
        ))
    }
}

/// Step a: air_server installed, its files written, nginx checked, started.
pub fn do_air(ctx: &Ctx) -> Result<Vec<String>, String> {
    let aso = ctx.aso;
    let mut lines = binary::install_binary(aso, ctx.exec, Program::Air, ctx.source)?;
    refresh(ctx)?;
    if let Some((path, _)) = service::nginx_file(aso)? {
        match ctx.exec.run("nginx", &["-t".to_string()], &[]) {
            Err(e) => lines.push(format!(
                "WARNING: {e}: check the nginx file yourself ({})",
                path.display()
            )),
            Ok(ran) if !ran.ok() => {
                return Err(format!(
                    "`nginx -t` failed: {}. The file aso wrote is {}",
                    ran.last_line(),
                    path.display()
                ));
            }
            Ok(_) => lines.push("nginx -t: ok".to_string()),
        }
        lines.push(format!(
            "nginx file: {}. Make nginx read it (put it in nginx's conf.d, or include it) and reload nginx: aso does not reload nginx",
            path.display()
        ));
    } else {
        lines.push("air.public_name is not set, so no nginx file was written".to_string());
    }
    ctl::apply(aso, ctx.exec, Program::Air, Action::Start)?;
    let wait = aso.whole("service.start_wait_secs");
    let h = ctl::wait_healthy(aso, Program::Air, wait);
    if !h.up {
        return Err(format!(
            "air_server was started but its health is not ok after {wait} s: {}. Its log is {}",
            h.detail,
            aso.log_dir().join(Program::Air.log_name()).display()
        ));
    }
    lines.push("air_server: started, health ok".to_string());
    let mut st = state::read(aso)?;
    if st.air_name.is_empty() && aso.opt("air.name").is_none() {
        let log =
            fs::read_to_string(aso.log_dir().join(Program::Air.log_name())).unwrap_or_default();
        match air_name_in(&log) {
            Some(name) => {
                lines.push(format!("the AIR server's name is {name}"));
                st.air_name = name;
                state::write(aso, &st)?;
                refresh(ctx)?;
            }
            None => lines.push(format!(
                "WARNING: the AIR server's name (its 66-character public key) is not in {}: set air.name in aso.toml",
                aso.log_dir().join(Program::Air.log_name()).display()
            )),
        }
    }
    state::mark_applied(aso, Program::Air)?;
    Ok(lines)
}

/// Step b: fsp-mint installed, the service key made, started.
pub fn do_fsp(ctx: &Ctx) -> Result<Vec<String>, String> {
    let aso = ctx.aso;
    let mut lines = binary::install_binary(aso, ctx.exec, Program::Fsp, ctx.source)?;
    let mut st = state::read(aso)?;
    if st.service_did.is_empty() || !aso.service_key_file().is_file() {
        let ran = fsp::run(aso, ctx.exec, &["service-key".to_string()])?;
        if !ran.ok() {
            return Err(format!(
                "`fsp-mint service-key` failed: {}",
                ran.last_line()
            ));
        }
        let did = ran
            .stdout
            .lines()
            .find_map(|l| l.trim().strip_prefix("service.did = "))
            .map(|d| d.trim().to_string())
            .filter(|d| !d.is_empty())
            .ok_or("`fsp-mint service-key` printed no `service.did = ` line")?;
        st.service_did = did.clone();
        state::write(aso, &st)?;
        chown(ctx, &aso.fsp_base())?;
        lines.push(format!("the service's DID is {did}"));
    }
    lines.extend(did_key_notice(aso));
    refresh(ctx)?;
    let missing = status::fsp_missing(aso);
    if !missing.is_empty() {
        return Err(format!(
            "fsp-mint serve refuses to start without {}: set them in aso.toml ([fsp.air] esp_did, esp_server, esp_host name the ESP mint, the SPI's, this service converts through), then run `aso install` again",
            missing.join(", ")
        ));
    }
    if !ctl::health(aso, Program::Air).up {
        return Err("air_server is not healthy: fsp-mint is started after it (step a)".into());
    }
    ctl::apply(aso, ctx.exec, Program::Fsp, Action::Start)?;
    let wait = aso.whole("service.start_wait_secs");
    let h = ctl::wait_healthy(aso, Program::Fsp, wait);
    if !h.up {
        return Err(format!(
            "fsp-mint was started but its health is not ok after {wait} s: {}. Its log is {}",
            h.detail,
            aso.log_dir().join(Program::Fsp.log_name()).display()
        ));
    }
    lines.push("fsp-mint: started, health ok".to_string());
    state::mark_applied(aso, Program::Fsp)?;
    match generate::fsp_entry(aso, &st) {
        Some(entry) => lines.push(format!(
            "Give wallets this entry for [air] fsp: {entry}"
        )),
        None => lines.push(
            "The `air.fsp` entry for wallets (<service DID>@<AIR server name>@<public host>) needs air.public_name in aso.toml"
                .to_string(),
        ),
    }
    Ok(lines)
}

/// Step c: ecash-wallet installed and funded.
pub fn do_wallet(ctx: &Ctx, funding: &Funding) -> Result<Vec<String>, String> {
    let aso = ctx.aso;
    let mut lines = binary::install_binary(aso, ctx.exec, Program::Wallet, ctx.source)?;
    let written = refresh(ctx)?;
    lines.extend(reconcile_redeem(ctx, &written)?);
    let before =
        wallet::balance(aso, ctx.exec).map_err(|e| format!("the wallet cannot be read: {e}"))?;
    if before.psp > 0 {
        lines.push(format!("the wallet is funded (PSP {})", before.psp));
        return Ok(lines);
    }
    let command: Vec<String> = match funding {
        Funding::None => {
            lines.push("The wallet holds no PSP. Fund it with one of: `aso install --buy-psp MINT_KEY` (prints a payload to pay once), then `aso install --claim-psp` once it has its confirmations; or `aso install --handoff FILE --did-key-file KEYFILE`".to_string());
            return Ok(lines);
        }
        Funding::Buy { mint_key } => vec![
            "buy-psp".to_string(),
            "--mint-key".to_string(),
            mint_key.clone(),
        ],
        Funding::Claim => vec!["claim-psp".to_string()],
        Funding::Handoff {
            input,
            did_key_file,
        } => {
            let mut c = vec!["handoff".to_string(), "import".to_string(), input.clone()];
            if let Some(f) = did_key_file {
                c.push("--did-key-file".to_string());
                c.push(f.clone());
            }
            c
        }
    };
    let ran = wallet::run_ok(aso, ctx.exec, &command)?;
    for l in ran.stdout.lines() {
        lines.push(format!("  {l}"));
    }
    if matches!(funding, Funding::Buy { .. }) {
        lines.push("Pay that payload once (in an OP_RETURN), wait for its confirmations, then run `aso install --claim-psp`".to_string());
        return Ok(lines);
    }
    let after = wallet::balance(aso, ctx.exec)?;
    lines.push(if after.psp > 0 {
        format!("the wallet is funded (PSP {})", after.psp)
    } else {
        "the wallet still holds no PSP".to_string()
    });
    Ok(lines)
}

/// Step d: ESP exchanged for another service, and issued as FSP there.
pub fn do_backup(ctx: &Ctx, service: &str, psp: &str) -> Result<Vec<String>, String> {
    let aso = ctx.aso;
    let mut lines = Vec::new();
    let exchange = [
        "exchange".to_string(),
        "--service".to_string(),
        service.to_string(),
        "--psp".to_string(),
        psp.to_string(),
    ];
    let ran = wallet::run_ok(aso, ctx.exec, &exchange)?;
    lines.extend(ran.stdout.lines().map(|l| format!("  {l}")));
    let issue = [
        "fsp".to_string(),
        "issue".to_string(),
        "--service".to_string(),
        service.to_string(),
    ];
    let ran = wallet::run_ok(aso, ctx.exec, &issue)?;
    lines.extend(ran.stdout.lines().map(|l| format!("  {l}")));
    let mut st = state::read(aso)?;
    st.backup = "done".to_string();
    state::write(aso, &st)?;
    lines.push(format!("FSP at {service} funded from {psp} PSP"));
    Ok(lines)
}

/// Set `[redeem] payout` in aso.toml (a backup of the old file is kept).
pub fn set_payout(ctx: &Ctx, payout: &str) -> Result<String, String> {
    let text = fs::read_to_string(&ctx.config).unwrap_or_default();
    let edited = edit::set_value(&text, "redeem", "payout", payout);
    let backup = backup_and_replace(
        &ctx.config,
        &edited,
        ctx.now,
        ctx.aso.whole("service.config_backups") as usize,
        "aso.toml",
    )?;
    Ok(format!(
        "payout set to {payout} in {} (the old file is {})",
        ctx.config.display(),
        backup.display()
    ))
}

/// `aso install`: rescan, then do the steps that are not done. `ask` is used
/// for the payout (when unset) and nothing else; with no terminal it asks
/// nothing and the install goes on.
pub fn install(ctx: &Ctx, opts: &Options, ask: &mut dyn Ask) -> Outcome {
    let mut out = Outcome::new();
    for l in did_key_notice(ctx.aso) {
        out.say(l);
    }
    match prepare(ctx) {
        Ok(lines) => lines.into_iter().for_each(|l| out.say(l)),
        Err(e) => {
            out.fail(format!("preparing the accounts and folders: {e}"));
            return out;
        }
    }
    let scan = match status::collect(ctx.aso, ctx.exec) {
        Ok(s) => s,
        Err(e) => {
            out.fail(e);
            return out;
        }
    };
    for step in [Step::Air, Step::Fsp, Step::Wallet] {
        let funding_asked = step == Step::Wallet && opts.funding != Funding::None;
        if scan.done(step) && !funding_asked {
            out.say(format!("{}: done already, left as it is", step.label()));
            continue;
        }
        for m in scan.missing(step) {
            out.say(format!("{}: missing: {m}", step.label()));
        }
        let done = match step {
            Step::Air => do_air(ctx),
            Step::Fsp => do_fsp(ctx),
            _ => do_wallet(ctx, &opts.funding),
        };
        match done {
            Ok(lines) => lines.into_iter().for_each(|l| out.say(format!("  {l}"))),
            Err(e) => {
                out.fail(format!("{}: {e}", step.label()));
                out.say(
                    "stopped: fix that and run `aso install` again; what is done is not done twice",
                );
                return out;
            }
        }
    }
    if ctx.aso.opt("redeem.payout").is_none() {
        let asked = ask.line(
            "Where should the ESP this service earns be paid out (for example btc:ADDRESS)? Enter to leave the automatic redeem off:",
        );
        if let Some(payout) = asked {
            match set_payout(ctx, &payout) {
                Ok(l) => {
                    out.say(l);
                    out.say("run `aso reload` to start the automatic redeem");
                }
                Err(e) => out.fail(e),
            }
        } else {
            out.say(
                "automatic redeem: off until redeem.payout is set in aso.toml (then `aso reload`)",
            );
        }
    }
    let st = state::read(ctx.aso).unwrap_or_default();
    if st.backup.is_empty() {
        if let Some((service, psp)) = &opts.backup {
            match do_backup(ctx, service, psp) {
                Ok(lines) => lines.into_iter().for_each(|l| out.say(format!("  {l}"))),
                Err(e) => out.fail(format!("{}: {e}", Step::Backup.label())),
            }
        } else if opts.skip_backup {
            let mut st = st;
            st.backup = "skipped".to_string();
            match state::write(ctx.aso, &st) {
                Ok(()) => out.say("step d skipped (recommended, not required)"),
                Err(e) => out.fail(e),
            }
        } else {
            out.say("step d (recommended): `aso install --backup-service DID --backup-psp N` puts FSP at another service for backups; `aso install --skip-backup` skips it");
        }
    }
    match status::collect(ctx.aso, ctx.exec) {
        Ok(s) => status::render(&s).into_iter().for_each(|l| out.say(l)),
        Err(e) => out.fail(e),
    }
    out
}
