//! The three programs aso installs, and what aso knows of each.

use std::path::PathBuf;

use crate::config::Aso;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Program {
    /// The AIR server (stock air_server 1.0.1), behind nginx.
    Air,
    /// The service's FSP mint.
    Fsp,
    /// The service's ecash-wallet.
    Wallet,
}

impl Program {
    /// The order a full upgrade goes in: the order the services start, then the
    /// wallet.
    pub const ALL: [Program; 3] = [Program::Air, Program::Fsp, Program::Wallet];

    pub fn name(self) -> &'static str {
        match self {
            Program::Air => "air_server",
            Program::Fsp => "fsp-mint",
            Program::Wallet => "ecash-wallet",
        }
    }

    pub fn parse(text: &str) -> Option<Program> {
        Program::ALL.into_iter().find(|p| p.name() == text)
    }

    /// The `aso.toml` section that holds the program's `bin` and `version`.
    pub fn section(self) -> &'static str {
        match self {
            Program::Air => "air",
            Program::Fsp => "fsp",
            Program::Wallet => "wallet",
        }
    }

    /// The pin: the `version` `aso.toml` names for this program, if any.
    pub fn pin(self, aso: &Aso) -> Option<String> {
        aso.opt(&format!("{}.version", self.section()))
    }

    /// The `bin` setting: the operator's file, used until a release is installed
    /// and for a program that has no releases.
    pub fn bin(self, aso: &Aso) -> String {
        aso.text(&format!("{}.bin", self.section()))
    }

    /// The oldest release aso works with, and why; `None` for air_server, whose
    /// version is only the operator's note. fsp-mint 0.8.0 has `export-esp
    /// --serve-only`, which the automatic redeem needs so that it never writes
    /// fsp-mint's data folder as root; ecash-wallet 0.7.0 has `redeem
    /// --service-key-file`.
    pub fn min_version(self) -> Option<(&'static str, &'static str)> {
        match self {
            Program::Air => None,
            Program::Fsp => Some(("0.8.0", "export-esp --serve-only")),
            Program::Wallet => Some(("0.7.0", "redeem --service-key-file")),
        }
    }

    /// Whether upstream publishes release binaries for this program. air_server
    /// (ramp-stack) publishes none; when it does, this is the one place that
    /// changes.
    pub fn publishes(self) -> bool {
        !matches!(self, Program::Air)
    }

    /// Whether the program runs as a service.
    pub fn is_service(self) -> bool {
        !matches!(self, Program::Wallet)
    }

    /// Where the service install keeps this program's binary.
    pub fn installed_path(self, aso: &Aso) -> PathBuf {
        aso.bin_dir().join(self.name())
    }

    /// The systemd unit name, and the launchd label without its prefix.
    pub fn unit(self) -> &'static str {
        match self {
            Program::Air => "aso-air",
            Program::Fsp => "aso-fsp",
            Program::Wallet => "aso-wallet",
        }
    }

    /// The log the program's output goes to.
    pub fn log_name(self) -> &'static str {
        match self {
            Program::Air => "air_server.log",
            Program::Fsp => "fsp-mint.log",
            Program::Wallet => "ecash-wallet.log",
        }
    }

    /// What an upgrade of this program costs, and why, in these words.
    pub fn method(self) -> &'static str {
        match self {
            Program::Air => {
                "BRIEF DOWNTIME: air_server is stopped and the new one started. Stock air_server 1.0.1 has one SQLite database and one process that owns it and both sockets, so it cannot hand over; sessions are kept in the database, and fsp-mint's AIR client reconnects when it is back"
            }
            Program::Fsp => {
                "BRIEF DOWNTIME: fsp-mint is stopped and the new one started. One process may write its data folder (the writer lock), so the new one starts only after the old one has exited; wallets' AIR requests wait in the mailbox slots and are answered when it is back"
            }
            Program::Wallet => {
                "no downtime: ecash-wallet is a program run on demand, not a service; the file is replaced between two runs"
            }
        }
    }
}
