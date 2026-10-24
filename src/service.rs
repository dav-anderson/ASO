//! The service manager's files: a systemd unit (Linux) or a launchd property
//! list (macOS) for air_server and for fsp-mint, the env file each reads, the
//! nginx server block, the job that keeps the logs under their cap while the
//! services run, and the job that runs the automatic redeem.
//!
//! **Start order.** air_server first, then fsp-mint: on Linux `After=` and
//! `Wants=`; launchd has no start order, so `aso start` starts air_server and
//! waits for its health before it starts fsp-mint ([`crate::ctl`]).
//!
//! **Restart on failure.** systemd `Restart=on-failure`; launchd `KeepAlive`
//! with `SuccessfulExit` false. A crash is fault recovery and stays: the lock
//! (fsp-mint) and the database (air_server) are released by the operating system
//! when the process ends, so the restart never waits on a stale file.
//!
//! **Logs.** Each unit starts the program through `sh`, which first runs
//! `aso logs rotate LOG`: a log over `[service] log_max_size_mb` is moved aside
//! and only the newest `[service] log_keep` `.bak` files stay ([`crate::logs`]).
//! The caps travel in the env file (`ASO_SERVICE_LOG_*`), so a change reaches the
//! next start with no unit rewrite. The programs are meant never to stop, so the
//! logs job ([`logs_files`]) runs `aso logs rotate --running` on both logs every
//! [`LOGS_EVERY_MINUTES`] minutes, as the service account, with the same caps
//! from the same env file.

use std::path::{Path, PathBuf};

use air_minterface::{Managed, air};

use crate::config::Aso;
use crate::generate;
use crate::program::Program;
use crate::template::{expand, plain, protect_home};

const SERVICE: &str = include_str!("../ops/systemd/service.service");
const PLIST: &str = include_str!("../ops/launchd/service.plist");
const NGINX_HEADER: &str = include_str!("../ops/nginx-air.header");
const REDEEM_SERVICE: &str = include_str!("../ops/systemd/redeem.service");
const REDEEM_TIMER: &str = include_str!("../ops/systemd/redeem.timer");
const REDEEM_PLIST: &str = include_str!("../ops/launchd/redeem.plist");
const LOGS_SERVICE: &str = include_str!("../ops/systemd/logs.service");
const LOGS_TIMER: &str = include_str!("../ops/systemd/logs.timer");
const LOGS_PLIST: &str = include_str!("../ops/launchd/logs.plist");

/// How often the logs job checks the logs of the running services. A log can
/// pass `[service] log_max_size_mb` by what is written in this time.
pub const LOGS_EVERY_MINUTES: u64 = 5;

/// How long a stop may take before the program is killed. fsp-mint finishes the
/// request it holds (3 replies of up to air.timeout_secs 10 s), drains its local
/// routes and flushes; the same figures as spi's mints. air_server stops at once.
pub const FSP_STOP_SECS: u64 = 120;
pub const AIR_STOP_SECS: u64 = 30;

/// The systemd unit file name of a service program.
pub fn unit_file(aso: &Aso, program: Program) -> PathBuf {
    aso.unit_dir().join(format!("{}.service", program.unit()))
}

/// The launchd label of a service program.
pub fn label(program: Program) -> String {
    format!("org.servicepoints.{}", program.unit())
}

/// The launchd property list of a service program.
pub fn plist_file(aso: &Aso, program: Program) -> PathBuf {
    aso.unit_dir().join(format!("{}.plist", label(program)))
}

/// The service definition's file (unit or property list).
pub fn definition_file(aso: &Aso, program: Program) -> PathBuf {
    if aso.mac {
        plist_file(aso, program)
    } else {
        unit_file(aso, program)
    }
}

pub fn env_file(aso: &Aso, program: Program) -> PathBuf {
    match program {
        Program::Air => aso.etc_dir().join("air-server.env"),
        _ => aso.etc_dir().join("fsp-mint.env"),
    }
}

/// The files of one service program, as `(path, text)`: the env file, then the
/// definition. Nothing is written.
pub fn service_files(
    aso: &Aso,
    exe: &Path,
    program: Program,
) -> Result<Vec<(PathBuf, String)>, String> {
    let (env_text, description, after, wants, args, workdir, stop_secs, rw, reload) = match program
    {
        Program::Air => (
            generate::air_env(aso),
            "the AIR server (air_server): the mailbox the mint and the wallets reach each other through",
            "network-online.target".to_string(),
            "network-online.target".to_string(),
            String::new(),
            aso.data_dir().join("air"),
            AIR_STOP_SECS,
            vec![
                aso.data_dir().join("air").display().to_string(),
                aso.log_dir().display().to_string(),
            ],
            String::new(),
        ),
        Program::Fsp => (
            generate::fsp_env(aso),
            "the FSP mint (fsp-mint serve): the service's own mint",
            format!("network-online.target {}.service", Program::Air.unit()),
            format!("network-online.target {}.service", Program::Air.unit()),
            " serve".to_string(),
            aso.fsp_base(),
            FSP_STOP_SECS,
            vec![
                aso.fsp_base().display().to_string(),
                aso.log_dir().display().to_string(),
            ],
            "# `systemctl reload` sends SIGHUP: fsp-mint reads its settings file again and\n# applies every setting that can change while it runs, with no stop. A setting\n# that cannot is named in its log and waits for a restart (`aso restart`).\nExecReload=/bin/kill -HUP $MAINPID".to_string(),
        ),
        Program::Wallet => return Err("ecash-wallet is not a service".into()),
    };
    let exe_text = exe.display().to_string();
    let log = aso.log_dir().join(program.log_name()).display().to_string();
    let bin = program.installed_path(aso).display().to_string();
    let env_path = env_file(aso, program);
    let workdir = workdir.display().to_string();
    let user = aso.text("service.user");
    let group = aso.text("service.group");
    for (what, value) in [
        ("the path of aso", exe_text.as_str()),
        ("the program's path", bin.as_str()),
        ("the log", log.as_str()),
        ("the env file", &env_path.display().to_string()),
        ("the working folder", workdir.as_str()),
    ] {
        plain(what, value)?;
    }
    let rw_refs: Vec<&str> = rw.iter().map(String::as_str).collect();
    let mut vars = vec![
        ("name", program.unit().to_string()),
        ("description", description.to_string()),
        ("after", after),
        ("wants", wants),
        ("user", user),
        ("group", group),
        ("env_file", env_path.display().to_string()),
        ("workdir", workdir),
        ("exe", exe_text),
        ("log", log),
        ("program", bin),
        ("args", args),
        ("stop_secs", stop_secs.to_string()),
        ("rw_paths", rw.join(" ")),
        ("protect_home", protect_home(&rw_refs).to_string()),
        ("reload_line", reload),
    ];
    let definition = if aso.mac {
        // The property list has no ExecReload; `aso reload` sends SIGHUP itself.
        vars.retain(|(k, _)| *k != "reload_line");
        expand("the service property list", PLIST, &vars, true)?
    } else {
        expand("the service unit", SERVICE, &vars, false)?
    };
    Ok(vec![
        (env_path, env_text),
        (definition_file(aso, program), definition),
    ])
}

/// The nginx server block, when `air.public_name` is set.
pub fn nginx_file(aso: &Aso) -> Result<Option<(PathBuf, String)>, String> {
    let Some(name) = aso.opt("air.public_name") else {
        return Ok(None);
    };
    let vars = vec![
        ("public_name", name),
        ("public_cert", aso.text("air.public_cert")),
        ("public_key", aso.text("air.public_key")),
        ("ws_addr", aso.text("air.ws_addr")),
    ];
    for (what, value) in &vars {
        plain(&format!("air {what}"), value)?;
    }
    Ok(Some((
        aso.path("air.nginx_conf"),
        expand(
            "the nginx file",
            &air::nginx_template(NGINX_HEADER),
            &vars,
            false,
        )?,
    )))
}

/// The job that runs the automatic redeem every `redeem.interval_hours`, as
/// `(path, text)`: on Linux a service and a timer, on macOS a property list.
pub fn redeem_files(
    aso: &Aso,
    exe: &Path,
    config: &Path,
) -> Result<Vec<(PathBuf, String)>, String> {
    let exe_text = exe.display().to_string();
    let dir = aso.dir.display().to_string();
    let config_text = config.display().to_string();
    for (what, value) in [
        ("the path of aso", exe_text.as_str()),
        ("the aso folder", dir.as_str()),
        ("the settings file", config_text.as_str()),
        ("the home folder", aso.home.as_str()),
    ] {
        plain(what, value)?;
    }
    let hours = aso.whole("redeem.interval_hours");
    let vars = vec![
        ("home", aso.home.clone()),
        ("exe", exe_text),
        ("dir", dir),
        ("config", config_text),
        ("interval_hours", hours.to_string()),
        ("interval_secs", (hours * 3600).to_string()),
    ];
    if aso.mac {
        Ok(vec![(
            aso.unit_dir().join(format!("{}.plist", redeem_label())),
            expand("the redeem property list", REDEEM_PLIST, &vars, true)?,
        )])
    } else {
        Ok(vec![
            (
                aso.unit_dir().join(format!("{REDEEM_UNIT}.service")),
                expand("the redeem unit", REDEEM_SERVICE, &vars, false)?,
            ),
            (
                aso.unit_dir().join(format!("{REDEEM_UNIT}.timer")),
                expand("the redeem timer", REDEEM_TIMER, &vars, false)?,
            ),
        ])
    }
}

pub const REDEEM_UNIT: &str = "aso-redeem";

pub fn redeem_label() -> String {
    format!("org.servicepoints.{REDEEM_UNIT}")
}

pub const LOGS_UNIT: &str = "aso-logs";

pub fn logs_label() -> String {
    format!("org.servicepoints.{LOGS_UNIT}")
}

/// The job that keeps `fsp-mint.log` and `air_server.log` under the cap while
/// the services run, as `(path, text)`: on Linux a service and a timer, on
/// macOS a property list. It runs as the service account, which owns the logs.
pub fn logs_files(aso: &Aso, exe: &Path) -> Result<Vec<(PathBuf, String)>, String> {
    let exe_text = exe.display().to_string();
    let env_path = env_file(aso, Program::Fsp).display().to_string();
    let log_dir = aso.log_dir().display().to_string();
    let logs: Vec<String> = [Program::Fsp, Program::Air]
        .iter()
        .map(|p| aso.log_dir().join(p.log_name()).display().to_string())
        .collect();
    for (what, value) in [
        ("the path of aso", exe_text.as_str()),
        ("the env file", env_path.as_str()),
        ("the log", logs[0].as_str()),
        ("the log", logs[1].as_str()),
    ] {
        plain(what, value)?;
    }
    let vars = vec![
        ("user", aso.text("service.user")),
        ("group", aso.text("service.group")),
        ("env_file", env_path),
        ("exe", exe_text),
        // `plain` paths are single shell words as they are.
        ("logs", logs.join(" ")),
        (
            "protect_home",
            protect_home(&[log_dir.as_str()]).to_string(),
        ),
        ("log_dir", log_dir),
        ("minutes", LOGS_EVERY_MINUTES.to_string()),
        ("secs", (LOGS_EVERY_MINUTES * 60).to_string()),
    ];
    if aso.mac {
        Ok(vec![(
            aso.unit_dir().join(format!("{}.plist", logs_label())),
            expand("the logs property list", LOGS_PLIST, &vars, true)?,
        )])
    } else {
        Ok(vec![
            (
                aso.unit_dir().join(format!("{LOGS_UNIT}.service")),
                expand("the logs unit", LOGS_SERVICE, &vars, false)?,
            ),
            (
                aso.unit_dir().join(format!("{LOGS_UNIT}.timer")),
                expand("the logs timer", LOGS_TIMER, &vars, false)?,
            ),
        ])
    }
}
