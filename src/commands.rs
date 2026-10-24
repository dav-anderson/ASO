//! The commands that act on the services: `start`, `stop`, `restart`, `reload`
//! and the `upgrade` family. Each returns an [`Outcome`] (lines to print and
//! whether it worked); nothing here prints.
//!
//! **Only `stop`, `restart`, `upgrade` (and its scheduled run) and `install` can
//! stop a service**; they say so, and they ask before they act. `reload` sends
//! the services' reload signal and rewrites files, and stops nothing.

use air_interface::release;
use air_interface::schedule::{self, DEFAULT_LATE_SECS, Due, Pending, Where};
use air_interface::upgrade::{self, Options};
use air_interface::{Managed, Probe};
use jiff::Zoned;
use jiff::tz::TimeZone;

use crate::ask::Ask;
use crate::ctl::{self, Action};
use crate::install::{self, Ctx};
use crate::interface::unpinned;
use crate::logs::{self, Log};
use crate::outcome::Outcome;
use crate::program::Program;
use crate::state;
use crate::status;

/// Said before any upgrade, and by `aso upgrade --help`.
pub const UPGRADE_WARNING: &str = "An upgrade of air_server or fsp-mint is a BRIEF DOWNTIME: the old program is stopped and the new one started (the time it was down is reported). Nothing is upgraded while it runs.";

fn zoned_now(ctx: &Ctx) -> Result<Zoned, String> {
    schedule::zoned(ctx.now, &TimeZone::system())
}

/// `aso start`: air_server, then fsp-mint, each until its health is ok.
pub fn start(ctx: &Ctx) -> Outcome {
    let mut out = Outcome::new();
    let missing = status::fsp_missing(ctx.aso);
    if !missing.is_empty() {
        out.fail(format!(
            "fsp-mint cannot start without {}: set them in aso.toml (or run `aso install`)",
            missing.join(", ")
        ));
        return out;
    }
    match ctl::start_all(ctx.aso, ctx.exec) {
        Ok(lines) => lines.into_iter().for_each(|l| out.say(l)),
        Err(e) => {
            out.fail(e);
            return out;
        }
    }
    for p in [Program::Air, Program::Fsp] {
        if let Err(e) = state::mark_applied(ctx.aso, p) {
            out.fail(e);
        }
    }
    out
}

/// `aso stop`: take the whole service down on purpose (decommission). fsp-mint
/// goes first and gets its full stop time, then air_server; neither starts at
/// boot again until `aso start`.
pub fn stop(ctx: &Ctx, ask: &mut dyn Ask, yes: bool) -> Outcome {
    let mut out = Outcome::new();
    out.say("`aso stop` takes the service DOWN: wallets cannot reach it until `aso start`. It is for decommissioning; to apply a change use `aso reload` or `aso restart`.");
    if !yes && !ask.confirm("Stop fsp-mint and air_server?", false) {
        out.say("not confirmed: nothing was stopped (use --yes to answer for yourself)");
        return out;
    }
    for p in [Program::Fsp, Program::Air] {
        match ctl::apply(ctx.aso, ctx.exec, p, Action::Stop) {
            Ok(()) => out.say(format!("{}: stopped, and not started at boot", p.name())),
            Err(e) => {
                out.fail(e);
                return out;
            }
        }
    }
    out
}

/// `aso restart [air_server|fsp-mint]`: stop and start, saying what it costs
/// first. Both services restart in the start order when none is named.
pub fn restart(ctx: &Ctx, which: Option<Program>, ask: &mut dyn Ask, yes: bool) -> Outcome {
    let mut out = Outcome::new();
    let programs = match which {
        Some(p) if p.is_service() => vec![p],
        Some(p) => {
            out.fail(format!(
                "{} is not a service: there is nothing to restart",
                p.name()
            ));
            return out;
        }
        None => vec![Program::Air, Program::Fsp],
    };
    for p in &programs {
        out.say(format!("{}: {}", p.name(), p.method()));
    }
    if !yes && !ask.confirm("Restart now?", false) {
        out.say("not confirmed: nothing was restarted (use --yes to answer for yourself)");
        return out;
    }
    let wait = ctx.aso.whole("service.start_wait_secs");
    for p in programs {
        let began = std::time::Instant::now();
        if let Err(e) = ctl::apply(ctx.aso, ctx.exec, p, Action::Restart) {
            out.fail(e);
            return out;
        }
        let h = ctl::wait_healthy(ctx.aso, p, wait);
        if !h.up {
            out.fail(format!(
                "{} was restarted but its health is not ok after {wait} s: {}",
                p.name(),
                h.detail
            ));
            return out;
        }
        out.say(format!(
            "{}: restarted, health ok (back after {} s)",
            p.name(),
            began.elapsed().as_secs()
        ));
        if let Err(e) = state::mark_applied(ctx.aso, p) {
            out.fail(e);
        }
    }
    out
}

/// `aso reload`: write the files again from `aso.toml`, make fsp-mint read its
/// settings (SIGHUP), put the redeem job right, and say which settings need a
/// restart and are waiting for one. It stops nothing.
pub fn reload(ctx: &Ctx) -> Outcome {
    let mut out = Outcome::new();
    let written = match install::refresh(ctx) {
        Ok(w) => w,
        Err(e) => {
            out.fail(e);
            return out;
        }
    };
    if written.is_empty() {
        out.say("the generated files already match aso.toml");
    }
    for p in &written {
        out.say(format!("wrote {}", p.display()));
    }
    match install::reconcile_redeem(ctx, &written) {
        Ok(lines) => lines.into_iter().for_each(|l| out.say(l)),
        Err(e) => out.fail(e),
    }
    match ctl::is_running(ctx.aso, ctx.exec, Program::Fsp) {
        Some(true) => match ctl::apply(ctx.aso, ctx.exec, Program::Fsp, Action::Reload) {
            Ok(()) => out.say("fsp-mint: told to read its settings again (nothing was stopped)"),
            Err(e) => out.fail(e),
        },
        _ => out.say(
            "fsp-mint is not running: nothing to reload; it reads its settings when it starts",
        ),
    }
    out.say("air_server has no reload: its air.* settings apply at its next restart");
    match state::read(ctx.aso) {
        Ok(st) => {
            for (key, was, now) in ctx.aso.restart_changes(&st.applied) {
                out.say(format!(
                    "WAITING FOR A RESTART: {key} was {was} when the service started, aso.toml says {now}: `aso restart` applies it"
                ));
            }
        }
        Err(e) => out.fail(e),
    }
    out
}

/// What `aso upgrade` does with pins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinPolicy {
    /// Ask about each pin; without a terminal keep it, with a warning.
    Ask,
    /// `--keep-pins`.
    Keep,
    /// `--reset-pins`.
    Reset,
}

/// When `aso upgrade` runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum When {
    Now,
    /// `--at HH:MM`.
    At(String),
    /// `--in 90m`, `--in 2h30m`.
    In(String),
}

/// `aso upgrade [PROGRAM...]` and its options.
#[derive(Clone, Debug)]
pub struct UpgradeArgs {
    /// Empty: all three.
    pub programs: Vec<Program>,
    pub when: When,
    pub pins: PinPolicy,
    pub yes: bool,
    pub dry_run: bool,
}

fn names(programs: &[Program]) -> Vec<String> {
    programs.iter().map(|p| p.name().to_string()).collect()
}

/// Run the upgrade of `programs` with `aso` (the pins already removed or kept).
fn run_upgrade(
    ctx: &Ctx,
    aso: &crate::config::Aso,
    programs: &[Program],
    dry_run: bool,
) -> upgrade::Upgraded<Program> {
    let opts = Options {
        dry_run,
        wait_secs: ctx.aso.whole("service.start_wait_secs"),
        source: ctx.source,
    };
    upgrade::upgrade(aso, programs, &opts, ctx.exec, &|p| Probe {
        up: ctl::health(ctx.aso, p).up,
        version: None,
    })
}

/// `aso upgrade`: warn, ask about pins, confirm, then run now or schedule.
pub fn upgrade(ctx: &Ctx, args: &UpgradeArgs, ask: &mut dyn Ask) -> Outcome {
    let mut out = Outcome::new();
    let aso = ctx.aso;
    let programs = if args.programs.is_empty() {
        Program::ALL.to_vec()
    } else {
        args.programs.clone()
    };
    let now = match zoned_now(ctx) {
        Ok(n) => n,
        Err(e) => {
            out.fail(e);
            return out;
        }
    };
    let scheduled = args.when != When::Now;
    if scheduled
        && !args.dry_run
        && let Err(e) = schedule::refuse_if_pending(aso, &now)
    {
        out.fail(e);
        return out;
    }
    let due: Result<Option<Zoned>, String> = match &args.when {
        When::Now => Ok(None),
        When::At(text) => schedule::parse_at(text)
            .and_then(|(h, m)| schedule::next_at(&now, h, m))
            .map(Some),
        When::In(text) => schedule::parse_duration(text)
            .and_then(|s| schedule::after(&now, s))
            .map(Some),
    };
    let due = match due {
        Ok(d) => d,
        Err(e) => {
            out.fail(e);
            return out;
        }
    };
    out.say(UPGRADE_WARNING);
    for p in &programs {
        out.say(format!("{}: {}", p.name(), p.method()));
    }
    let held = release::pinned(aso, ctx.source, &programs);
    release::pin_warning(&held)
        .into_iter()
        .for_each(|l| out.say(l));
    let mut reset = Vec::new();
    for h in &held {
        let reset_it = match args.pins {
            PinPolicy::Keep => false,
            PinPolicy::Reset => true,
            PinPolicy::Ask if ask.interactive() => ask.confirm(
                &format!(
                    "Reset the pin of {} (pinned at {}, newest {}) so the newest is installed?",
                    h.program.name(),
                    h.pin,
                    h.newest.as_deref().unwrap_or("unknown")
                ),
                false,
            ),
            PinPolicy::Ask => {
                out.say(format!(
                    "WARNING: {} stays pinned at {}: there is no terminal to ask (--reset-pins resets it)",
                    h.program.name(),
                    h.pin
                ));
                false
            }
        };
        if reset_it {
            reset.push(h.program);
        }
    }
    let effective = unpinned(aso, &reset);
    if args.dry_run {
        let plan = run_upgrade(ctx, &effective, &programs, true);
        plan.lines.into_iter().for_each(|l| out.say(l));
        out.say("dry run: nothing was changed");
        out.ok = plan.ok;
        return out;
    }
    let question = match &due {
        Some(d) => format!("Schedule this upgrade for {}?", schedule::describe(d, &now)),
        None => "Upgrade now?".to_string(),
    };
    if !args.yes && !ask.confirm(&question, false) {
        out.say("not confirmed: nothing was changed (use --yes to answer for yourself)");
        return out;
    }
    if let Some(due) = due {
        let pending = Pending {
            due: due.timestamp().as_second(),
            created: ctx.now,
            programs: if args.programs.is_empty() {
                Vec::new()
            } else {
                names(&programs)
            },
            late_secs: DEFAULT_LATE_SECS,
            reset: names(&reset),
        };
        let w = Where::system(aso, ctx.exe.clone(), ctx.config.clone());
        match schedule::schedule(aso, &w, ctx.exec, &pending, &due, &now) {
            Ok(line) => out.say(line),
            Err(e) => out.fail(e),
        }
        return out;
    }
    if !reset.is_empty() {
        match release::reset_pins(
            &ctx.config,
            &reset,
            ctx.now,
            aso.whole("service.config_backups") as usize,
        ) {
            Ok(lines) => lines.into_iter().for_each(|l| out.say(l)),
            Err(e) => {
                out.fail(e);
                return out;
            }
        }
    }
    let done = run_upgrade(ctx, &effective, &programs, false);
    done.lines.into_iter().for_each(|l| out.say(l));
    if !done.ok {
        out.ok = false;
    }
    out
}

/// `aso upgrade check`.
pub fn upgrade_check(ctx: &Ctx) -> Outcome {
    let mut out = Outcome::new();
    let lines = release::check(ctx.aso, ctx.exec, ctx.source);
    release::render(&lines, "aso.toml", 13)
        .lines()
        .for_each(|l| out.say(l));
    let held = release::pinned(ctx.aso, ctx.source, &Program::ALL);
    release::pin_warning(&held)
        .into_iter()
        .for_each(|l| out.say(l));
    out
}

/// `aso upgrade status`.
pub fn upgrade_status(ctx: &Ctx) -> Outcome {
    let mut out = Outcome::new();
    let w = Where::system(ctx.aso, ctx.exe.clone(), ctx.config.clone());
    match zoned_now(ctx).and_then(|now| schedule::status(ctx.aso, &w, &now)) {
        Ok(lines) => lines.into_iter().for_each(|l| out.say(l)),
        Err(e) => out.fail(e),
    }
    let held = release::pinned(ctx.aso, ctx.source, &Program::ALL);
    release::pin_warning(&held)
        .into_iter()
        .for_each(|l| out.say(l));
    out
}

/// `aso upgrade cancel`.
pub fn upgrade_cancel(ctx: &Ctx) -> Outcome {
    let mut out = Outcome::new();
    let w = Where::system(ctx.aso, ctx.exe.clone(), ctx.config.clone());
    match zoned_now(ctx).and_then(|now| schedule::cancel(ctx.aso, &w, ctx.exec, &now)) {
        Ok(line) => out.say(line),
        Err(e) => out.fail(e),
    }
    out
}

/// What the scheduled job runs (`aso upgrade run-scheduled`): ask the clock,
/// then upgrade, skip (too late) or do nothing (not yet). It asks nothing, and
/// writes `aso-upgrade.log` (capped like every log aso writes).
pub fn run_scheduled(ctx: &Ctx) -> Outcome {
    let mut out = Outcome::new();
    let aso = ctx.aso;
    let epoch = u64::try_from(ctx.now).unwrap_or(0);
    let mut log = match Log::open(
        "aso",
        &schedule::log_file(aso),
        aso.log_max_mb(),
        aso.log_keep(),
        epoch,
        false,
    ) {
        Ok(l) => l,
        Err(e) => {
            out.fail(e);
            return out;
        }
    };
    let mut say = |out: &mut Outcome, line: String| {
        log.line(logs::now_secs(), &line);
        out.say(line);
    };
    let w = Where::system(aso, ctx.exe.clone(), ctx.config.clone());
    let state = match schedule::due_state(aso, ctx.now) {
        Ok(s) => s,
        Err(e) => {
            say(&mut out, format!("FAIL: {e}"));
            out.ok = false;
            return out;
        }
    };
    match state {
        Due::Nothing => say(&mut out, "no upgrade is scheduled: nothing to do".into()),
        Due::NotYet(p) => say(
            &mut out,
            format!("the upgrade is due at {}, not yet: left scheduled", p.due),
        ),
        Due::TooLate(p, late) => {
            say(
                &mut out,
                format!(
                    "SKIPPED: the scheduled upgrade was due at {} and it is {} s late (more than the {} s allowed): nothing was changed",
                    p.due, late, p.late_secs
                ),
            );
            if let Err(e) = schedule::remove(aso, &w, ctx.exec) {
                say(&mut out, format!("FAIL: removing the job: {e}"));
                out.ok = false;
            }
        }
        Due::Go(p) => {
            let programs: Vec<Program> = if p.programs.is_empty() {
                Program::ALL.to_vec()
            } else {
                p.programs
                    .iter()
                    .filter_map(|n| Program::parse(n))
                    .collect()
            };
            let reset: Vec<Program> = p.reset.iter().filter_map(|n| Program::parse(n)).collect();
            say(&mut out, "running the scheduled upgrade".into());
            if !reset.is_empty() {
                match release::reset_pins(
                    &ctx.config,
                    &reset,
                    ctx.now,
                    aso.whole("service.config_backups") as usize,
                ) {
                    Ok(lines) => lines.into_iter().for_each(|l| say(&mut out, l)),
                    Err(e) => {
                        say(&mut out, format!("FAIL: {e}"));
                        out.ok = false;
                    }
                }
            }
            let effective = unpinned(aso, &reset);
            let done = run_upgrade(ctx, &effective, &programs, false);
            done.lines.into_iter().for_each(|l| say(&mut out, l));
            if !done.ok {
                out.ok = false;
            }
            if let Err(e) = schedule::remove(aso, &w, ctx.exec) {
                say(&mut out, format!("FAIL: removing the job: {e}"));
                out.ok = false;
            }
        }
    }
    out
}
