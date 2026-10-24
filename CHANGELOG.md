# Changelog

## [0.1.0] - 2026-10-23

First version.

- `aso install` (guided: air_server behind nginx, fsp-mint, a funded wallet, optional
  FSP at another service for backups; re-running checks what exists and builds
  only what is missing), `start`, `stop` (decommission), `restart`, `reload`,
  `status`, `audit`, `versions`, `upgrade` (check, status, cancel, `--at`, `--in`,
  pins), `config`, `redeem run`, `logs rotate`.
- One `aso.toml`; `aso config` shows every setting and its source; keys aso does
  not name pass through to fsp-mint and ecash-wallet.
- AIR server fee settings, 0 only (refused otherwise, naming upstream item 1).
- Release candidates and pre-releases are never installed; a pin on one is
  refused when `aso.toml` loads.
- Log caps (`service.log_max_size_mb`, `service.log_keep`) on every log aso
  writes or sends a program's output to; `service.config_backups`, `release.keep`.
- Automatic ESP redeem: export-esp, receive, redeem, collect, resume. The
  export is `fsp-mint export-esp --serve-only`: the job runs as root and never
  exports by itself, so it cannot leave a root-owned file in fsp-mint's data
  folder. It is skipped when the installed fsp-mint is older than 0.8.0 or aso
  has no record of its version. The redeem job is loaded by whichever step
  writes its files, so a payout set in `aso.toml` before `aso install` turns it on.
- Needs fsp-mint 0.8.0 (`export-esp --serve-only`) and ecash-wallet 0.7.0
  (`redeem --service-key-file`) or later: an `fsp.version` or `wallet.version`
  pin older than that is refused when `aso.toml` loads.
- The logs job (`aso-logs`, a systemd timer or a launchd job) runs `aso logs
  rotate --running` every 5 minutes as the service account, so fsp-mint.log
  and air_server.log stay within `service.log_max_size_mb` while they run: a log
  over the cap is copied aside and emptied in place.
- `air.version` is a note in your words (air_server prints no version), any
  text; it is not checked as a pin.
