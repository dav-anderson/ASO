//! Running the service's ecash-wallet, and reading what it prints.
//!
//! ecash-wallet is a separate installed program. aso calls it with
//! `--wallet FILE --config FILE [network flag] COMMAND ...`, so nothing depends
//! on the home folder of whoever runs aso. The service key is never typed on a
//! command line: `redeem` gets `--service-key-file`.

use air_minterface::Managed;

use crate::config::Aso;
use crate::exec::{Exec, Ran};
use crate::generate;
use crate::program::Program;
use air_minterface::files::find_binary;

/// The ecash-wallet binary: the installed copy, else the file `wallet.bin` names.
pub fn binary(aso: &Aso) -> String {
    let installed = Program::Wallet.installed_path(aso);
    if installed.is_file() {
        return installed.display().to_string();
    }
    find_binary(&Program::Wallet.bin(aso))
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| Program::Wallet.bin(aso))
}

/// The arguments before the command.
pub fn base_args(aso: &Aso) -> Vec<String> {
    let mut args = vec![
        "--wallet".to_string(),
        aso.wallet_file().display().to_string(),
        "--config".to_string(),
        aso.wallet_config_file().display().to_string(),
    ];
    args.extend(generate::network_flag(&aso.network()));
    args
}

/// Run `ecash-wallet <base args> <command>`. `Err` only when it cannot be run.
pub fn run(aso: &Aso, exec: &dyn Exec, command: &[String]) -> Result<Ran, String> {
    let mut args = base_args(aso);
    args.extend(command.iter().cloned());
    exec.run(&binary(aso), &args, &[])
}

/// Run it and require success; the error names the command and its last line.
pub fn run_ok(aso: &Aso, exec: &dyn Exec, command: &[String]) -> Result<Ran, String> {
    let ran = run(aso, exec, command)?;
    if ran.ok() {
        Ok(ran)
    } else {
        Err(format!(
            "`ecash-wallet {}` failed: {}",
            command.join(" "),
            ran.last_line()
        ))
    }
}

/// What `ecash-wallet balance` printed that aso reads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Balance {
    pub psp: u128,
    pub esp: u128,
    pub fsp: u128,
    /// Redemptions sent and not answered: `redeem --resume ID` sends them again.
    pub sent: Vec<String>,
    /// Redemptions the mint queued for payout: `redeem --collect` asks about them.
    pub queued: Vec<String>,
    /// The wallet's own output, to show.
    pub text: String,
}

/// Read the output of `ecash-wallet balance`: the lines `psp N`, `esp N`,
/// `fsp N`, `pending redemption ID (not answered yet)` and `pending redemption
/// ID (payout of N ESP queued at the mint)`.
pub fn parse_balance(text: &str) -> Result<Balance, String> {
    let mut b = Balance {
        text: text.to_string(),
        ..Balance::default()
    };
    let (mut psp, mut esp, mut fsp) = (None, None, None);
    for line in text.lines() {
        let mut words = line.split_whitespace();
        match (words.next(), words.next()) {
            (Some("psp"), Some(n)) if psp.is_none() => psp = n.parse::<u128>().ok(),
            (Some("esp"), Some(n)) if esp.is_none() => esp = n.parse::<u128>().ok(),
            (Some("fsp"), Some(n)) if fsp.is_none() => fsp = n.parse::<u128>().ok(),
            (Some("pending"), Some("redemption")) => {
                if let Some(id) = words.next() {
                    if line.contains("(not answered yet)") {
                        b.sent.push(id.to_string());
                    } else if line.contains("queued at the mint") {
                        b.queued.push(id.to_string());
                    }
                }
            }
            _ => {}
        }
    }
    b.psp = psp.ok_or("`ecash-wallet balance` printed no `psp` line")?;
    b.esp = esp.ok_or("`ecash-wallet balance` printed no `esp` line")?;
    b.fsp = fsp.ok_or("`ecash-wallet balance` printed no `fsp` line")?;
    Ok(b)
}

/// `ecash-wallet balance`, parsed.
pub fn balance(aso: &Aso, exec: &dyn Exec) -> Result<Balance, String> {
    let ran = run_ok(aso, exec, &["balance".to_string()])?;
    parse_balance(&ran.stdout)
}
