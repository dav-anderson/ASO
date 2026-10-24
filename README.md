# aso

Runs one Service Point's own services from one command and one settings file:
**air_server** (the AIR server, behind nginx), **fsp-mint** (the service's FSP
mint) and **ecash-wallet** (the service's wallet). It installs them, writes their
settings from `aso.toml`, starts them in order, shows whether they are well,
upgrades them, and redeems the ESP the service earns, with no operator command.

aso is a separate program that **runs** the three; it links none of the family's
crates (no sp-wire, no mint-core), so it has no pins to keep in step. It talks
to the programs by their command lines and to their health listeners on this
machine over loopback HTTP. It builds nothing from source: it installs release
binaries (and air_server, which publishes none, from the file you give it).

**aso is designed to run on a Linux system** (systemd). macOS is supported only as
a development machine: the launchd files and the macOS settings exist so that aso
can be tried and tested on a Mac, not to run a production service. Run the
commands that touch the services as root (`sudo aso ...`): they write the service
manager's files.

The code aso shares with `spi` (downloading releases and checking their SHA-256,
the upgrade with its rollback, scheduling, log rotation, service-file templates)
is the [air-minterface](https://github.com/dav-anderson/air-minterface) crate, used
by git tag; `aso versions` shows its version.

## Commands

`aso --help` lists them. Each prints what it did; a command that fails exits
non-zero.

| Command | What it does |
|---|---|
| `aso install` | The guided install, four steps. See below. Run it again at any time. |
| `aso start` | Start air_server, then fsp-mint, each until its health answers ok. |
| `aso stop` | Take the whole service down, fsp-mint first. **Decommissioning only**; it asks first (`--yes` answers for you). |
| `aso restart [air_server\|fsp-mint]` | Stop and start. It says the downtime and asks first. |
| `aso reload` | Write every generated file again from `aso.toml`, make fsp-mint read its settings (SIGHUP), put the redeem job right, and name the settings that wait for a restart. **Stops nothing.** |
| `aso status` | Each install step (done or what is missing), each process (running, health, version), the redeem job, and **ready** or why not. Exit 1 when not ready. |
| `aso audit` | `fsp-mint audit` and the wallet's `balance`. Stops nothing. Exit 1 when the mint audit is not consistent. |
| `aso versions` | The version of each program and library, or `unknown` and why. |
| `aso upgrade` | See "Upgrading". |
| `aso config` | Every setting, its value, where the value came from (default, `aso.toml`, or an `ASO_...` variable) and whether a change reloads live. |
| `aso redeem run` | One run of the automatic redeem. The timer runs this; you can too. |
| `aso logs rotate [--running] FILE...` | Move a log over its cap aside. The service units run it before a program starts; with `--running` (the logs job runs it every 5 minutes) the log is copied aside and emptied in place, for a program that has it open. No `aso.toml` needed. |

Options before the command: `--dir DIR` (the aso folder, default `~/.aso`,
or `$ASO_DIR`), `--config FILE` (default `DIR/aso.toml`, or `$ASO_CONFIG`).
Only `stop`, `restart`, `upgrade` (and its scheduled run) and `install` can stop
a service. Nothing else issues a stop, a disable or a restart of air_server or
fsp-mint; the tests check the command lists.

## The install

`aso install` has four steps and a smart rescan. Every run starts by checking
what exists, says what is missing, and builds only that. A finished step is not
done again; `aso status` shows each.

* **a. AIR service.** The air_server binary you give (`air.bin`; air_server 1.0.1
  publishes no release) is installed, its settings and service definition are
  written, and when `air.public_name` is set the nginx file (the public name,
  your certificate, the forward to air_server). The file is checked with
  `nginx -t`; **aso does not reload nginx** (it is shared with whatever else the
  machine serves). air_server is started and its health (a bare `ok`) awaited,
  and its name is read from its log.
* **b. FSP.** fsp-mint (newest release, or the pin) is installed, `fsp-mint
  service-key` makes the service's DID key, fsp-mint is started after
  air_server is healthy. It needs the three ESP-mint settings
  `fsp.air.esp_did`, `fsp.air.esp_server`, `fsp.air.esp_host` in `aso.toml`.
  The install prints the `air.fsp` entry to give wallets.
* **c. Wallet funded.** ecash-wallet is installed and its settings written. The
  service is **ready only when the wallet is funded** (PSP above zero). Fund it
  with `aso install --buy-psp MINT_KEY AMOUNT...` (prints the payload to pay,
  once), then `aso install --claim-psp`; or `aso install --handoff FILE
  --did-key-file KEYFILE`.
* **d. Backups (optional).** `aso install --backup-service DID --backup-psp N`
  exchanges PSP for ESP at another service and issues it as FSP there, so you
  can pay that service for backups. `--skip-backup` skips it and remembers.

**Back up the DID key yourself, offline.** `fsp-mint service-key` makes
`service_key.hex` in the data folder. Whoever holds it can redeem the ESP the
service earns, and if it is lost the service is lost. aso says where it is
**before the service starts** and again when it makes it. aso makes no copy of
it: copy it to paper or to a drive that is not on this machine.

Without an `aso.toml`, `aso install` writes a starter (every line a comment).
The install also asks, when `redeem.payout` is unset, where the ESP should be
paid out, and writes the answer to `aso.toml` (the old file is kept as
`aso.toml.<epoch>.bak`).

## One settings file

Everything is in `aso.toml`. `aso config` prints each setting and where it came
from. **A key aso does not name passes through**: put it under `[fsp.<section>]`
or `[wallet.<section>]` and it reaches fsp-mint's or ecash-wallet's own file as
written, so a setting a new fsp-mint adds works without a new aso. Defaults are
the components' own: a setting you do not set is not written, and the component
uses its default (the tables below show them). A setting aso names is checked
when the file loads, and **every problem is listed with nothing changed**.
aso's own settings can also be set as `ASO_<SECTION>_<KEY>` (for example
`ASO_SERVICE_LOG_KEEP=6`); a variable beats the file.

Refused when the file loads: an `air.fee_read` or `air.fee_write` other than 0
(stock air_server has no fee hook; see "Needs from upstream"); a `version`
pin on a release candidate or pre-release (`0.9.0-rc1`), which is never
installed; an `fsp.version` older than 0.8.0 or a `wallet.version` older than
0.7.0 (aso uses fsp-mint's `export-esp --serve-only` and ecash-wallet's
`redeem --service-key-file`); a path that is not absolute; an fsp-mint bind that is not loopback.

`aso reload` applies what can change while the service runs. A setting marked
**needs a restart** is only named by `aso reload` as waiting; `aso restart`
applies it, with a brief downtime.

### aso's own settings

| Setting | Default | Reload | What it does |
|---|---|---|---|
| `service.user` | the account `aso` (Linux), the account that ran `sudo aso install` (macOS) | needs a restart | The account the services run as. Linux: `aso` (made by `aso install`). macOS: the account that runs `sudo aso install`. |
| `service.group` | `aso` (Linux), `staff` (macOS) | needs a restart | The group of that account. |
| `service.bin_dir` | `<aso folder>/bin` | needs a restart | Where the installed programs are (fsp-mint, air_server, ecash-wallet); the services run these copies. |
| `service.etc_dir` | `<aso folder>/etc` | needs a restart | Where aso writes the files it generates: fsp-mint.toml, the env files, the wallet's config, the nginx file. |
| `service.data_dir` | `<aso folder>/data` | needs a restart | Where the data is: fsp/ (fsp-mint), air/ (air_server's STORAGE.db), wallet/ (ecash-wallet's file), redeem/ (hand-over files, owner-only, deleted once received). |
| `service.log_dir` | `<aso folder>/logs` | needs a restart | Where the logs are: fsp-mint.log, air_server.log, aso-upgrade.log, aso-redeem.log. |
| `service.unit_dir` | `/etc/systemd/system` (Linux), `/Library/LaunchDaemons` (macOS) | needs a restart | Where the service manager's files go: /etc/systemd/system (Linux), /Library/LaunchDaemons (macOS). |
| `service.log_max_size_mb` | `50` | reloads live | A log bigger than this many MB is moved aside to <log>.<epoch>.bak when its program starts, or aso opens it; fsp-mint.log and air_server.log are also checked every 5 minutes while they run (see "Logs and files that grow"). |
| `service.log_keep` | `4` | reloads live | How many of those .bak files are kept for each log; older ones are removed. |
| `service.config_backups` | `5` | reloads live | How many aso.toml.<epoch>.bak copies are kept (one is made when an upgrade resets a pin). |
| `service.start_wait_secs` | `60` | reloads live | How long start, restart and upgrade wait for a program to answer its health check before they call it failed. |
| `air.bin` | `air_server` | reloads live | The air_server binary `aso install` and `aso upgrade` copy into bin_dir (a path, or a name on PATH). air_server publishes no releases, so this is always the operator's file. |
| `air.version` | (not set) | reloads live | A note: the version of that binary, in your words (air_server prints none), any text. Not a pin, and not checked. Recorded with the file's SHA-256 when it is installed. |
| `air.name` | (not set) | needs a restart | The AIR server's name (its public key, 66 hex characters). Normally empty: `aso install` reads it from air_server's first start and remembers it. |
| `air.host` | `127.0.0.1` | needs a restart | The host fsp-mint dials to reach air_server (port 5702 is fixed on the client side). Written to fsp-mint as air.host. |
| `air.public_name` | (not set) | needs a restart | The public host name nginx answers on, and the host in this service's air.fsp entry. Empty: no nginx file is written. |
| `air.public_cert` | (not set) | needs a restart | nginx's TLS certificate file (absolute path). With public_key. |
| `air.public_key` | (not set) | needs a restart | nginx's TLS key file (absolute path). |
| `air.nginx_conf` | `<etc_dir>/nginx-air.conf` | needs a restart | Where the nginx server block is written. Point it into nginx's own folder (for example /etc/nginx/conf.d/aso-air.conf), or include it from there. |
| `air.ws_addr` | `127.0.0.1:5702` | needs a restart | AIR_WS_ADDR: where air_server listens for the plain WebSocket, and where nginx forwards to. air_server's own default is 0.0.0.0:5702; aso binds loopback because nginx is in front. |
| `air.health_addr` | `127.0.0.1:5701` | needs a restart | AIR_HEALTH_ADDR: air_server's health listener, which answers a bare `ok`. |
| `air.tls_addr` | (not set) | needs a restart | AIR_TLS_ADDR: air_server's own TLS listener. Empty: off (nginx terminates TLS). With tls_cert and tls_key. |
| `air.tls_cert` | (not set) | needs a restart | AIR_TLS_CERT (absolute path). |
| `air.tls_key` | (not set) | needs a restart | AIR_TLS_KEY (absolute path). |
| `air.identity` | (not set) | needs a restart | AIR_SECRET_FILE: an identity file that seeds a new STORAGE.db (absolute path). Empty: air_server makes one. Once STORAGE.db holds an identity this is ignored by air_server. |
| `air.storage` | `<data_dir>/air/STORAGE.db` | needs a restart | AIR_STORAGE: air_server's SQLite database, which holds the identity. |
| `air.fee_read` | `0` | needs a restart | AIR server fee per read. Only 0 is accepted: stock air_server 1.0.1 has no fee hook (upstream item 1). |
| `air.fee_write` | `0` | needs a restart | AIR server fee per write. Only 0 is accepted: stock air_server 1.0.1 has no fee hook (upstream item 1). |
| `fsp.bin` | `fsp-mint` | reloads live | The fsp-mint binary used until `aso install` has put a release in bin_dir (a path, or a name on PATH). |
| `fsp.version` | (not set) | reloads live | Pins fsp-mint to this release (x.y.z). Empty: the newest tagged release. Never a release candidate. |
| `wallet.bin` | `ecash-wallet` | reloads live | The ecash-wallet binary, as fsp.bin. |
| `wallet.version` | (not set) | reloads live | Pins ecash-wallet to this release (x.y.z). Empty: the newest tagged release. |
| `redeem.payout` | (not set) | reloads live | Where the ESP the service earns is paid out, for example btc:ADDRESS. Empty: no automatic redeem. |
| `redeem.interval_hours` | `24` | reloads live | How often the automatic redeem runs, in hours (the timer is written again by `aso reload`). |
| `release.url` | `https://api.github.com` | reloads live | The GitHub API address releases are listed from. |
| `release.keep` | `3` | reloads live | How many downloaded releases of each program are kept in the aso folder; the installed one and the one before it always stay. |

### fsp-mint (`[fsp.<section>]`)

| Setting | Default | Reload | Note |
|---|---|---|---|
| `fsp.server.network` | `main` | needs a restart | fsp-mint's `server.network` (see `fsp-mint config`) |
| `fsp.server.bind` | `127.0.0.1:3200` | needs a restart | fsp-mint's `server.bind` (see `fsp-mint config`) |
| `fsp.server.max_request_bytes` | `1048576` | reloads live | fsp-mint's `server.max_request_bytes` (see `fsp-mint config`) |
| `fsp.service.did` | (not set) | needs a restart | fsp-mint's `service.did` (see `fsp-mint config`) |
| `fsp.fees.issue_fee_per_byte` | `0` | reloads live | fsp-mint's `fees.issue_fee_per_byte` (see `fsp-mint config`) |
| `fsp.rotation.activation_hours` | `8760` | needs a restart | fsp-mint's `rotation.activation_hours` (see `fsp-mint config`) |
| `fsp.rotation.period_hours` | `8760` | needs a restart | fsp-mint's `rotation.period_hours` (see `fsp-mint config`) |
| `fsp.issue.max_items` | `1000` | reloads live | fsp-mint's `issue.max_items` (see `fsp-mint config`) |
| `fsp.spend.max_items` | `1000` | reloads live | fsp-mint's `spend.max_items` (see `fsp-mint config`) |
| `fsp.spend.max_memo_bytes` | `256` | reloads live | fsp-mint's `spend.max_memo_bytes` (see `fsp-mint config`) |
| `fsp.spend.max_batch` | `1024` | reloads live | fsp-mint's `spend.max_batch` (see `fsp-mint config`) |
| `fsp.esp.keyset_ttl_secs` | `3600` | reloads live | fsp-mint's `esp.keyset_ttl_secs` (see `fsp-mint config`) |
| `fsp.air.esp_did` | (not set) | needs a restart | fsp-mint's `air.esp_did` (see `fsp-mint config`) |
| `fsp.air.esp_server` | (not set) | needs a restart | fsp-mint's `air.esp_server` (see `fsp-mint config`) |
| `fsp.air.esp_host` | (not set) | needs a restart | fsp-mint's `air.esp_host` (see `fsp-mint config`) |
| `fsp.air.server` | (not set) | needs a restart | fsp-mint's `air.server` (see `fsp-mint config`) |
| `fsp.air.host` | (not set) | needs a restart | fsp-mint's `air.host` (see `fsp-mint config`) |
| `fsp.air.held_reads` | `16` | needs a restart | fsp-mint's `air.held_reads` (see `fsp-mint config`) |
| `fsp.air.reply_wait_secs` | `30` | needs a restart | fsp-mint's `air.reply_wait_secs` (see `fsp-mint config`) |
| `fsp.air.attempts` | `3` | needs a restart | fsp-mint's `air.attempts` (see `fsp-mint config`) |
| `fsp.air.max_skew_secs` | `300` | needs a restart | fsp-mint's `air.max_skew_secs` (see `fsp-mint config`) |
| `fsp.air.timeout_secs` | `10` | needs a restart | fsp-mint's `air.timeout_secs` (see `fsp-mint config`) |
| `fsp.storage.mint_state` | (not set) | needs a restart | fsp-mint's `storage.mint_state` (see `fsp-mint config`) |

### ecash-wallet (`[wallet.<section>]`)

| Setting | Default | Reload | Note |
|---|---|---|---|
| `wallet.storage.wallet` | (not set) | reloads live | ecash-wallet's `storage.wallet` (it reads its file on every run) |
| `wallet.storage.app` | (not set) | reloads live | ecash-wallet's `storage.app` (it reads its file on every run) |
| `wallet.air.psp_did` | (not set) | reloads live | ecash-wallet's `air.psp_did` (it reads its file on every run) |
| `wallet.air.esp_did` | (not set) | reloads live | ecash-wallet's `air.esp_did` (it reads its file on every run) |
| `wallet.air.server` | (not set) | reloads live | ecash-wallet's `air.server` (it reads its file on every run) |
| `wallet.air.host` | (not set) | reloads live | ecash-wallet's `air.host` (it reads its file on every run) |
| `wallet.air.fsp` | (not set) | reloads live | ecash-wallet's `air.fsp` (it reads its file on every run) |
| `wallet.air.reply_wait_secs` | `30` | reloads live | ecash-wallet's `air.reply_wait_secs` (it reads its file on every run) |
| `wallet.air.attempts` | `3` | reloads live | ecash-wallet's `air.attempts` (it reads its file on every run) |
| `wallet.air.max_skew_secs` | `300` | reloads live | ecash-wallet's `air.max_skew_secs` (it reads its file on every run) |
| `wallet.air.timeout_secs` | `10` | reloads live | ecash-wallet's `air.timeout_secs` (it reads its file on every run) |
| `wallet.psp.min_purchase` | `50` | reloads live | ecash-wallet's `psp.min_purchase` (it reads its file on every run) |

## Upgrading

`aso upgrade [air_server|fsp-mint|ecash-wallet ...]` brings programs to their
target: **the newest tagged release unless `version` pins one**. Release
candidates and pre-releases are never installed (a pin on one is refused when
`aso.toml` loads). Only release binaries are installed, checked against the
release's SHA-256; air_server's binary is the one you supply (`air.bin`).

**It is a brief downtime.** For air_server and fsp-mint the old program is
stopped and the new one started (one `restart` call after two renames), and aso
reports how long it was down. Everything that can fail (download, checksum,
copy) happens before anything is stopped, and a new build that does not become
healthy is replaced by the old one. fsp-mint cannot hand over: one process may
write its data folder (the writer lock), so the new one starts only after the old
one has exited. Wallets' AIR requests wait in the mailbox and are answered when it
is back, and none is lost or answered twice. air_server has one SQLite database
and one process that owns it and both sockets; sessions are kept in the database
and fsp-mint's AIR client reconnects. ecash-wallet is run on demand, so
replacing its file is no downtime. `aso upgrade` warns of this and asks first
(`--yes` answers for you; with no terminal and no `--yes` nothing is changed).

* `--at HH:MM` or `--in 90m` / `--in 1h30m` schedules it: a one-shot systemd timer
  or launchd job that survives a reboot. It prints for example `Upgrade
  scheduled for Sat Oct 24 03:00 EDT (in 15h 28m)`. One upgrade can be pending;
  `aso upgrade status` shows it and `aso upgrade cancel` removes it. A run that
  starts more than an hour late is skipped and logged.
* A pinned program is warned about (pin and newest release) and asked about, per
  pin. `--keep-pins` keeps all, `--reset-pins` resets all (the `version` line is
  removed from `aso.toml`, the old file kept). With no terminal a pin is kept,
  with the warning.
* `aso upgrade check` lists which program has a newer release and ends with the
  pin warning. `--dry-run` says what would happen and changes nothing.

## The automatic redeem

With `redeem.payout` set (for example `btc:ADDRESS`), a timer runs `aso redeem
run` every `redeem.interval_hours` (24; on Linux also ten minutes after boot). One run:

1. asks the running fsp-mint to export into an owner-only folder with
   `fsp-mint export-esp --serve-only`, which only asks the running `serve` and
   fails, touching nothing in fsp-mint's data folder, when none answers: the job
   runs as root, and must never make a file there that fsp-mint cannot write.
   aso checks fsp-mint answers its health check first, and skips the export if
   the installed fsp-mint is older than 0.8.0 (it has no `--serve-only`) or aso
   has no record of its version;
2. `ecash-wallet receive FILE` for each file there, and deletes the file as soon
   as the wallet has it;
3. sends again (`redeem --resume ID`) any redemption the wallet shows as unanswered;
4. `ecash-wallet redeem --service-key-file KEY --memo PAYOUT` (the key is a file,
   never on a command line);
5. `ecash-wallet redeem --collect`.

fsp-mint marks each note exported when it hands it over, so a second run hands
out nothing twice. Unset, there is no automatic redeem and no job is written.

## Logs and files that grow

Every log aso writes, or sends a program's output to, is capped the same way:
when it is about to be opened and is bigger than `service.log_max_size_mb`
(50), it is moved to `<log>.<epoch>.bak` first, and only the newest
`service.log_keep` (4) of those stay. The service units run `aso logs rotate`
before the program opens its log, on every start.

fsp-mint and air_server keep their logs open while they run, so a job of its own,
`aso-logs` (a systemd timer, or the launchd job `org.servicepoints.aso-logs`),
runs `aso logs rotate --running` on both every 5 minutes, as the service
account, with the same caps. A log over the cap is copied to
`<log>.<epoch>.bak` and emptied in place; the program writes on at the end of
the now empty file, since its output is opened for appending. So a log can pass
the cap by up to 5 minutes of output, and a line written between the copy and
the emptying is lost. `aso install` (and `aso reload` after an upgrade of aso)
writes and loads the job.

| File | Cap |
|---|---|
| `logs/fsp-mint.log`, `logs/air_server.log`, `logs/aso-upgrade.log`, `logs/aso-redeem.log` | `service.log_max_size_mb`, `service.log_keep` |
| `aso.toml.<epoch>.bak` | `service.config_backups` (5) |
| `releases/<program>/<version>/` downloads | `release.keep` (3; the installed one and the one before it always stay) |
| `bin/<program>.previous` | one per program, replaced at each upgrade |
| hand-over files in `data/redeem/handover` | deleted as soon as the wallet has received them |

fsp-mint's own growing records are not aso's to cap (ROADMAP item 29: passed
through when fsp-mint has the settings). air_server's `STORAGE.db` and the
wallet file grow by upstream design. aso has **no snapshot or backup feature**;
see "Disk snapshots" for what a snapshot takes.

## Disk snapshots

Would a disk snapshot taken while fsp-mint serves be consistent? **Yes, if the
snapshot is atomic across the volume** (LVM, ZFS, EBS and cloud volume snapshots
are): it then looks exactly like a power cut at that instant, which fsp-mint is
written to survive. Every small JSON file (`issues.json`, `keysets.json`,
`air_mailbox_next`, ...) is written to a temporary file, synced and renamed, so a
snapshot holds the old file or the new one, never half of one; a spend is
appended to `spends-<keyset>.jsonl` and synced before it is answered; and
fsp-mint cuts a torn last line from that log when it starts.

**A copy of the folder made file by file while fsp-mint runs (`cp`, `rsync`) is
not consistent**: each file is whole, but they are read at different moments, so
the spent log can be newer than `issues.json` and `aso audit` on the copy may
warn. Take the copy from an atomic snapshot.

Files a snapshot can catch half-written: the last line of
`spends-<keyset>.jsonl` (cut at the next start, and its spend was never
answered), `*.tmp` beside a JSON file being replaced (ignored, replaced at the
next write), and `*.part` files of an export in progress (settled by the next
`export-esp`). air_server's `STORAGE.db` is a SQLite database that air_server owns;
how it behaves under a snapshot is air_server's to say (if it uses a write-ahead
log, the `-wal` file must be in the same snapshot). aso could not check it.
aso has no snapshot or backup feature.

## Needs from upstream

* **air_server fee hook** (upstream item 1): `air.fee_read` and `air.fee_write`
  exist, are 0 by default, and a non-zero value is refused until air_server can
  take a fee.
* **air_server releases** (item 16) and a **version** air_server can report
  (item 15): until then aso records the version of the file it installed.
* **`--version` for fsp-mint** (and a version in its `/health`): today aso knows
  fsp-mint's version only from its own record of the file it installed, and
  confirms an upgrade by health alone.
* air_server's own default listens on all interfaces; aso binds it to loopback
  because nginx is in front. `air.host` reaches fsp-mint's client on the fixed
  port 5702.

## Folders

`--dir` holds `aso.toml`, `state.json`, `installed/` (what aso installed: version
and SHA-256), `releases/`, `upgrade/pending.json`, `bin/`, `etc/` (every generated
file), `data/` (`air/`, `fsp/`, `wallet/`, `redeem/`) and `logs/`. The units run
the `aso` file that installed them, so keep it where it is.
