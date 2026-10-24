//! The command line, as a function: `run(args, env, ...)` returns the exit code
//! and the text for each stream, so every command can be tested without a
//! process. `main.rs` only builds the real [`Env`] and prints.
//!
//! Exit codes: 0 worked; 1 the command ran and failed, or `status` found the
//! service not ready, or `audit` found the mint inconsistent; 2 the command
//! line or `aso.toml` is wrong (nothing was changed).

use std::path::{Path, PathBuf};

use crate::ask::Ask;
use crate::audit;
use crate::commands::{self, PinPolicy, UpgradeArgs, When};
use crate::config::{Aso, Inputs};
use air_minterface::Managed;
use air_minterface::files::replace_file;
use air_minterface::github::Github;
use air_minterface::release::Source;

use crate::exec::Exec;
use crate::install::{self, Ctx, Funding, Options};
use crate::logs::{self, Log};
use crate::outcome::Outcome;
use crate::program::Program;
use crate::redeem;
use crate::status;
use crate::versions;

pub const HELP: &str = "\
aso [--dir DIR] [--config FILE] COMMAND

  Runs one Service Point's own services: air_server (behind nginx), fsp-mint and
  ecash-wallet. All settings are in one file, aso.toml. DIR is the aso folder
  (default ~/.aso, or $ASO_DIR); FILE is aso.toml (default DIR/aso.toml, or
  $ASO_CONFIG).

COMMANDS
  install    The guided install, in four steps: (a) air_server behind nginx,
             (b) fsp-mint serve, (c) fund the wallet, (d) optional: FSP at another
             service for backups. Run it again at any time: it checks what exists,
             says what is missing and builds only that.
               --buy-psp MINT_KEY AMOUNT...   print the payload to pay (once)
               --claim-psp                    pick up what was paid for
               --handoff FILE|STRING [--did-key-file FILE]
               --backup-service DID --backup-psp N     step d
               --skip-backup                  skip step d
  start      Start air_server, then fsp-mint, each until its health is ok.
  stop       Take the whole service down (decommission only). Asks first; --yes.
  restart    [air_server|fsp-mint]  Stop and start. Asks first; --yes.
  reload     Write the generated files again from aso.toml and make fsp-mint read
             its settings. Stops nothing. Names the settings waiting for a restart.
  status     The install's steps, each process (running, health, version) and
             whether the service is ready (the wallet is funded). Exit 1 if not.
  audit      fsp-mint audit and the wallet's balance. Stops nothing. Exit 1 if the
             mint audit fails.
  versions   The version of each program, or `unknown` and why.
  upgrade    [air_server|fsp-mint|ecash-wallet ...] [--at HH:MM | --in DURATION]
             [--keep-pins | --reset-pins] [--dry-run] [--yes]
             A BRIEF DOWNTIME for air_server and fsp-mint. Warns and asks first.
  upgrade check    Which program has a newer release.
  upgrade status   The scheduled upgrade, if any.
  upgrade cancel   Cancel the scheduled upgrade.
  config     Every setting, its value and where the value came from.
  redeem run The automatic ESP redeem, once (the timer runs this).
  logs rotate [--running] FILE...
             Put a log over log_max_size_mb aside, keeping log_keep. The units
             run it before a start; the logs job runs it with --running (copy
             aside and empty in place) every 5 minutes while the services run.

OPTIONS
  -h, --help       This text.   -V, --version   aso's version.
";

/// What `run` needs from the machine.
pub struct Env<'a> {
    /// `std::env::consts::OS`.
    pub os: &'a str,
    pub home: String,
    pub vars: &'a dyn Fn(&str) -> Option<String>,
    /// Now, in seconds since the Unix epoch.
    pub now: i64,
    /// The `aso` binary (the units run it).
    pub exe: PathBuf,
    /// Where releases come from; `None`: GitHub through curl.
    pub source: Option<&'a dyn Source<Program>>,
}

/// The result of a command line.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Done {
    pub code: i32,
    pub out: String,
    pub err: String,
}

fn done(out: Outcome) -> Done {
    Done {
        code: i32::from(!out.ok),
        out: out.lines.join("\n") + "\n",
        err: String::new(),
    }
}

fn usage(why: impl Into<String>) -> Done {
    Done {
        code: 2,
        out: String::new(),
        err: format!("aso: {}\nRun `aso --help` for the commands.\n", why.into()),
    }
}

/// A starter `aso.toml`: every line a comment, so it changes nothing until
/// the operator edits it.
pub const STARTER: &str = "\
# aso.toml: the one settings file of this Service Point. `aso config` prints every
# setting and where its value comes from; the README lists them all. Settings aso
# does not name under [fsp.*] and [wallet.*] go to fsp-mint and ecash-wallet as
# written.

# The ESP mint (the SPI's) this service converts through. fsp-mint serve refuses to
# start without these three.
# [fsp.air]
# esp_did = \"\"
# esp_server = \"\"
# esp_host = \"\"

# The Bitcoin network. main, regtest, signet or testnet.
# [fsp.server]
# network = \"main\"

# The public name nginx answers on (and the host in this service's air.fsp entry),
# its certificate and key.
# [air]
# public_name = \"\"
# public_cert = \"/etc/ssl/aso/fullchain.pem\"
# public_key = \"/etc/ssl/aso/privkey.pem\"
# bin = \"/path/to/air_server\"
# version = \"1.0.1\"

# Where the ESP this service earns is paid out. `aso install` asks for it.
# [redeem]
# payout = \"btc:ADDRESS\"
";

struct Place {
    dir: PathBuf,
    config: PathBuf,
}

fn flag_value(args: &mut Vec<String>, flag: &str) -> Result<Option<String>, String> {
    let Some(at) = args.iter().position(|a| a == flag) else {
        return Ok(None);
    };
    if at + 1 >= args.len() {
        return Err(format!("{flag} needs a value"));
    }
    let value = args.remove(at + 1);
    args.remove(at);
    Ok(Some(value))
}

fn flag(args: &mut Vec<String>, name: &str) -> bool {
    match args.iter().position(|a| a == name) {
        Some(at) => {
            args.remove(at);
            true
        }
        None => false,
    }
}

fn no_more(args: &[String], command: &str) -> Result<(), String> {
    match args.first() {
        None => Ok(()),
        Some(a) => Err(format!("`aso {command}` does not take `{a}`")),
    }
}

/// `aso logs rotate [--running] FILE...`: needs no `aso.toml`; the caps come
/// from the environment the unit gives it. With `--running` the files are held
/// open by a running program: they are copied aside and emptied in place.
fn logs_rotate(args: &[String], env: &Env) -> Done {
    let mut args = args.to_vec();
    let running = flag(&mut args, "--running");
    let number = |name: &str, default: u64| -> Result<u64, String> {
        match (env.vars)(name) {
            None => Ok(default),
            Some(v) => v
                .trim()
                .parse::<u64>()
                .map_err(|_| format!("{name} is `{v}`: it must be a whole number")),
        }
    };
    let (max, keep) = match (
        number("ASO_SERVICE_LOG_MAX_SIZE_MB", logs::DEFAULT_MAX_MB),
        number("ASO_SERVICE_LOG_KEEP", logs::DEFAULT_KEEP),
    ) {
        (Ok(m), Ok(k)) => (m, k),
        (Err(e), _) | (_, Err(e)) => return usage(e),
    };
    if args.is_empty() {
        return usage("`aso logs rotate` needs at least one log file");
    }
    let mut out = Outcome::new();
    let now = u64::try_from(env.now).unwrap_or(0);
    for file in &args {
        let rotated = if running {
            logs::rotate_running(Path::new(file), max, keep, now)
        } else {
            logs::rotate(Path::new(file), max, keep, now)
        };
        match rotated {
            Ok(Some(moved)) if running => out.say(format!(
                "{file} is over {max} MB: copied to {} and emptied",
                moved.display()
            )),
            Ok(Some(moved)) => out.say(format!(
                "{file} is over {max} MB: moved to {}",
                moved.display()
            )),
            Ok(None) => {}
            Err(e) => out.fail(e),
        }
    }
    let mut d = done(out);
    if d.out == "\n" {
        d.out.clear();
    }
    d
}

fn parse_funding(args: &mut Vec<String>) -> Result<Funding, String> {
    let handoff = flag_value(args, "--handoff")?;
    let did_key = flag_value(args, "--did-key-file")?;
    let claim = flag(args, "--claim-psp");
    let buy = args.iter().position(|a| a == "--buy-psp");
    let kinds = usize::from(handoff.is_some()) + usize::from(claim) + usize::from(buy.is_some());
    if kinds > 1 {
        return Err("give one of --buy-psp, --claim-psp, --handoff".into());
    }
    if did_key.is_some() && handoff.is_none() {
        return Err("--did-key-file goes with --handoff".into());
    }
    if let Some(at) = buy {
        let mut rest: Vec<String> = args.drain(at..).collect();
        rest.remove(0);
        if rest.len() < 2 {
            return Err("--buy-psp needs MINT_KEY and at least one AMOUNT".into());
        }
        // Flags that follow the amounts belong to install.
        let split = rest
            .iter()
            .position(|a| a.starts_with("--"))
            .unwrap_or(rest.len());
        let tail = rest.split_off(split);
        args.extend(tail);
        let mint_key = rest.remove(0);
        if rest.is_empty() {
            return Err("--buy-psp needs at least one AMOUNT after MINT_KEY".into());
        }
        return Ok(Funding::Buy {
            mint_key,
            amounts: rest,
        });
    }
    if claim {
        return Ok(Funding::Claim);
    }
    Ok(match handoff {
        Some(input) => Funding::Handoff {
            input,
            did_key_file: did_key,
        },
        None => Funding::None,
    })
}

fn parse_install(mut args: Vec<String>) -> Result<Options, String> {
    let funding = parse_funding(&mut args)?;
    let service = flag_value(&mut args, "--backup-service")?;
    let psp = flag_value(&mut args, "--backup-psp")?;
    let skip_backup = flag(&mut args, "--skip-backup");
    no_more(&args, "install")?;
    let backup = match (service, psp) {
        (Some(s), Some(p)) => Some((s, p)),
        (None, None) => None,
        _ => return Err("--backup-service and --backup-psp go together".into()),
    };
    if backup.is_some() && skip_backup {
        return Err("--skip-backup and --backup-service cannot be used together".into());
    }
    Ok(Options {
        funding,
        backup,
        skip_backup,
    })
}

fn parse_upgrade(mut args: Vec<String>) -> Result<UpgradeArgs, String> {
    let at = flag_value(&mut args, "--at")?;
    let within = flag_value(&mut args, "--in")?;
    let keep = flag(&mut args, "--keep-pins");
    let reset = flag(&mut args, "--reset-pins");
    let dry_run = flag(&mut args, "--dry-run");
    let yes = flag(&mut args, "--yes");
    let when = match (at, within) {
        (Some(_), Some(_)) => return Err("--at and --in cannot be used together".into()),
        (Some(t), None) => When::At(t),
        (None, Some(t)) => When::In(t),
        (None, None) => When::Now,
    };
    if keep && reset {
        return Err("--keep-pins and --reset-pins cannot be used together".into());
    }
    let mut programs = Vec::new();
    for a in &args {
        match Program::parse(a) {
            Some(p) => programs.push(p),
            None => {
                return Err(format!(
                    "`{a}` is not a program: air_server, fsp-mint or ecash-wallet"
                ));
            }
        }
    }
    Ok(UpgradeArgs {
        programs,
        when,
        pins: if keep {
            PinPolicy::Keep
        } else if reset {
            PinPolicy::Reset
        } else {
            PinPolicy::Ask
        },
        yes,
        dry_run,
    })
}

/// Run a command line (`args` without the program name).
pub fn run(args: &[String], env: &Env, exec: &dyn Exec, ask: &mut dyn Ask) -> Done {
    let mut args: Vec<String> = args.to_vec();
    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        return Done {
            code: 0,
            out: HELP.to_string(),
            err: String::new(),
        };
    }
    if args.iter().any(|a| a == "--version" || a == "-V") {
        return Done {
            code: 0,
            out: format!("aso {}\n", env!("CARGO_PKG_VERSION")),
            err: String::new(),
        };
    }
    let (dir_flag, config_flag) = match (
        flag_value(&mut args, "--dir"),
        flag_value(&mut args, "--config"),
    ) {
        (Ok(d), Ok(c)) => (d, c),
        (Err(e), _) | (_, Err(e)) => return usage(e),
    };
    let Some(command) = args.first().cloned() else {
        return usage("no command given");
    };
    args.remove(0);
    if command == "logs" {
        return match args.first().map(String::as_str) {
            Some("rotate") => logs_rotate(&args[1..], env),
            _ => usage("`aso logs` takes `rotate [--running] FILE...`"),
        };
    }
    let dir = dir_flag
        .or_else(|| (env.vars)("ASO_DIR"))
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(&env.home).join(".aso"));
    let config = config_flag
        .or_else(|| (env.vars)("ASO_CONFIG"))
        .map(PathBuf::from)
        .unwrap_or_else(|| dir.join("aso.toml"));
    let place = Place { dir, config };
    let mut started = Vec::new();
    if command == "install" && !place.config.is_file() {
        match replace_file(&place.config, STARTER.as_bytes()) {
            Ok(()) => started.push(format!(
                "wrote a starter {} (every line a comment): edit it, then run `aso install` again to use what you set",
                place.config.display()
            )),
            Err(e) => return usage(e),
        }
    }
    let text = std::fs::read_to_string(&place.config).ok();
    let aso = match Aso::load(&Inputs {
        dir: place.dir.clone(),
        file: text.as_deref(),
        env: env.vars,
        home: env.home.clone(),
        os: env.os,
    }) {
        Ok(a) => a,
        Err(problems) => {
            return Done {
                code: 2,
                out: String::new(),
                err: format!(
                    "{} has problems; nothing was changed:\n  {}\n",
                    place.config.display(),
                    problems.join("\n  ")
                ),
            };
        }
    };
    let curl = Github::new(&aso, exec);
    let source: &dyn Source<Program> = env.source.unwrap_or(&curl);
    let ctx = Ctx {
        aso: &aso,
        exec,
        source,
        exe: env.exe.clone(),
        config: place.config.clone(),
        now: env.now,
    };
    let sub = args.first().cloned();
    match command.as_str() {
        "install" => match parse_install(args) {
            Ok(opts) => {
                let mut out = install::install(&ctx, &opts, ask);
                for (i, l) in started.into_iter().enumerate() {
                    out.lines.insert(i, l);
                }
                done(out)
            }
            Err(e) => usage(e),
        },
        "start" => match no_more(&args, "start") {
            Ok(()) => done(commands::start(&ctx)),
            Err(e) => usage(e),
        },
        "stop" => {
            let yes = flag(&mut args, "--yes");
            match no_more(&args, "stop") {
                Ok(()) => done(commands::stop(&ctx, ask, yes)),
                Err(e) => usage(e),
            }
        }
        "restart" => {
            let yes = flag(&mut args, "--yes");
            let which = match args.as_slice() {
                [] => Ok(None),
                [one] => Program::parse(one)
                    .map(Some)
                    .ok_or_else(|| format!("`{one}` is not a program: air_server or fsp-mint")),
                _ => Err("`aso restart` takes one program at most".to_string()),
            };
            match which {
                Ok(w) => done(commands::restart(&ctx, w, ask, yes)),
                Err(e) => usage(e),
            }
        }
        "reload" => match no_more(&args, "reload") {
            Ok(()) => done(commands::reload(&ctx)),
            Err(e) => usage(e),
        },
        "status" => match no_more(&args, "status") {
            Ok(()) => match status::collect(&aso, exec) {
                Ok(s) => {
                    let mut o = Outcome::new();
                    status::render(&s).into_iter().for_each(|l| o.say(l));
                    o.ok = s.ready;
                    done(o)
                }
                Err(e) => {
                    let mut o = Outcome::new();
                    o.fail(e);
                    done(o)
                }
            },
            Err(e) => usage(e),
        },
        "audit" => match no_more(&args, "audit") {
            Ok(()) => {
                let a = audit::run(&aso, exec);
                let mut o = Outcome::new();
                a.lines.into_iter().for_each(|l| o.say(l));
                o.ok = a.ok;
                done(o)
            }
            Err(e) => usage(e),
        },
        "versions" => match no_more(&args, "versions") {
            Ok(()) => {
                let mut o = Outcome::new();
                versions::render(&versions::versions(&aso, exec))
                    .lines()
                    .for_each(|l| o.say(l));
                done(o)
            }
            Err(e) => usage(e),
        },
        "config" => match no_more(&args, "config") {
            Ok(()) => {
                let mut o = Outcome::new();
                aso.describe().into_iter().for_each(|l| o.say(l));
                done(o)
            }
            Err(e) => usage(e),
        },
        "upgrade" => match sub.as_deref() {
            Some("check") => done(commands::upgrade_check(&ctx)),
            Some("status") => done(commands::upgrade_status(&ctx)),
            Some("cancel") => done(commands::upgrade_cancel(&ctx)),
            Some("run-scheduled") => done(commands::run_scheduled(&ctx)),
            _ => match parse_upgrade(args) {
                Ok(a) => done(commands::upgrade(&ctx, &a, ask)),
                Err(e) => usage(e),
            },
        },
        "redeem" => match args.as_slice() {
            [run] if run == "run" => redeem_run(&ctx),
            _ => usage("`aso redeem` takes `run`"),
        },
        other => usage(format!("unknown command `{other}`")),
    }
}

/// `aso redeem run`: the chain once, logged to `aso-redeem.log`.
fn redeem_run(ctx: &Ctx) -> Done {
    let aso = ctx.aso;
    let epoch = u64::try_from(ctx.now).unwrap_or(0);
    let mut log = match Log::open(
        "aso",
        &aso.log_dir().join("aso-redeem.log"),
        aso.log_max_mb(),
        aso.log_keep(),
        epoch,
        false,
    ) {
        Ok(l) => l,
        Err(e) => {
            let mut o = Outcome::new();
            o.fail(e);
            return done(o);
        }
    };
    let ran = redeem::run(aso, ctx.exec, &mut |line| log.line(logs::now_secs(), line));
    let mut o = Outcome::new();
    ran.lines.into_iter().for_each(|l| o.say(l));
    o.ok = ran.ok;
    done(o)
}
