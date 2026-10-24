//! The files aso generates from `aso.toml`: fsp-mint's config and environment,
//! air_server's environment (it takes environment variables only), and
//! ecash-wallet's config. A setting `aso.toml` sets reaches the component's file
//! as written, named by aso or not; a setting it does not set is left to the
//! component's own default.

use toml::{Table, Value};

use crate::config::Aso;
use crate::state::State;

const HEADER: &str = "# Written by `aso` from aso.toml. Don't edit: change aso.toml and run\n# `aso reload` (or `aso install`), which writes this file again.\n";

fn table_text(table: &Table) -> Result<String, String> {
    toml::to_string(table).map_err(|e| format!("cannot write the settings as TOML: {e}"))
}

/// Set `section.name = value` in `table` unless it is already there.
fn inject(table: &mut Table, section: &str, name: &str, value: &str) {
    if value.is_empty() {
        return;
    }
    let entry = table
        .entry(section.to_string())
        .or_insert_with(|| Value::Table(Table::new()));
    if let Value::Table(t) = entry {
        t.entry(name.to_string())
            .or_insert_with(|| Value::String(value.to_string()));
    }
}

/// The AIR server's name: `air.name`, else what the install read from
/// air_server's first start.
pub fn air_name(aso: &Aso, state: &State) -> String {
    aso.opt("air.name")
        .unwrap_or_else(|| state.air_name.clone())
}

/// fsp-mint's `fsp-mint.toml`: the `[fsp.*]` tables of `aso.toml`, with
/// `service.did`, `air.server` and `air.host` added when `aso.toml` does not set
/// them.
pub fn fsp_toml(aso: &Aso, state: &State) -> Result<String, String> {
    let mut table = aso.component_table("fsp");
    inject(&mut table, "service", "did", &state.service_did);
    inject(&mut table, "air", "server", &air_name(aso, state));
    inject(&mut table, "air", "host", &aso.text("air.host"));
    Ok(format!("{HEADER}{}", table_text(&table)?))
}

/// ecash-wallet's `config.toml`.
pub fn wallet_toml(aso: &Aso) -> Result<String, String> {
    Ok(format!(
        "{HEADER}{}",
        table_text(&aso.component_table("wallet"))?
    ))
}

/// fsp-mint's environment file: where its data and config are, and the log caps
/// the unit's `aso logs rotate` reads.
pub fn fsp_env(aso: &Aso) -> String {
    format!(
        "{HEADER}FSP_DATA_DIR={}\nFSP_CONFIG={}\n{}",
        aso.fsp_base().display(),
        aso.fsp_config_file().display(),
        log_caps(aso)
    )
}

/// air_server's environment file.
pub fn air_env(aso: &Aso) -> String {
    let mut text = format!(
        "{HEADER}AIR_STORAGE={}\nAIR_WS_ADDR={}\nAIR_HEALTH_ADDR={}\n",
        aso.text("air.storage"),
        aso.text("air.ws_addr"),
        aso.text("air.health_addr"),
    );
    if let Some(identity) = aso.opt("air.identity") {
        text.push_str(&format!("AIR_SECRET_FILE={identity}\n"));
    }
    if let (Some(addr), Some(cert), Some(key)) = (
        aso.opt("air.tls_addr"),
        aso.opt("air.tls_cert"),
        aso.opt("air.tls_key"),
    ) {
        text.push_str(&format!(
            "AIR_TLS_ADDR={addr}\nAIR_TLS_CERT={cert}\nAIR_TLS_KEY={key}\n"
        ));
    }
    text.push_str(&log_caps(aso));
    text
}

fn log_caps(aso: &Aso) -> String {
    format!(
        "ASO_SERVICE_LOG_MAX_SIZE_MB={}\nASO_SERVICE_LOG_KEEP={}\n",
        aso.log_max_mb(),
        aso.log_keep()
    )
}

/// This service's `air.fsp` entry for a wallet: `<service DID>@<AIR server
/// name>@<public host>`. `None` until the DID, the name and `air.public_name`
/// are all known.
pub fn fsp_entry(aso: &Aso, state: &State) -> Option<String> {
    let name = air_name(aso, state);
    let host = aso.opt("air.public_name")?;
    if state.service_did.is_empty() || name.is_empty() {
        return None;
    }
    Some(format!("{}@{name}@{host}", state.service_did))
}

/// The wallet's network flag: none for mainnet.
pub fn network_flag(network: &str) -> Option<String> {
    (network != "main").then(|| format!("--{network}"))
}
