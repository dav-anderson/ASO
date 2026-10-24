//! aso's side of air-interface: what the shared code needs to know about this
//! operator ([`Operator`], implemented on [`Aso`]) and about each program
//! ([`Managed`], implemented on [`Program`]). The shared code does the rest;
//! the sentences that are aso's own wording are said here, so the output is
//! what it has always been.

use std::path::{Path, PathBuf};
use std::time::Duration;

use air_interface::exec::Exec;
use air_interface::schedule::Job;
use air_interface::version::{Tags, program_version};
use air_interface::{Managed, Operator, Say};

use crate::config::Aso;
use crate::ctl::{self, Action};
use crate::program::Program;
use crate::{binary, state};

impl Operator for Aso {
    const NAME: &'static str = "aso";
    const CONFIG_NAME: &'static str = "aso.toml";
    const UPGRADE_JOB: Job = Job {
        unit: "aso-upgrade",
        label: "org.servicepoints.aso-upgrade",
        service: include_str!("../ops/systemd/upgrade.service"),
        timer: include_str!("../ops/systemd/upgrade.timer"),
        plist: include_str!("../ops/launchd/upgrade.plist"),
        all_programs: "air_server, fsp-mint and ecash-wallet (all three)",
        log_name: "aso-upgrade.log",
    };

    fn dir(&self) -> &Path {
        &self.dir
    }

    fn home(&self) -> &str {
        &self.home
    }

    fn mac(&self) -> bool {
        self.mac
    }

    fn log_dir(&self) -> PathBuf {
        Aso::log_dir(self)
    }

    fn unit_dir(&self) -> PathBuf {
        Aso::unit_dir(self)
    }

    fn release_url(&self) -> String {
        self.text("release.url")
    }

    fn release_keep(&self) -> usize {
        self.whole("release.keep") as usize
    }
}

impl Managed for Program {
    type Op = Aso;

    fn all() -> &'static [Program] {
        &Program::ALL
    }

    fn name(self) -> &'static str {
        match self {
            Program::Air => "air_server",
            Program::Fsp => "fsp-mint",
            Program::Wallet => "ecash-wallet",
        }
    }

    /// The `aso.toml` section that holds the program's `bin` and `version`.
    fn section(self) -> &'static str {
        match self {
            Program::Air => "air",
            Program::Fsp => "fsp",
            Program::Wallet => "wallet",
        }
    }

    /// Whether upstream publishes release binaries for this program. air_server
    /// (ramp-stack) publishes none; when it does, this is the one place that
    /// changes.
    fn publishes(self) -> bool {
        !matches!(self, Program::Air)
    }

    /// fsp-mint and ecash-wallet are `x.y.z`: a `0.8` tag is ignored.
    fn tags(self) -> Tags {
        Tags::Strict
    }

    /// Where the service install keeps this program's binary.
    fn installed_path(self, aso: &Aso) -> PathBuf {
        aso.bin_dir().join(self.name())
    }

    /// The `bin` setting: the operator's file, used until a release is installed
    /// and for a program that has no releases.
    fn bin(self, aso: &Aso) -> String {
        aso.text(&format!("{}.bin", self.section()))
    }

    /// The pin: the `version` `aso.toml` names for this program, if any.
    fn pin(self, aso: &Aso) -> Option<String> {
        aso.opt(&format!("{}.version", self.section()))
    }

    /// Whether the program runs as a service.
    fn is_service(self) -> bool {
        !matches!(self, Program::Wallet)
    }

    /// Only ecash-wallet prints its version; fsp-mint (0.8.0 too) and air_server
    /// are known by aso's record of the exact file.
    fn prints_version(self) -> bool {
        self == Program::Wallet
    }

    /// The installed program: the service install's copy, as aso recorded it.
    fn installed_version(self, aso: &Aso, exec: &dyn Exec) -> Result<String, String> {
        let path = self.installed_path(aso);
        if !path.is_file() {
            return Err(format!("{} does not exist", path.display()));
        }
        self.reported(aso, exec, &path)
    }

    /// The version a program reports for the binary at `path`, or why it cannot.
    /// ecash-wallet says so itself; fsp-mint and air_server only through aso's
    /// record of that exact file.
    fn reported(self, aso: &Aso, exec: &dyn Exec, path: &Path) -> Result<String, String> {
        if self != Program::Wallet {
            return state::recorded_version(aso, self, path).ok_or_else(|| {
                format!(
                    "aso has no record of the version of {} ({} prints no version)",
                    path.display(),
                    self.name()
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
        program_version(self.name(), first).ok_or_else(|| {
            format!(
                "`{} --version` printed `{first}`, which has no version",
                path.display()
            )
        })
    }

    fn restart(self, aso: &Aso, exec: &dyn Exec) -> Result<(), String> {
        ctl::apply(aso, exec, self, Action::Restart)
    }

    fn service_name(self, _aso: &Aso) -> String {
        self.unit().to_string()
    }

    /// How long the service was down, and the settings it now runs with.
    fn after_start(self, aso: &Aso, took: Duration) -> Vec<String> {
        let name = self.name();
        let mut lines = vec![format!(
            "{name}: was briefly down while the new version started: back, healthy, after {:.1} s",
            took.as_secs_f64()
        )];
        if let Err(e) = state::mark_applied(aso, self) {
            lines.push(format!("WARNING: {name}: {e}"));
        }
        lines
    }

    fn record(
        self,
        aso: &Aso,
        _from_release: bool,
        version: &str,
        installed: &Path,
    ) -> Result<(), String> {
        binary::record(aso, self, Some(version), installed)
    }

    fn say(self, _aso: &Aso, what: Say<'_>) -> String {
        let (name, section) = (self.name(), self.section());
        match what {
            Say::NoReleases => {
                "no releases published upstream: the binary is yours (air.bin)".into()
            }
            // aso manages every program it knows.
            Say::UnmanagedStatus | Say::Unmanaged => format!("aso manages {name}"),
            Say::NoVersionUnpublished => format!(
                "aso.toml names no version for {name} (set `version` under [{section}]: {name} publishes no releases, so the version is your word for the file `bin` names)"
            ),
            Say::NoVersionNoRelease => format!(
                "aso.toml names no version for {name}, and the release source lists no release for it (set `version` under [{section}] to use the file `bin` names)"
            ),
            Say::NoVersionNotFound(why) => format!(
                "aso.toml names no version for {name}, and the newest release could not be found: {why}"
            ),
            Say::NotInstalled(path) => format!(
                "{} does not exist: `aso upgrade` upgrades the service install, so run `aso install` first",
                path.display()
            ),
            Say::BinMissing(bin) => {
                format!("cannot find the build `{bin}` ({section}.bin in aso.toml names it)")
            }
            Say::BinIsInstalled(path) => format!(
                "`{section}.bin` names the installed file {} itself: point it at the new build",
                path.display()
            ),
            Say::Plan { from, target } => {
                format!("{name}: {from} -> {target}: {}", self.method())
            }
            Say::DryRunDownload => format!(
                "{name}: the build would be downloaded from the release source and checked against its SHA-256 before anything is stopped"
            ),
            Say::Done { target } => format!("{name}: now {target}"),
            Say::RunAs => "aso".into(),
        }
    }
}

/// A copy of `aso` with the pins of `programs` removed, so that the newest
/// release is the target.
pub fn unpinned(aso: &Aso, programs: &[Program]) -> Aso {
    let mut out = aso.clone();
    for p in programs {
        let key = format!("{}.version", p.section());
        for e in out.entries.iter_mut().filter(|e| e.key == key) {
            e.value = toml::Value::String(String::new());
            e.source = crate::config::Source::Default;
        }
    }
    out
}
