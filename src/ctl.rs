//! Talking to the service manager (systemctl on Linux, launchctl on macOS) and
//! to the services' health listeners.
//!
//! **Only four commands may stop or restart a service**: `aso stop` (take the
//! whole service down on purpose), `aso restart`, `aso upgrade` (and its
//! scheduled run) and `aso install` when it has to start what is not running.
//! Everything else (status, audit, versions, config, reload, redeem, the logs
//! command) uses [`Action::Reload`], [`is_running`] and [`health`] only, which
//! cannot stop a program. The tests check the command lists.

use std::time::{Duration, Instant};

use air_minterface::{Managed, air};

use crate::config::Aso;
use crate::exec::{Exec, command_line};
use crate::httpc;
use crate::program::Program;
use crate::service;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Load the definition if need be, enable it at boot and start it.
    Start,
    /// Stop it and do not start it at boot (decommission).
    Stop,
    /// Stop it and start it again.
    Restart,
    /// Make it read its settings again (SIGHUP). Stops nothing.
    Reload,
}

pub type Cmd = (String, Vec<String>);

fn cmd(bin: &str, args: &[&str]) -> Cmd {
    (
        bin.to_string(),
        args.iter().map(|a| a.to_string()).collect(),
    )
}

/// The commands that make the service manager read the definitions again.
pub fn define_commands(aso: &Aso) -> Vec<Cmd> {
    if aso.mac {
        Vec::new()
    } else {
        vec![cmd("systemctl", &["daemon-reload"])]
    }
}

/// The commands for `action` on a service program, in order. On macOS `Start`
/// begins with `launchctl bootstrap`, which fails when the job is already loaded;
/// [`apply`] ignores that one failure.
pub fn commands(aso: &Aso, program: Program, action: Action) -> Vec<Cmd> {
    if aso.mac {
        let label = service::label(program);
        let target = format!("system/{label}");
        let plist = service::plist_file(aso, program).display().to_string();
        match action {
            Action::Start => vec![
                cmd("launchctl", &["bootstrap", "system", &plist]),
                cmd("launchctl", &["kickstart", &target]),
            ],
            Action::Stop => vec![cmd("launchctl", &["bootout", &target])],
            Action::Restart => vec![cmd("launchctl", &["kickstart", "-k", &target])],
            Action::Reload => vec![cmd("launchctl", &["kill", "HUP", &target])],
        }
    } else {
        let unit = format!("{}.service", program.unit());
        match action {
            Action::Start => vec![cmd("systemctl", &["enable", "--now", &unit])],
            Action::Stop => vec![cmd("systemctl", &["disable", "--now", &unit])],
            Action::Restart => vec![cmd("systemctl", &["restart", &unit])],
            Action::Reload => vec![cmd("systemctl", &["reload", &unit])],
        }
    }
}

/// Run `action` on a service program.
pub fn apply(aso: &Aso, exec: &dyn Exec, program: Program, action: Action) -> Result<(), String> {
    for (i, (bin, args)) in commands(aso, program, action).into_iter().enumerate() {
        let ran = exec.run(&bin, &args, &[])?;
        // launchctl bootstrap of a job that is already loaded.
        let already_loaded = aso.mac && action == Action::Start && i == 0;
        if !ran.ok() && !already_loaded {
            return Err(format!(
                "`{}` failed: {}",
                command_line(&bin, &args),
                ran.last_line()
            ));
        }
    }
    Ok(())
}

/// Run the commands that make the service manager read the definitions again.
pub fn define(aso: &Aso, exec: &dyn Exec) -> Result<(), String> {
    for (bin, args) in define_commands(aso) {
        let ran = exec.run(&bin, &args, &[])?;
        if !ran.ok() {
            return Err(format!(
                "`{}` failed: {}",
                command_line(&bin, &args),
                ran.last_line()
            ));
        }
    }
    Ok(())
}

/// Whether the service manager has the program running; `None` when it cannot
/// say (a macOS job is looked at by root only).
pub fn is_running(aso: &Aso, exec: &dyn Exec, program: Program) -> Option<bool> {
    if aso.mac {
        let target = format!("system/{}", service::label(program));
        let ran = exec
            .run("launchctl", &["print".to_string(), target], &[])
            .ok()?;
        if !ran.ok() {
            return None;
        }
        Some(ran.stdout.contains("state = running"))
    } else {
        let unit = format!("{}.service", program.unit());
        let ran = exec
            .run(
                "systemctl",
                &["is-active".to_string(), "--quiet".to_string(), unit],
                &[],
            )
            .ok()?;
        Some(ran.ok())
    }
}

/// What a program's health listener answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Health {
    pub up: bool,
    pub detail: String,
}

/// fsp-mint's `GET /health` on `server.bind`, or air_server's health listener.
pub fn health(aso: &Aso, program: Program) -> Health {
    let (addr, path) = match program {
        Program::Air => (aso.text("air.health_addr"), "/"),
        Program::Fsp => (aso.fsp_bind(), "/health"),
        Program::Wallet => {
            return Health {
                up: false,
                detail: "ecash-wallet is not a service".into(),
            };
        }
    };
    match httpc::get(&addr, path) {
        Err(e) => Health {
            up: false,
            detail: e,
        },
        Ok((status, body)) => {
            let ok = match program {
                Program::Air => air::healthy(status, &body),
                _ => httpc::fsp_healthy(status, &body),
            };
            Health {
                up: ok,
                detail: if ok {
                    "ok".into()
                } else {
                    format!("answered {status}: {}", body.trim())
                },
            }
        }
    }
}

/// Poll the health listener until it says ok, or `secs` run out.
pub fn wait_healthy(aso: &Aso, program: Program, secs: u64) -> Health {
    let end = Instant::now() + Duration::from_secs(secs);
    loop {
        let h = health(aso, program);
        if h.up || Instant::now() >= end {
            return h;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// Start air_server, wait for its health, then start fsp-mint and wait for its
/// health: the start order. Programs already running are started again by the
/// manager as a no-op. Returns the lines to print.
pub fn start_all(aso: &Aso, exec: &dyn Exec) -> Result<Vec<String>, String> {
    let wait = aso.whole("service.start_wait_secs");
    let mut lines = Vec::new();
    for program in [Program::Air, Program::Fsp] {
        apply(aso, exec, program, Action::Start)?;
        let h = wait_healthy(aso, program, wait);
        if !h.up {
            return Err(format!(
                "{} was started but its health is not ok after {wait} s: {}. Its log is {}",
                program.name(),
                h.detail,
                aso.log_dir().join(program.log_name()).display()
            ));
        }
        lines.push(format!("{}: started, health ok", program.name()));
    }
    Ok(lines)
}

/// What to do with one of aso's timed jobs: the automatic redeem
/// ([`service::REDEEM_UNIT`]) or the logs job ([`service::LOGS_UNIT`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Job {
    /// Load it and let it run on its schedule.
    Enable,
    /// Unload it.
    Disable,
    /// Load it again (its definition changed).
    Reload,
}

/// The commands for the timed job `unit`. These touch the timer only: no
/// service is stopped or restarted.
pub fn job_commands(aso: &Aso, unit: &str, job: Job) -> Vec<Cmd> {
    if aso.mac {
        let label = format!("org.servicepoints.{unit}");
        let target = format!("system/{label}");
        let plist = aso
            .unit_dir()
            .join(format!("{label}.plist"))
            .display()
            .to_string();
        match job {
            Job::Enable => vec![cmd("launchctl", &["bootstrap", "system", &plist])],
            Job::Disable => vec![cmd("launchctl", &["bootout", &target])],
            Job::Reload => vec![
                cmd("launchctl", &["bootout", &target]),
                cmd("launchctl", &["bootstrap", "system", &plist]),
            ],
        }
    } else {
        let timer = format!("{unit}.timer");
        match job {
            Job::Enable => vec![cmd("systemctl", &["enable", "--now", &timer])],
            Job::Disable => vec![cmd("systemctl", &["disable", "--now", &timer])],
            Job::Reload => vec![cmd("systemctl", &["restart", &timer])],
        }
    }
}

/// Run the commands for the timed job `unit`. A failure of `bootout` or of a
/// `bootstrap` that finds the job loaded already is not an error.
pub fn job(aso: &Aso, exec: &dyn Exec, unit: &str, job: Job) -> Result<(), String> {
    for (bin, args) in job_commands(aso, unit, job) {
        let ran = exec.run(&bin, &args, &[])?;
        let lenient = aso.mac;
        if !ran.ok() && !lenient {
            return Err(format!(
                "`{}` failed: {}",
                command_line(&bin, &args),
                ran.last_line()
            ));
        }
    }
    Ok(())
}
