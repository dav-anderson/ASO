//! `aso status` and the smart rescan: check what exists, say what is missing.
//!
//! The install has four steps (a. the AIR service, b. FSP, c. the wallet, d.
//! funding FSP at another service for backups). Each is a list of facts that
//! can be checked on the machine; a step is done when all of its facts hold, and
//! `aso install` builds only the facts that do not. The service is **ready**
//! only when air_server and fsp-mint answer their health checks and the wallet
//! is funded (PSP above zero); step d is recommended and does not hold it back.

use air_minterface::Managed;

use crate::config::Aso;
use crate::ctl;
use crate::exec::Exec;
use crate::generate;
use crate::program::Program;
use crate::service;
use crate::state::{self, State};
use crate::wallet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Air,
    Fsp,
    Wallet,
    Backup,
}

impl Step {
    pub const ALL: [Step; 4] = [Step::Air, Step::Fsp, Step::Wallet, Step::Backup];

    pub fn label(self) -> &'static str {
        match self {
            Step::Air => "a. AIR service (air_server behind nginx)",
            Step::Fsp => "b. FSP (fsp-mint serve)",
            Step::Wallet => "c. wallet funded (ecash-wallet)",
            Step::Backup => "d. FSP funded at another service, for backups (recommended)",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Step::Air => "air",
            Step::Fsp => "fsp",
            Step::Wallet => "wallet",
            Step::Backup => "backup",
        }
    }

    pub fn parse(text: &str) -> Option<Step> {
        Step::ALL.into_iter().find(|s| s.name() == text)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    pub step: Step,
    pub what: String,
    pub ok: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Process {
    pub name: String,
    pub running: String,
    pub health: String,
    pub version: String,
}

#[derive(Clone, Debug)]
pub struct Status {
    pub checks: Vec<Check>,
    pub processes: Vec<Process>,
    pub ready: bool,
    pub redeem: String,
}

impl Status {
    /// Whether every fact of the step holds. Step d is done when it was done or
    /// skipped.
    pub fn done(&self, step: Step) -> bool {
        self.checks.iter().filter(|c| c.step == step).all(|c| c.ok)
    }

    /// What is missing for a step.
    pub fn missing(&self, step: Step) -> Vec<String> {
        self.checks
            .iter()
            .filter(|c| c.step == step && !c.ok)
            .map(|c| c.what.clone())
            .collect()
    }
}

/// The fsp-mint settings that `serve` refuses to start without.
pub const FSP_REQUIRED: [&str; 3] = ["fsp.air.esp_did", "fsp.air.esp_server", "fsp.air.esp_host"];

/// Those of [`FSP_REQUIRED`] that `aso.toml` leaves empty.
pub fn fsp_missing(aso: &Aso) -> Vec<&'static str> {
    FSP_REQUIRED
        .iter()
        .copied()
        .filter(|k| aso.text(k).is_empty())
        .collect()
}

pub fn collect(aso: &Aso, exec: &dyn Exec) -> Result<Status, String> {
    let st: State = state::read(aso)?;
    let mut checks = Vec::new();
    let mut add = |step: Step, what: &str, ok: bool| {
        checks.push(Check {
            step,
            what: what.to_string(),
            ok,
        })
    };
    let air_health = ctl::health(aso, Program::Air);
    let fsp_health = ctl::health(aso, Program::Fsp);

    add(
        Step::Air,
        "air_server is installed",
        Program::Air.installed_path(aso).is_file(),
    );
    add(
        Step::Air,
        "air_server's settings and service definition are written",
        service::env_file(aso, Program::Air).is_file()
            && service::definition_file(aso, Program::Air).is_file(),
    );
    if let Some(nginx) = aso
        .opt("air.public_name")
        .map(|_| aso.path("air.nginx_conf"))
    {
        add(
            Step::Air,
            "the nginx file is written (air.public_name is set)",
            nginx.is_file(),
        );
    }
    add(
        Step::Air,
        "the AIR server's name is known",
        !generate::air_name(aso, &st).is_empty(),
    );
    add(
        Step::Air,
        &format!(
            "air_server answers its health check ({})",
            air_health.detail
        ),
        air_health.up,
    );

    add(
        Step::Fsp,
        "fsp-mint is installed",
        Program::Fsp.installed_path(aso).is_file(),
    );
    add(
        Step::Fsp,
        "the service key is made and its DID known",
        !st.service_did.is_empty() && aso.service_key_file().is_file(),
    );
    add(
        Step::Fsp,
        "fsp-mint's settings and service definition are written",
        aso.fsp_config_file().is_file()
            && service::env_file(aso, Program::Fsp).is_file()
            && service::definition_file(aso, Program::Fsp).is_file(),
    );
    let missing = fsp_missing(aso);
    add(
        Step::Fsp,
        &if missing.is_empty() {
            "the ESP mint's settings are in aso.toml".to_string()
        } else {
            format!(
                "aso.toml sets {} (fsp-mint serve refuses to start without them)",
                missing.join(", ")
            )
        },
        missing.is_empty(),
    );
    add(
        Step::Fsp,
        &format!("fsp-mint answers its health check ({})", fsp_health.detail),
        fsp_health.up,
    );

    add(
        Step::Wallet,
        "ecash-wallet is installed",
        Program::Wallet.installed_path(aso).is_file(),
    );
    add(
        Step::Wallet,
        "ecash-wallet's settings are written",
        aso.wallet_config_file().is_file(),
    );
    let funded = if Program::Wallet.installed_path(aso).is_file() {
        match wallet::balance(aso, exec) {
            Ok(b) => (b.psp > 0, format!("PSP {}", b.psp)),
            Err(e) => (false, format!("balance not read: {e}")),
        }
    } else {
        (false, "ecash-wallet is not installed".to_string())
    };
    add(
        Step::Wallet,
        &format!("the wallet is funded ({})", funded.1),
        funded.0,
    );

    add(
        Step::Backup,
        &format!(
            "FSP funded at another service, or skipped ({})",
            if st.backup.is_empty() {
                "not done"
            } else {
                &st.backup
            }
        ),
        !st.backup.is_empty(),
    );

    let mut processes = Vec::new();
    for (program, health) in [(Program::Air, &air_health), (Program::Fsp, &fsp_health)] {
        let running = match ctl::is_running(aso, exec, program) {
            Some(true) => "running",
            Some(false) => "not running",
            None => "unknown",
        };
        let installed = program.installed_path(aso);
        let version = if installed.is_file() {
            program
                .reported(aso, exec, &installed)
                .unwrap_or_else(|_| "unknown".to_string())
        } else {
            "not installed".to_string()
        };
        processes.push(Process {
            name: program.name().to_string(),
            running: running.to_string(),
            health: if health.up {
                "ok".to_string()
            } else {
                format!("not ok ({})", health.detail)
            },
            version,
        });
    }
    let ready = air_health.up && fsp_health.up && funded.0;
    let redeem = match aso.opt("redeem.payout") {
        Some(p) => format!(
            "automatic redeem: on, every {} h, payout to {p}",
            aso.whole("redeem.interval_hours")
        ),
        None => "automatic redeem: off (redeem.payout is not set in aso.toml)".to_string(),
    };
    Ok(Status {
        checks,
        processes,
        ready,
        redeem,
    })
}

pub fn render(s: &Status) -> Vec<String> {
    let mut lines = Vec::new();
    for step in Step::ALL {
        if s.done(step) {
            lines.push(format!("[done]     {}", step.label()));
        } else {
            lines.push(format!("[not done] {}", step.label()));
            for m in s.missing(step) {
                lines.push(format!("             missing: {m}"));
            }
        }
    }
    lines.push(String::new());
    for p in &s.processes {
        lines.push(format!(
            "{:<12} {:<12} health {:<8} version {}",
            p.name, p.running, p.health, p.version
        ));
    }
    lines.push(s.redeem.clone());
    lines.push(String::new());
    lines.push(if s.ready {
        "ready: air_server and fsp-mint are healthy and the wallet is funded".to_string()
    } else {
        let mut why = Vec::new();
        for step in [Step::Air, Step::Fsp, Step::Wallet] {
            if !s.done(step) {
                why.push(
                    step.label()
                        .split_once(". ")
                        .map(|x| x.1)
                        .unwrap_or("")
                        .to_string(),
                );
            }
        }
        format!("NOT ready: still to do: {}", why.join("; "))
    });
    lines
}
