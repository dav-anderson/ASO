//! The three programs aso installs, and what aso knows of each. What it shares
//! with spi (name, section, pin, where it is installed, ...) is in
//! [`crate::interface`], as the [`air_minterface::Managed`] impl.

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

    /// The oldest release aso works with, and why; `None` for air_server, whose
    /// version is only the operator's note. fsp-mint 0.10.0 is the release that
    /// uses the one shared sp-wire keyset ID (wire version 5) and publishes its
    /// key sets on AIR, so a wallet can check the keys it is signed with; it
    /// also has `export-esp --serve-only` (from 0.8.0), which the automatic
    /// redeem needs so that it never writes fsp-mint's data folder as root.
    /// ecash-wallet 0.9.0 is the release that makes that key check (and has
    /// the `fsp.blacklist_on_key_mismatch` setting aso passes through); an
    /// older wallet or fsp-mint speaks wire version 4 and cannot talk to the
    /// 0.9.0/0.11.0 mints. ecash-wallet also has the claim flow (`buy-psp
    /// --mint-key` with no amounts, then `claim-psp`), which `aso install`
    /// runs, and the `[psp]` settings aso passes through.
    pub fn min_version(self) -> Option<(&'static str, &'static str)> {
        match self {
            Program::Air => None,
            Program::Fsp => Some(("0.10.0", "wire version 5 and the keys it publishes")),
            Program::Wallet => Some(("0.9.0", "wire version 5 and the key check")),
        }
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
