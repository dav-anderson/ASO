//! The logs aso writes, or sends a child's output to, and their caps.
//!
//! **One rule for every log**: a log bigger than `[service] log_max_size_mb` is
//! put aside as `<name>.<epoch>.bak`, and only the newest `[service] log_keep`
//! of those `.bak` files are kept. The logs are:
//!
//! - `fsp-mint.log` and `air_server.log`, written by programs that are meant
//!   never to stop. Two things keep them under the cap:
//!   - the service units run `aso logs rotate NAME` before the program starts,
//!     on every start, including a start by the service manager after a crash
//!     (see [`crate::service`]). Nothing has the file open yet, so it is moved
//!     aside ([`rotate`]);
//!   - while they run, the logs job runs `aso logs rotate --running NAME...`
//!     every [`crate::service::LOGS_EVERY_MINUTES`] minutes. The program holds
//!     the file open for appending, so it is copied aside and emptied in place
//!     ([`rotate_running`]); the program's next line goes to the start of the
//!     emptied file. A log can pass the cap by what is written between two
//!     runs, and a line written in the moment between the copy and the emptying
//!     is lost.
//! - `aso-upgrade.log` (the scheduled upgrade) and `aso-redeem.log` (the
//!   automatic redeem): aso opens them itself with [`Log::open`], which rotates
//!   first, and each run is short and ends, so no run keeps one open for long.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// `[service] log_max_size_mb`: 50, as in spi.
pub const DEFAULT_MAX_MB: u64 = 50;
/// `[service] log_keep`: 4, as in spi.
pub const DEFAULT_KEEP: u64 = 4;

/// The `.bak` files of `log`, newest first: `(epoch, path)`.
fn backups(log: &Path) -> Vec<(u64, PathBuf)> {
    let (Some(dir), Some(name)) = (log.parent(), log.file_name()) else {
        return Vec::new();
    };
    let name = name.to_string_lossy().to_string();
    let dir = if dir.as_os_str().is_empty() {
        Path::new(".")
    } else {
        dir
    };
    let mut found: Vec<(u64, PathBuf)> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let file = e.file_name().to_string_lossy().to_string();
            let epoch = file
                .strip_prefix(name.as_str())?
                .strip_prefix('.')?
                .strip_suffix(".bak")?;
            Some((epoch.parse::<u64>().ok()?, e.path()))
        })
        .collect();
    found.sort_by_key(|f| std::cmp::Reverse(f.0));
    found
}

/// Remove all but the newest `keep` `.bak` files of `log`. Returns how many were
/// removed.
pub fn prune(log: &Path, keep: u64) -> Result<usize, String> {
    let mut removed = 0;
    for (_, path) in backups(log).into_iter().skip(keep as usize) {
        fs::remove_file(&path).map_err(|e| format!("cannot remove {}: {e}", path.display()))?;
        removed += 1;
    }
    Ok(removed)
}

/// Move `log` aside to `<log>.<epoch>.bak` if it is bigger than `max_mb`
/// megabytes, then prune to the newest `keep`. Returns the `.bak` file made, if
/// any. A log that is not there, or is not over the cap, is left alone (but the
/// `.bak` files are still pruned, so a lowered `log_keep` takes effect).
pub fn rotate(log: &Path, max_mb: u64, keep: u64, now: u64) -> Result<Option<PathBuf>, String> {
    let over = fs::metadata(log).is_ok_and(|m| m.is_file() && m.len() > max_mb * 1024 * 1024);
    let made = if over {
        let mut epoch = now;
        let target = loop {
            let t = PathBuf::from(format!("{}.{epoch}.bak", log.display()));
            if !t.exists() {
                break t;
            }
            epoch += 1;
        };
        fs::rename(log, &target).map_err(|e| {
            format!(
                "cannot move {} aside to {}: {e}",
                log.display(),
                target.display()
            )
        })?;
        Some(target)
    } else {
        None
    };
    prune(log, keep)?;
    Ok(made)
}

/// For a log a running program holds open for appending: if `log` is bigger
/// than `max_mb` megabytes, copy it to `<log>.<epoch>.bak` (owner-only) and empty
/// it in place, then prune to the newest `keep`. A rename would leave the
/// program writing to the moved file, past any cap. Returns the `.bak` file
/// made, if any. A log that is not there, or is not over the cap, is left alone
/// (the `.bak` files are still pruned).
pub fn rotate_running(
    log: &Path,
    max_mb: u64,
    keep: u64,
    now: u64,
) -> Result<Option<PathBuf>, String> {
    let over = fs::metadata(log).is_ok_and(|m| m.is_file() && m.len() > max_mb * 1024 * 1024);
    let made = if over {
        let mut epoch = now;
        let (target, mut copy) = loop {
            let t = PathBuf::from(format!("{}.{epoch}.bak", log.display()));
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(&t) {
                Ok(f) => break (t, f),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => epoch += 1,
                Err(e) => return Err(format!("cannot make {}: {e}", t.display())),
            }
        };
        let mut source =
            File::open(log).map_err(|e| format!("cannot read {}: {e}", log.display()))?;
        std::io::copy(&mut source, &mut copy)
            .and_then(|_| copy.sync_all())
            .map_err(|e| format!("cannot copy {} to {}: {e}", log.display(), target.display()))?;
        OpenOptions::new()
            .write(true)
            .open(log)
            .and_then(|f| f.set_len(0))
            .map_err(|e| format!("cannot empty {}: {e}", log.display()))?;
        Some(target)
    } else {
        None
    };
    prune(log, keep)?;
    Ok(made)
}

/// The seconds since the Unix epoch.
pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A log aso writes itself: rotated when it is opened, then appended to. Each
/// line is also printed, so a run at a terminal shows it.
pub struct Log {
    file: File,
    echo: bool,
}

impl Log {
    /// Rotate `path` if it is over the cap, then open it for appending (mode
    /// 0600 when it is made). `echo` also prints each line.
    pub fn open(path: &Path, max_mb: u64, keep: u64, now: u64, echo: bool) -> Result<Log, String> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        }
        rotate(path, max_mb, keep, now)?;
        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(path)
            .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
        Ok(Log { file, echo })
    }

    /// Write one line, with the time (`epoch`, Unix seconds).
    pub fn line(&mut self, epoch: u64, text: &str) {
        let line = format!("{epoch} {text}");
        // A log that cannot be written must not stop the work it reports on.
        if writeln!(self.file, "{line}").is_err() {
            eprintln!("aso: cannot write the log: {line}");
        }
        if self.echo {
            println!("{text}");
        }
    }
}
