//! `aso.toml`: one file that sets every component of an AIR service.
//!
//! **Sections.** `[service]` (accounts, folders, log caps), `[air]` (air_server
//! and nginx), `[fsp]` (fsp-mint), `[wallet]` (ecash-wallet), `[redeem]` (the
//! automatic redeem) and `[release]` (where releases come from). Every setting
//! is in a table below with its default, what it does, and whether a change
//! `reloads live` or `needs a restart`; an unknown setting in `[service]`,
//! `[air]`, `[redeem]` or `[release]` is refused with every other problem.
//!
//! **Pass-through.** `[fsp.<section>]` and `[wallet.<section>]` hold the
//! component's own settings, written as they are in fsp-mint.toml and
//! ecash-wallet's config.toml (`[fsp.server] bind = "127.0.0.1:3200"`). The
//! settings aso knows ([`FSP`], [`WALLET`]) are listed by `aso config` with
//! their defaults and reload behaviour; a key aso does not name (a setting a
//! newer fsp-mint has, such as the finished-records caps of ROADMAP item 29)
//! is written to the component's file unchanged, and the component checks it.
//! Defaults are the component's own: aso writes into the generated file only
//! what `aso.toml` sets (and `service.did`, `air.server` and `air.host`, which
//! it knows from the install).
//!
//! **Where a value comes from**, highest first: an `ASO_<SECTION>_<KEY>`
//! environment variable (aso's own settings only), `aso.toml`, the default.
//! `aso config` prints each setting with its source.

use air_minterface::Managed;
use std::path::PathBuf;

use toml::{Table, Value};

pub const FILE_NAME: &str = "aso.toml";

/// Whether a change to a setting reaches the running program with no stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reload {
    /// `aso reload` applies it (fsp-mint reads its file again on SIGHUP; the
    /// wallet reads its file on every run; aso reads its own at each command).
    Live,
    /// It takes effect at the next restart, a brief downtime (`aso restart`).
    Restart,
}

impl Reload {
    pub fn words(self) -> &'static str {
        match self {
            Reload::Live => "reloads live",
            Reload::Restart => "needs a restart",
        }
    }
}

/// What a setting holds.
#[derive(Clone, Copy, Debug)]
pub enum Kind {
    Text,
    /// A whole number in `min..=max`.
    Whole(u64, u64),
    List,
    /// A component setting aso does not check: the component does.
    Raw,
}

pub struct Setting {
    pub key: &'static str,
    pub kind: Kind,
    /// The default, as text. `@user`, `@group`, `@unit_dir`, `@dir/x`, `@etc/x`
    /// and `@data/x` are worked out when the file is read. Empty: not set.
    pub default: &'static str,
    pub reload: Reload,
    pub what: &'static str,
}

const fn s(
    key: &'static str,
    kind: Kind,
    default: &'static str,
    reload: Reload,
    what: &'static str,
) -> Setting {
    Setting {
        key,
        kind,
        default,
        reload,
        what,
    }
}

use Kind::{List, Raw, Text, Whole};
use Reload::{Live, Restart};

/// aso's own settings. Order matters: `service.*` come first because later
/// defaults are inside the folders they set.
pub const SETTINGS: &[Setting] = &[
    s(
        "service.user",
        Text,
        "@user",
        Restart,
        "The account the services run as. Linux: `aso` (made by `aso install`). macOS: the account that runs `sudo aso install`.",
    ),
    s(
        "service.group",
        Text,
        "@group",
        Restart,
        "The group of that account.",
    ),
    s(
        "service.bin_dir",
        Text,
        "@dir/bin",
        Restart,
        "Where the installed programs are (fsp-mint, air_server, ecash-wallet); the services run these copies.",
    ),
    s(
        "service.etc_dir",
        Text,
        "@dir/etc",
        Restart,
        "Where aso writes the files it generates: fsp-mint.toml, the env files, the wallet's config, the nginx file.",
    ),
    s(
        "service.data_dir",
        Text,
        "@dir/data",
        Restart,
        "Where the data is: fsp/ (fsp-mint), air/ (air_server's STORAGE.db), wallet/ (ecash-wallet's file), redeem/ (hand-over files, owner-only, deleted once received).",
    ),
    s(
        "service.log_dir",
        Text,
        "@dir/logs",
        Restart,
        "Where the logs are: fsp-mint.log, air_server.log, aso-upgrade.log, aso-redeem.log.",
    ),
    s(
        "service.unit_dir",
        Text,
        "@unit_dir",
        Restart,
        "Where the service manager's files go: /etc/systemd/system (Linux), /Library/LaunchDaemons (macOS).",
    ),
    s(
        "service.log_max_size_mb",
        Whole(0, 10_240),
        "50",
        Live,
        "A log bigger than this many MB is moved aside to <log>.<epoch>.bak when its program starts, or aso opens it. 0: a log is never moved aside.",
    ),
    s(
        "service.log_keep",
        Whole(0, 52),
        "4",
        Live,
        "How many of those .bak files are kept for each log; older ones are removed. 0: every .bak file is kept.",
    ),
    s(
        "service.config_backups",
        Whole(0, 100),
        "5",
        Live,
        "How many aso.toml.<epoch>.bak copies are kept (one is made when an upgrade resets a pin). 0: every copy is kept.",
    ),
    s(
        "service.start_wait_secs",
        Whole(1, 3600),
        "60",
        Live,
        "How long start, restart and upgrade wait for a program to answer its health check before they call it failed.",
    ),
    s(
        "air.bin",
        Text,
        "air_server",
        Live,
        "The air_server binary `aso install` and `aso upgrade` copy into bin_dir (a path, or a name on PATH). air_server publishes no releases, so this is always the operator's file.",
    ),
    s(
        "air.version",
        Text,
        "",
        Live,
        "A note: the version of that binary, in your words (air_server prints none), any text. Not a pin: air_server publishes no releases, so nothing is chosen by it. Recorded with the file's SHA-256 when it is installed.",
    ),
    s(
        "air.name",
        Text,
        "",
        Restart,
        "The AIR server's name (its public key, 66 hex characters). Normally empty: `aso install` reads it from air_server's first start and remembers it.",
    ),
    s(
        "air.host",
        Text,
        "127.0.0.1",
        Restart,
        "The host fsp-mint dials to reach air_server (port 5702 is fixed on the client side). Written to fsp-mint as air.host.",
    ),
    s(
        "air.public_name",
        Text,
        "",
        Restart,
        "The public host name nginx answers on, and the host in this service's air.fsp entry. Empty: no nginx file is written.",
    ),
    s(
        "air.public_cert",
        Text,
        "",
        Restart,
        "nginx's TLS certificate file (absolute path). With public_key.",
    ),
    s(
        "air.public_key",
        Text,
        "",
        Restart,
        "nginx's TLS key file (absolute path).",
    ),
    s(
        "air.nginx_conf",
        Text,
        "@etc/nginx-air.conf",
        Restart,
        "Where the nginx server block is written. Point it into nginx's own folder (for example /etc/nginx/conf.d/aso-air.conf), or include it from there.",
    ),
    s(
        "air.ws_addr",
        Text,
        "127.0.0.1:5702",
        Restart,
        "AIR_WS_ADDR: where air_server listens for the plain WebSocket, and where nginx forwards to. air_server's own default is 0.0.0.0:5702; aso binds loopback because nginx is in front.",
    ),
    s(
        "air.health_addr",
        Text,
        "127.0.0.1:5701",
        Restart,
        "AIR_HEALTH_ADDR: air_server's health listener, which answers a bare `ok`.",
    ),
    s(
        "air.tls_addr",
        Text,
        "",
        Restart,
        "AIR_TLS_ADDR: air_server's own TLS listener. Empty: off (nginx terminates TLS). With tls_cert and tls_key.",
    ),
    s(
        "air.tls_cert",
        Text,
        "",
        Restart,
        "AIR_TLS_CERT (absolute path).",
    ),
    s(
        "air.tls_key",
        Text,
        "",
        Restart,
        "AIR_TLS_KEY (absolute path).",
    ),
    s(
        "air.identity",
        Text,
        "",
        Restart,
        "AIR_SECRET_FILE: an identity file that seeds a new STORAGE.db (absolute path). Empty: air_server makes one. Once STORAGE.db holds an identity this is ignored by air_server.",
    ),
    s(
        "air.storage",
        Text,
        "@data/air/STORAGE.db",
        Restart,
        "AIR_STORAGE: air_server's SQLite database, which holds the identity.",
    ),
    s(
        "air.fee_read",
        Whole(0, u64::MAX),
        "0",
        Restart,
        "AIR server fee per read. Only 0 is accepted: stock air_server 1.0.1 has no fee hook (upstream item 1).",
    ),
    s(
        "air.fee_write",
        Whole(0, u64::MAX),
        "0",
        Restart,
        "AIR server fee per write. Only 0 is accepted: stock air_server 1.0.1 has no fee hook (upstream item 1).",
    ),
    s(
        "fsp.bin",
        Text,
        "fsp-mint",
        Live,
        "The fsp-mint binary used until `aso install` has put a release in bin_dir (a path, or a name on PATH).",
    ),
    s(
        "fsp.version",
        Text,
        "",
        Live,
        "Pins fsp-mint to this release (x.y.z), 0.8.0 or later. Empty: the newest tagged release. Never a release candidate.",
    ),
    s(
        "wallet.bin",
        Text,
        "ecash-wallet",
        Live,
        "The ecash-wallet binary, as fsp.bin.",
    ),
    s(
        "wallet.version",
        Text,
        "",
        Live,
        "Pins ecash-wallet to this release (x.y.z), 0.8.0 or later. Empty: the newest tagged release.",
    ),
    s(
        "redeem.payout",
        Text,
        "",
        Live,
        "Where the ESP the service earns is paid out, for example btc:ADDRESS. Empty: no automatic redeem.",
    ),
    s(
        "redeem.interval_hours",
        Whole(1, 720),
        "24",
        Live,
        "How often the automatic redeem runs, in hours (the timer is written again by `aso reload`).",
    ),
    s(
        "release.url",
        Text,
        "https://api.github.com",
        Live,
        "The GitHub API address releases are listed from.",
    ),
    s(
        "release.keep",
        Whole(0, 20),
        "2",
        Live,
        "How many downloaded releases of each program are kept in the aso folder, the newest first; the installed one and the one before it always stay. 0: every release is kept.",
    ),
];

/// fsp-mint's own settings, as in `fsp-mint config` (fsp-mint 0.8.0
/// `config::SPECS`): `(key, default, reloads live)`. An empty default is "not
/// set". Written into `aso.toml` as `[fsp.<section>]`.
pub const FSP: &[(&str, &str, bool)] = &[
    ("server.network", "main", false),
    ("server.bind", "127.0.0.1:3200", false),
    ("server.max_request_bytes", "1048576", true),
    ("service.did", "", false),
    ("fees.issue_fee_per_byte", "0", true),
    ("rotation.activation_hours", "8760", false),
    ("rotation.period_hours", "8760", false),
    ("issue.max_items", "1000", true),
    ("spend.max_items", "1000", true),
    ("spend.max_memo_bytes", "256", true),
    ("spend.max_batch", "1024", true),
    ("esp.keyset_ttl_secs", "3600", true),
    ("air.esp_did", "", false),
    ("air.esp_server", "", false),
    ("air.esp_host", "", false),
    ("air.server", "", false),
    ("air.host", "", false),
    ("air.held_reads", "16", false),
    ("air.reply_wait_secs", "30", false),
    ("air.attempts", "3", false),
    ("air.max_skew_secs", "300", false),
    ("air.timeout_secs", "10", false),
    ("storage.mint_state", "", false),
];

/// ecash-wallet's settings (ecash-wallet 0.8.0 `config::SPECS`). The wallet reads
/// its file on every run, so every one is live.
pub const WALLET: &[(&str, &str, bool)] = &[
    ("storage.wallet", "", true),
    ("storage.app", "", true),
    ("air.psp_did", "", true),
    ("air.esp_did", "", true),
    ("air.server", "", true),
    ("air.host", "", true),
    ("air.fsp", "", true),
    ("air.reply_wait_secs", "30", true),
    ("air.attempts", "3", true),
    ("air.max_skew_secs", "300", true),
    ("air.timeout_secs", "10", true),
    ("psp.unpaid_key_days", "90", true),
];

/// The wallet settings that were removed, with why: `[wallet.<section>]` keys
/// (`psp.min_purchase`). A file that still sets one is refused with this text, not
/// passed through, so an old file is never silently ignored.
const WALLET_REMOVED: &[(&str, &str)] = &[(
    "psp.min_purchase",
    "buy-psp takes no amounts, so there is no purchase size to check. We recommend a PSP purchase of at least $5 (50 PSP): the PSP mint's fees come out of what a payment buys, so a smaller payment can buy little or nothing. Nothing enforces it",
)];

pub const NETWORKS: [&str; 4] = ["main", "regtest", "signet", "testnet"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Default,
    File,
    Env(String),
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Source::Default => write!(f, "default"),
            Source::File => write!(f, "aso.toml"),
            Source::Env(name) => write!(f, "env {name}"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub key: String,
    pub value: Value,
    pub source: Source,
    /// `None` for a key aso does not name: it is passed through.
    pub reload: Option<Reload>,
    pub what: String,
}

/// What `Aso::load` reads.
pub struct Inputs<'a> {
    /// The aso folder.
    pub dir: PathBuf,
    /// The text of `aso.toml`, if there is a file.
    pub file: Option<&'a str>,
    pub env: &'a dyn Fn(&str) -> Option<String>,
    pub home: String,
    /// `std::env::consts::OS`: `linux` or `macos`.
    pub os: &'a str,
}

#[derive(Clone, Debug)]
pub struct Aso {
    pub dir: PathBuf,
    pub mac: bool,
    pub home: String,
    pub entries: Vec<Entry>,
}

pub fn env_name(key: &str) -> String {
    format!("ASO_{}", key.to_uppercase().replace('.', "_"))
}

/// `x.y.z`, three numbers.
pub fn is_version(v: &str) -> bool {
    let parts: Vec<&str> = v.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

fn is_hex(v: &str, len: usize) -> bool {
    v.len() == len && v.chars().all(|c| c.is_ascii_hexdigit())
}

fn is_host(v: &str) -> bool {
    !v.is_empty()
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

fn value_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Integer(n) => n.to_string(),
        Value::Boolean(b) => b.to_string(),
        Value::Array(a) => a.iter().map(value_text).collect::<Vec<_>>().join(","),
        other => other.to_string(),
    }
}

/// `value` checked against `kind`.
fn typed(key: &str, kind: Kind, value: &Value) -> Result<Value, String> {
    match (kind, value) {
        (Raw, v) => Ok(v.clone()),
        (Text, Value::String(_)) => Ok(value.clone()),
        (Text, _) => Err(format!(
            "{key} must be text (in quotes), not {}",
            value.type_str()
        )),
        (Whole(min, max), Value::Integer(n)) => {
            let n = u64::try_from(*n).map_err(|_| format!("{key} = {n} must not be negative"))?;
            if (min..=max).contains(&n) {
                Ok(Value::Integer(n as i64))
            } else {
                Err(format!("{key} = {n} must be {min} to {max}"))
            }
        }
        (Whole(..), _) => Err(format!(
            "{key} must be a whole number, not {}",
            value.type_str()
        )),
        (List, Value::String(_)) => Ok(Value::Array(vec![value.clone()])),
        (List, Value::Array(a)) if a.iter().all(|v| v.is_str()) => Ok(value.clone()),
        (List, _) => Err(format!("{key} must be a list of strings")),
    }
}

fn from_env(key: &str, kind: Kind, text: &str) -> Result<Value, String> {
    let v = match kind {
        Text => Value::String(text.to_string()),
        Whole(..) => Value::Integer(
            text.trim()
                .parse::<i64>()
                .map_err(|_| format!("{} = `{text}` must be a whole number", env_name(key)))?,
        ),
        List => Value::Array(
            text.split(',')
                .map(|p| Value::String(p.trim().to_string()))
                .filter(|v| !v.as_str().unwrap_or("").is_empty())
                .collect(),
        ),
        Raw => Value::String(text.to_string()),
    };
    typed(key, kind, &v)
        .map_err(|e| format!("{e} (from the environment variable {})", env_name(key)))
}

impl Aso {
    /// Read the settings. `Err` lists every problem.
    pub fn load(i: &Inputs) -> Result<Aso, Vec<String>> {
        let mut problems = Vec::new();
        let table: Table = match i.file {
            None => Table::new(),
            Some(text) => match text.parse::<Table>() {
                Ok(t) => t,
                Err(e) => return Err(vec![format!("aso.toml is not valid TOML: {}", e.message())]),
            },
        };
        let mac = i.os == "macos";
        let mut entries: Vec<Entry> = Vec::new();
        let mut used: Vec<String> = Vec::new();
        let dir = i.dir.display().to_string();

        // aso's own settings.
        for st in SETTINGS {
            let (section, name) = st.key.split_once('.').unwrap_or((st.key, ""));
            let in_file = table
                .get(section)
                .and_then(|v| v.as_table())
                .and_then(|t| t.get(name));
            if in_file.is_some() {
                used.push(st.key.to_string());
            }
            let lookup = |key: &str| -> String {
                entries
                    .iter()
                    .find(|e| e.key == key)
                    .map(|e| value_text(&e.value))
                    .unwrap_or_default()
            };
            let default = default_text(st.default, &dir, mac, i.env, &lookup);
            let env_key = env_name(st.key);
            let env_value = (i.env)(&env_key).filter(|v| !v.is_empty());
            let (value, source) = if let Some(text) = env_value {
                match from_env(st.key, st.kind, &text) {
                    Ok(v) => (v, Source::Env(env_key)),
                    Err(e) => {
                        problems.push(e);
                        continue;
                    }
                }
            } else if let Some(v) = in_file {
                match typed(st.key, st.kind, v) {
                    Ok(v) => (v, Source::File),
                    Err(e) => {
                        problems.push(e);
                        continue;
                    }
                }
            } else {
                (default_value(st.kind, &default), Source::Default)
            };
            entries.push(Entry {
                key: st.key.to_string(),
                value,
                source,
                reload: Some(st.reload),
                what: st.what.to_string(),
            });
        }

        // Unknown settings in aso's own sections.
        for (section, body) in &table {
            let own = ["service", "air", "redeem", "release"].contains(&section.as_str());
            let comp = ["fsp", "wallet"].contains(&section.as_str());
            if !own && !comp {
                problems.push(format!(
                    "unknown section [{section}]: the sections are [service], [air], [fsp], [wallet], [redeem] and [release]"
                ));
                continue;
            }
            let Some(body) = body.as_table() else {
                problems.push(format!("`{section}` must be a [section] of settings"));
                continue;
            };
            for (name, value) in body {
                let key = format!("{section}.{name}");
                if own || !value.is_table() {
                    if !used.contains(&key) {
                        problems.push(format!("unknown setting `{key}`"));
                    }
                    continue;
                }
                // [fsp.<section>] / [wallet.<section>]: the component's own keys.
                let Some(inner) = value.as_table() else {
                    continue;
                };
                let (mirror, label) = if section == "fsp" {
                    (FSP, "fsp")
                } else {
                    (WALLET, "wallet")
                };
                for (k, v) in inner {
                    let comp_key = format!("{name}.{k}");
                    if v.is_table() {
                        problems.push(format!(
                            "{label}.{comp_key} is a table: the component's settings are `[{label}.{name}] {k} = value`, one level"
                        ));
                        continue;
                    }
                    if section == "wallet"
                        && let Some((_, why)) = WALLET_REMOVED.iter().find(|(k, _)| *k == comp_key)
                    {
                        problems.push(format!(
                            "wallet.{comp_key} was removed in aso 0.3.0: {why}. Remove it"
                        ));
                        continue;
                    }
                    let known = mirror.iter().find(|(mk, _, _)| *mk == comp_key);
                    entries.push(Entry {
                        key: format!("{label}.{comp_key}"),
                        value: v.clone(),
                        source: Source::File,
                        reload: known.map(|(_, _, live)| if *live { Live } else { Restart }),
                        what: if known.is_some() {
                            format!("{label} setting {comp_key}")
                        } else {
                            format!(
                                "not named by aso: passed to {} unchanged",
                                if section == "fsp" {
                                    "fsp-mint"
                                } else {
                                    "ecash-wallet"
                                }
                            )
                        },
                    });
                }
            }
        }

        // The component settings aso names but the file does not set.
        for (label, mirror) in [("fsp", FSP), ("wallet", WALLET)] {
            for (k, default, live) in mirror {
                let key = format!("{label}.{k}");
                if entries.iter().any(|e| e.key == key) {
                    continue;
                }
                entries.push(Entry {
                    key,
                    value: if default.chars().all(|c| c.is_ascii_digit()) && !default.is_empty() {
                        Value::Integer(default.parse().unwrap_or(0))
                    } else {
                        Value::String(default.to_string())
                    },
                    source: Source::Default,
                    reload: Some(if *live { Live } else { Restart }),
                    what: format!("{label} setting {k}"),
                });
            }
        }

        let aso = Aso {
            dir: i.dir.clone(),
            mac,
            home: i.home.clone(),
            entries,
        };
        if problems.is_empty() {
            aso.check(&mut problems);
        }
        if problems.is_empty() {
            Ok(aso)
        } else {
            Err(problems)
        }
    }

    pub fn entry(&self, key: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.key == key)
    }

    /// The text of a setting; empty when it is not set.
    pub fn text(&self, key: &str) -> String {
        self.entry(key)
            .map(|e| value_text(&e.value))
            .unwrap_or_default()
    }

    /// A whole-number setting (0 when it is not a number).
    pub fn whole(&self, key: &str) -> u64 {
        self.entry(key)
            .and_then(|e| e.value.as_integer())
            .and_then(|n| u64::try_from(n).ok())
            .unwrap_or(0)
    }

    /// `Some(text)` when the setting is set to something other than empty.
    pub fn opt(&self, key: &str) -> Option<String> {
        Some(self.text(key)).filter(|t| !t.is_empty())
    }

    pub fn path(&self, key: &str) -> PathBuf {
        PathBuf::from(self.text(key))
    }

    pub fn bin_dir(&self) -> PathBuf {
        self.path("service.bin_dir")
    }
    pub fn etc_dir(&self) -> PathBuf {
        self.path("service.etc_dir")
    }
    pub fn data_dir(&self) -> PathBuf {
        self.path("service.data_dir")
    }
    pub fn log_dir(&self) -> PathBuf {
        self.path("service.log_dir")
    }
    pub fn unit_dir(&self) -> PathBuf {
        self.path("service.unit_dir")
    }
    pub fn log_max_mb(&self) -> u64 {
        self.whole("service.log_max_size_mb")
    }
    pub fn log_keep(&self) -> u64 {
        self.whole("service.log_keep")
    }

    /// The Bitcoin network of the ESP mint this service takes ESP from.
    pub fn network(&self) -> String {
        self.text("fsp.server.network")
    }

    /// fsp-mint's `FSP_DATA_DIR`; its data is in `<this>/<network>`.
    pub fn fsp_base(&self) -> PathBuf {
        self.data_dir().join("fsp")
    }

    /// fsp-mint's data folder for the network.
    pub fn fsp_dir(&self) -> PathBuf {
        self.fsp_base().join(self.network())
    }

    /// The service's key file, written by `fsp-mint service-key`.
    pub fn service_key_file(&self) -> PathBuf {
        self.fsp_dir().join("service_key.hex")
    }

    pub fn wallet_file(&self) -> PathBuf {
        self.data_dir().join("wallet").join("wallet.json")
    }

    pub fn wallet_config_file(&self) -> PathBuf {
        self.etc_dir().join("ecash-wallet.toml")
    }

    pub fn fsp_config_file(&self) -> PathBuf {
        self.etc_dir().join("fsp-mint.toml")
    }

    /// `server.bind`: where fsp-mint's `/health`, `/audit` and `/export-esp` are.
    pub fn fsp_bind(&self) -> String {
        self.text("fsp.server.bind")
    }

    /// The component's settings that `aso.toml` or the environment set, as the
    /// nested table the component's file holds: `{ server = { bind = ".." } }`.
    pub fn component_table(&self, label: &str) -> Table {
        let prefix = format!("{label}.");
        let mut out = Table::new();
        for e in &self.entries {
            if e.source == Source::Default {
                continue;
            }
            let Some(rest) = e.key.strip_prefix(&prefix) else {
                continue;
            };
            let Some((section, name)) = rest.split_once('.') else {
                continue;
            };
            let entry = out
                .entry(section.to_string())
                .or_insert_with(|| Value::Table(Table::new()));
            if let Value::Table(t) = entry {
                t.insert(name.to_string(), e.value.clone());
            }
        }
        out
    }

    /// The settings of `aso.toml` whose value differs from `other`'s and that
    /// need a restart: `(key, old, new)`.
    pub fn restart_changes(&self, applied: &[(String, String)]) -> Vec<(String, String, String)> {
        let mut out = Vec::new();
        for e in &self.entries {
            if e.reload != Some(Restart) {
                continue;
            }
            let now = value_text(&e.value);
            if let Some((_, was)) = applied.iter().find(|(k, _)| *k == e.key)
                && *was != now
            {
                out.push((e.key.clone(), was.clone(), now));
            }
        }
        out
    }

    /// The current value of every setting that needs a restart, as `(key, text)`:
    /// what a start or restart applies.
    pub fn restart_values(&self) -> Vec<(String, String)> {
        self.entries
            .iter()
            .filter(|e| e.reload == Some(Restart))
            .map(|e| (e.key.clone(), value_text(&e.value)))
            .collect()
    }

    /// `aso config`: one line per setting, with where it came from and whether a
    /// change reloads live.
    pub fn describe(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for e in &self.entries {
            let reload = match e.reload {
                Some(r) => r.words(),
                None => "passed through: the component says if it reloads",
            };
            let shown = if e.value.as_str().is_some_and(str::is_empty) {
                "(not set)".to_string()
            } else {
                value_text(&e.value)
            };
            lines.push(format!("{} = {shown}  [{}; {reload}]", e.key, e.source));
        }
        lines
    }

    fn check(&self, p: &mut Vec<String>) {
        let abs = |p: &mut Vec<String>, key: &str| {
            let v = self.text(key);
            if !v.is_empty() {
                if !v.starts_with('/') {
                    p.push(format!("{key} is `{v}`: it must be an absolute path"));
                } else if let Err(e) = crate::template::plain(key, &v) {
                    p.push(e);
                }
            }
        };
        for key in [
            "service.bin_dir",
            "service.etc_dir",
            "service.data_dir",
            "service.log_dir",
            "service.unit_dir",
            "air.nginx_conf",
            "air.storage",
            "air.public_cert",
            "air.public_key",
            "air.tls_cert",
            "air.tls_key",
            "air.identity",
        ] {
            abs(p, key);
        }
        for key in ["service.user", "service.group"] {
            if let Err(e) = crate::template::plain(key, &self.text(key)) {
                p.push(e);
            }
        }
        let name = self.text("air.name");
        if !name.is_empty() && !is_hex(&name, 66) {
            p.push(format!(
                "air.name is `{name}`: the AIR server's name is its public key, 66 hex characters"
            ));
        }
        let host = self.text("air.host");
        if !is_host(&host) {
            p.push(format!(
                "air.host is `{host}`: a host name or IP with no scheme, path or port (fsp-mint always dials port 5702)"
            ));
        }
        let public = self.text("air.public_name");
        if !public.is_empty() && !is_host(&public) {
            p.push(format!(
                "air.public_name is `{public}`: a host name, with no scheme, path or port"
            ));
        }
        if !public.is_empty()
            && (self.text("air.public_cert").is_empty() || self.text("air.public_key").is_empty())
        {
            p.push("air.public_name needs air.public_cert and air.public_key: nginx answers WSS with them".into());
        }
        if public.is_empty()
            && (!self.text("air.public_cert").is_empty() || !self.text("air.public_key").is_empty())
        {
            p.push("air.public_cert and air.public_key are for air.public_name: set it".into());
        }
        for key in ["air.ws_addr", "air.health_addr"] {
            let v = self.text(key);
            if v.parse::<std::net::SocketAddr>().is_err() {
                p.push(format!("{key} is `{v}`: it must be an IP:port"));
            }
        }
        let tls = self.text("air.tls_addr");
        if !tls.is_empty() && tls.parse::<std::net::SocketAddr>().is_err() {
            p.push(format!("air.tls_addr is `{tls}`: it must be an IP:port"));
        }
        let (cert, key) = (self.text("air.tls_cert"), self.text("air.tls_key"));
        if cert.is_empty() != key.is_empty() {
            p.push("air.tls_cert and air.tls_key go together: air_server starts its TLS listener only with both".into());
        }
        if !tls.is_empty() && cert.is_empty() {
            p.push("air.tls_addr is set without air.tls_cert and air.tls_key: air_server would not start its TLS listener".into());
        }
        if tls.is_empty() && !cert.is_empty() {
            p.push("air.tls_cert and air.tls_key are set without air.tls_addr: air_server would listen on its own default, 0.0.0.0:443. Set air.tls_addr".into());
        }
        for key in ["air.fee_read", "air.fee_write"] {
            let n = self.whole(key);
            if n != 0 {
                p.push(format!(
                    "{key} = {n}: stock air_server 1.0.1 has no fee hook, so it takes no fee (upstream item 1, the air_server fee hook). Leave it 0 or remove it"
                ));
            }
        }
        // air.version is a note in the operator's words, not a pin: air_server
        // publishes no releases, so nothing is chosen by it.
        for program in [
            crate::program::Program::Fsp,
            crate::program::Program::Wallet,
        ] {
            let key = format!("{}.version", program.section());
            let v = self.text(&key);
            if v.is_empty() {
                continue;
            }
            if is_version(&v) {
                if let Some((min, why)) = program.min_version()
                    && air_minterface::version::Tags::Strict.is_newer(min, &v)
                {
                    p.push(format!(
                        "{key} is `{v}`: aso needs {} {min} or later (it uses `{why}`). Pin {min} or later, or remove the line",
                        program.name()
                    ));
                }
                continue;
            }
            let pre = v.contains('-') || v.to_lowercase().contains("rc");
            p.push(if pre {
                format!(
                    "{key} is `{v}`: a release candidate or pre-release is never installed. Pin a release, three numbers like 1.0.1, or remove the line"
                )
            } else {
                format!("{key} is `{v}`: a version is three numbers, like 1.0.1")
            });
        }
        let network = self.network();
        if !NETWORKS.contains(&network.as_str()) {
            p.push(format!(
                "fsp.server.network is `{network}`: main, regtest, signet or testnet"
            ));
        }
        let bind = self.fsp_bind();
        match bind.parse::<std::net::SocketAddr>() {
            Err(_) => p.push(format!("fsp.server.bind is `{bind}`: it must be an IP:port")),
            Ok(a) if !a.ip().is_loopback() => p.push(format!(
                "fsp.server.bind is `{bind}`: not a loopback address. Wallets reach this mint over AIR only; aso reaches its health and audit on this machine"
            )),
            Ok(_) => {}
        }
        let payout = self.text("redeem.payout");
        if payout.chars().any(|c| c.is_control()) {
            p.push("redeem.payout must be one line of text, for example btc:ADDRESS".into());
        }
        if !payout.is_empty() && payout.trim() != payout {
            p.push("redeem.payout has a space at its start or end".into());
        }
    }
}

fn default_text(
    marker: &str,
    dir: &str,
    mac: bool,
    env: &dyn Fn(&str) -> Option<String>,
    lookup: &dyn Fn(&str) -> String,
) -> String {
    match marker {
        "@user" => {
            if mac {
                env("SUDO_USER")
                    .or_else(|| env("USER"))
                    .filter(|u| !u.is_empty())
                    .unwrap_or_else(|| "root".into())
            } else {
                "aso".into()
            }
        }
        "@group" => {
            if !mac {
                "aso".into()
            } else if lookup("service.user") == "root" {
                "wheel".into()
            } else {
                "staff".into()
            }
        }
        "@unit_dir" => {
            if mac {
                "/Library/LaunchDaemons".into()
            } else {
                "/etc/systemd/system".into()
            }
        }
        m => {
            if let Some(rest) = m.strip_prefix("@dir/") {
                format!("{dir}/{rest}")
            } else if let Some(rest) = m.strip_prefix("@etc/") {
                format!("{}/{rest}", lookup("service.etc_dir"))
            } else if let Some(rest) = m.strip_prefix("@data/") {
                format!("{}/{rest}", lookup("service.data_dir"))
            } else {
                m.to_string()
            }
        }
    }
}

fn default_value(kind: Kind, text: &str) -> Value {
    match kind {
        Whole(..) => Value::Integer(text.parse().unwrap_or(0)),
        List => Value::Array(Vec::new()),
        _ => Value::String(text.to_string()),
    }
}
