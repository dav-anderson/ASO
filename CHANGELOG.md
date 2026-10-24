# Changelog

## [0.2.1] - 2026-10-24

The shared crate is renamed air-minterface (it was air-interface): aso now
depends on `air-minterface = { git = "https://github.com/dav-anderson/air-minterface.git", tag = "v0.2.0" }`,
the code imports `air_minterface`, and the line `aso versions` shows is named
`air-minterface`. No other change.

## [0.2.0] - 2026-10-24

ROADMAP item 32. The code aso shared with spi is the new crate
[air-interface](https://github.com/dav-anderson/air-interface)
(`air-interface = { git = ..., tag = "v0.1.0" }`).

- The release source and its checksum check, the pin and `upgrade check` code,
  the upgrade engine (swap, restart, wait, rollback), the schedule, the log
  rotation (`Log`, `logs::rotate`, `rotate_running`), the template expander, the
  file helpers and the AIR server's environment and nginx block are now
  air-interface's. aso implements its two traits (`Operator` on `Aso`, `Managed`
  on `Program`, in `src/interface.rs`) and keeps the commands, the prompts, its
  wording (every upgrade message is as before), `httpc`, the units and its
  settings. `src/exec.rs`, `template.rs`, `perm.rs`, `logs.rs`, `fetch.rs`,
  `release.rs`, `schedule.rs` and `upgrade.rs` are gone from aso (`ops/nginx-air.header`
  is new); `sha2` moved to air-interface. The modules are still reachable as
  `aso::exec`, `aso::logs`, `aso::schedule` and `aso::template`.
- `aso versions` shows an `air-interface` line.
- The AIR server's health check (status 200 and the body `ok`, as aso always
  required) is air-interface's, and spi now uses the same one.
- Docs: aso is designed to run on a Linux system. macOS is supported only as a
  development machine (README).
- Tests: 84 before, 76 after. 8 moved to air-interface (log rotation, the
  newest-backups rule, same-second rotations, the log a program opens, the
  running-log copy, release tags, the pin line and the schedule times, which are
  merged into the schedule tests there). The `aso logs rotate` command, the
  units, the job and the install tests stay.

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
