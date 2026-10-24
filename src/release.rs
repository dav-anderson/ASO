//! Releases: which versions of fsp-mint and ecash-wallet exist upstream, which is
//! the newest, and how a build is obtained and checked. `aso upgrade check`
//! reads it; `aso upgrade` and `aso install` use it to pick the target and to get
//! the build.
//!
//! **The source is a trait.** [`Source`] answers two questions: which version
//! tags a program has published, and, for one version, where its release binary
//! is (downloaded into a folder aso gives it) and what SHA-256 the release
//! publishes for it. aso does the rest itself: it ignores pre-release and
//! non-version tags, picks the newest, compares the downloaded file's SHA-256
//! with the published one and refuses a build that differs. The real source is
//! [`crate::fetch::CurlSource`] (GitHub releases, through `curl`). [`NoSource`]
//! knows nothing and is for tests that need no upstream.
//!
//! **Versions.** A tag is `x.y.z`, with or without a leading `v`. A tag with
//! anything else in it (`0.7.0-rc1`) is a pre-release or not a version, and is
//! ignored: a release candidate is never installed.
//!
//! **air_server** publishes no releases (upstream item 16). Its binary is the
//! operator's (`air.bin`), and its version is the operator's word for it
//! (`air.version`), recorded with the file's SHA-256 when it is installed.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::config::Aso;
use crate::exec::Exec;
use crate::program::Program;
use crate::versions;

/// A release binary that was downloaded, and the SHA-256 its release publishes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Download {
    pub file: PathBuf,
    pub sha256: String,
}

/// Where releases come from.
pub trait Source {
    /// The version tags `program` has published, as they are tagged (`v0.7.0`).
    /// `Err` with the reason when they cannot be listed.
    fn tags(&self, program: Program) -> Result<Vec<String>, String>;

    /// Download the release binary of `program` at `version` for this machine
    /// into the folder `dir` (made if need be). `Err` with the reason when there
    /// is none or it cannot be downloaded.
    fn fetch(&self, program: Program, version: &str, dir: &Path) -> Result<Download, String>;
}

/// No release source: nothing is known upstream.
pub struct NoSource;

impl Source for NoSource {
    fn tags(&self, _program: Program) -> Result<Vec<String>, String> {
        Err("no release source is configured in this aso".into())
    }

    fn fetch(&self, _program: Program, _version: &str, _dir: &Path) -> Result<Download, String> {
        Err("no release source is configured in this aso".into())
    }
}

/// The numbers of a release tag: `x.y.z`, digits only, with an optional leading
/// `v`. `None` for anything else.
fn numbers(tag: &str) -> Option<[u64; 3]> {
    let text = tag.strip_prefix('v').unwrap_or(tag);
    let mut out = Vec::new();
    for part in text.split('.') {
        if part.is_empty() || !part.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        out.push(part.parse::<u64>().ok()?);
    }
    if out.len() != 3 {
        return None;
    }
    Some([out[0], out[1], out[2]])
}

/// Whether `a` is a higher version than `b`. A tag that is not a version is
/// never newer than anything.
pub fn is_newer(a: &str, b: &str) -> bool {
    match (numbers(a), numbers(b)) {
        (Some(x), Some(y)) => x > y,
        _ => false,
    }
}

/// The newest release among `tags`, without its `v`. Pre-release and
/// non-version tags are ignored.
pub fn newest(tags: &[String]) -> Option<String> {
    tags.iter()
        .filter_map(|t| numbers(t).map(|n| (n, t.strip_prefix('v').unwrap_or(t).to_string())))
        .max_by_key(|(n, _)| *n)
        .map(|(_, t)| t)
}

/// The newest release `source` knows for `program`. `Ok(None)` when it lists
/// none that is a release; `Err` when it cannot be asked.
pub fn newest_of(source: &dyn Source, program: Program) -> Result<Option<String>, String> {
    Ok(newest(&source.tags(program)?))
}

/// Where a downloaded release is kept.
pub fn releases_dir(aso: &Aso, program: Program, version: &str) -> PathBuf {
    aso.dir.join("releases").join(program.name()).join(version)
}

/// Remove old downloads of `program` from `releases/`: the `release.keep` newest
/// versions stay, and so do the versions in `protect` (the installed one and the
/// one it replaced). A folder whose name is not a version is left alone. Returns
/// the versions removed.
pub fn prune(aso: &Aso, program: Program, protect: &[&str]) -> Result<Vec<String>, String> {
    let root = aso.dir.join("releases").join(program.name());
    let Ok(entries) = fs::read_dir(&root) else {
        return Ok(Vec::new());
    };
    let mut found: Vec<([u64; 3], String)> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            numbers(&name).map(|n| (n, name))
        })
        .collect();
    found.sort_by_key(|f| std::cmp::Reverse(f.0));
    let keep = aso.whole("release.keep") as usize;
    let mut gone = Vec::new();
    for (i, (_, name)) in found.iter().enumerate() {
        if i < keep || protect.iter().any(|p| versions::same_version(p, name)) {
            continue;
        }
        let path = root.join(name);
        fs::remove_dir_all(&path).map_err(|e| format!("cannot remove {}: {e}", path.display()))?;
        gone.push(name.clone());
    }
    Ok(gone)
}

/// Mark a downloaded file executable (mode 0755).
pub fn make_executable(path: &Path) -> Result<(), String> {
    crate::perm::set_mode(path, 0o755)
}

/// One line of `aso upgrade check`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CheckLine {
    pub name: String,
    /// The version of the installed program, or `unknown`.
    pub installed: String,
    /// The `version` in aso.toml, or `none`.
    pub aso_toml: String,
    /// The newest tagged release, `unknown`, or `none` when upstream publishes
    /// no releases.
    pub newest: String,
    /// `up_to_date`, `upgrade_available`, `pinned`, `no_releases` or `unknown`.
    pub status: String,
    /// The same in words, with the reason when it is unknown.
    pub detail: String,
}

/// The installed program: the service install's copy, as aso recorded it.
pub fn installed_version(aso: &Aso, exec: &dyn Exec, program: Program) -> Result<String, String> {
    let path = program.installed_path(aso);
    if !path.is_file() {
        return Err(format!("{} does not exist", path.display()));
    }
    versions::reported(aso, exec, program, &path)
}

/// `aso upgrade check`: one line per program. Reads and runs `--version` only.
pub fn check(aso: &Aso, exec: &dyn Exec, source: &dyn Source) -> Vec<CheckLine> {
    Program::ALL
        .into_iter()
        .map(|program| check_one(aso, exec, source, program))
        .collect()
}

fn check_one(aso: &Aso, exec: &dyn Exec, source: &dyn Source, program: Program) -> CheckLine {
    let installed = installed_version(aso, exec, program);
    let pin = program.pin(aso);
    let line = |newest: &str, status: &str, detail: String| CheckLine {
        name: program.name().to_string(),
        installed: installed.clone().unwrap_or_else(|_| "unknown".into()),
        aso_toml: pin.clone().unwrap_or_else(|| "none".into()),
        newest: newest.to_string(),
        status: status.to_string(),
        detail,
    };
    if !program.publishes() {
        return line(
            "none",
            "no_releases",
            "no releases published upstream: the binary is yours (air.bin)".into(),
        );
    }
    let newest = match newest_of(source, program) {
        Ok(n) => n,
        Err(why) => return line("unknown", "unknown", format!("unknown ({why})")),
    };
    let Some(newest) = newest else {
        return line(
            "unknown",
            "unknown",
            "unknown (the release source lists no release for it)".into(),
        );
    };
    let installed = match &installed {
        Ok(v) => v.clone(),
        Err(why) => {
            return line(
                &newest,
                "unknown",
                format!("unknown (the installed version cannot be read: {why})"),
            );
        }
    };
    let target = pin.clone().unwrap_or_else(|| newest.clone());
    if versions::same_version(&installed, &target) {
        match &pin {
            Some(p) if is_newer(&newest, p) => line(
                &newest,
                "pinned",
                format!("pinned at {p} (newest {newest})"),
            ),
            _ => line(&newest, "up_to_date", "up to date".into()),
        }
    } else {
        line(
            &newest,
            "upgrade_available",
            format!("upgrade available ({target})"),
        )
    }
}

/// The last line of `aso upgrade check`.
pub fn summary(lines: &[CheckLine]) -> String {
    let count = |status: &str| lines.iter().filter(|l| l.status == status).count();
    format!(
        "summary: {} up to date, {} upgrade available, {} pinned, {} unknown, {} without upstream releases",
        count("up_to_date"),
        count("upgrade_available"),
        count("pinned"),
        count("unknown"),
        count("no_releases")
    )
}

/// Text for a person, one line per program, then the summary.
pub fn render(lines: &[CheckLine]) -> String {
    let mut out = String::new();
    for l in lines {
        out.push_str(&format!(
            "{:<13} installed {:<9} aso.toml {:<9} newest {:<9} {}\n",
            l.name, l.installed, l.aso_toml, l.newest, l.detail
        ));
    }
    out.push_str(&summary(lines));
    out.push('\n');
    out
}

/// A program pinned to a version that is not known to be the newest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pinned {
    pub program: Program,
    pub pin: String,
    /// The newest release; `None` when the source could not say.
    pub newest: Option<String>,
}

/// The pinned programs among `programs` whose pin is older than the newest
/// release, or whose newest release cannot be found. A program upstream
/// publishes nothing for (air_server) has no pin in this sense: its `version` is
/// the operator's word for the binary.
pub fn pinned(aso: &Aso, source: &dyn Source, programs: &[Program]) -> Vec<Pinned> {
    programs
        .iter()
        .filter(|p| p.publishes())
        .filter_map(|&program| {
            let pin = program.pin(aso)?;
            let newest = newest_of(source, program).ok().flatten();
            let held = newest.as_deref().is_none_or(|n| is_newer(n, &pin));
            held.then_some(Pinned {
                program,
                pin,
                newest,
            })
        })
        .collect()
}

/// The warning that ends `aso upgrade check` and `aso upgrade status` and opens
/// `aso upgrade`: every pinned program, its pin and the newest release. Empty
/// when nothing is pinned behind the newest.
pub fn pin_warning(held: &[Pinned]) -> Vec<String> {
    if held.is_empty() {
        return Vec::new();
    }
    let mut lines =
        vec!["WARNING: these programs are pinned and are not at the newest release:".to_string()];
    for h in held {
        lines.push(format!(
            "  {}: pinned at {} (newest {})",
            h.program.name(),
            h.pin,
            h.newest.as_deref().unwrap_or("unknown")
        ));
    }
    lines.push(
        "`aso upgrade` installs the pin unless it is reset. It asks about each pin (--keep-pins keeps them all, --reset-pins resets them all); a reset removes the `version` line from aso.toml (the file is backed up first).".to_string(),
    );
    lines
}

/// `text` (an aso.toml) without the `version` line of `[section]`, the rest as
/// it is, comments and all. `None` when that section has no such line.
pub fn without_pin_line(text: &str, section: &str) -> Option<String> {
    let mut current = String::new();
    let mut out = String::with_capacity(text.len());
    let mut removed = false;
    for line in text.split_inclusive('\n') {
        let t = line.trim();
        if t.starts_with('[') && !t.starts_with("[[") {
            current = t
                .trim_start_matches('[')
                .split(']')
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
        } else if !removed
            && current == section
            && t.strip_prefix("version")
                .is_some_and(|rest| rest.trim_start().starts_with('='))
        {
            removed = true;
            continue;
        }
        out.push_str(line);
    }
    removed.then_some(out)
}

/// Keep the newest `keep` `<name>.<epoch>.bak` files next to `config`.
pub(crate) fn prune_backups(config: &Path, name: &str, keep: usize) {
    let dir = config.parent().filter(|d| !d.as_os_str().is_empty());
    let mut found: Vec<(i64, PathBuf)> = fs::read_dir(dir.unwrap_or(Path::new(".")))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let file = e.file_name().to_string_lossy().to_string();
            let epoch = file
                .strip_prefix(name)?
                .strip_prefix('.')?
                .strip_suffix(".bak")?;
            Some((epoch.parse::<i64>().ok()?, e.path()))
        })
        .collect();
    found.sort_by_key(|f| std::cmp::Reverse(f.0));
    for (_, path) in found.into_iter().skip(keep) {
        let _ = fs::remove_file(path);
    }
}

/// Remove the pins of `programs` from the settings file `config`, after copying
/// it to `<config>.<epoch>.bak`; the newest `keep` backups stay. Returns what was
/// done, a line per program.
pub fn reset_pins(
    config: &Path,
    programs: &[Program],
    epoch: i64,
    keep: usize,
) -> Result<Vec<String>, String> {
    let text =
        fs::read_to_string(config).map_err(|e| format!("cannot read {}: {e}", config.display()))?;
    let mut edited = text.clone();
    let mut lines = Vec::new();
    let mut missing = Vec::new();
    for &program in programs {
        match without_pin_line(&edited, program.section()) {
            Some(next) => edited = next,
            None => missing.push(program.name()),
        }
    }
    if edited != text {
        let backup = crate::edit::backup_and_replace(config, &edited, epoch, keep)?;
        lines.push(format!(
            "reset: removed the `version` pin of {} from {} (the old file is {})",
            programs
                .iter()
                .map(|p| p.name())
                .filter(|n| !missing.contains(n))
                .collect::<Vec<_>>()
                .join(", "),
            config.display(),
            backup.display()
        ));
    }
    for m in missing {
        lines.push(format!(
            "WARNING: {m}: there is no `version` line under [{}] in {} to remove (a pin set in the environment is yours to unset); the newest release is still installed this time",
            Program::parse(m).map(|p| p.section()).unwrap_or("?"),
            config.display()
        ));
    }
    Ok(lines)
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
