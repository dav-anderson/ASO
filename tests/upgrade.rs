//! `aso upgrade`: check, status, pins, release candidates, confirmation,
//! scheduling, the scheduled run, rollback, and the short downtime.

mod common;

use std::fs;

use aso::exec::Exec;
use common::*;

fn pinned_text(w: &World, pin: &str) -> String {
    w.text().replacen(
        "[fsp.server]",
        &format!("[fsp]\nversion = \"{pin}\"\n\n[fsp.server]"),
        1,
    )
}

#[test]
fn check_says_which_programs_have_a_newer_release_and_changes_nothing() {
    // Fails if check counts a release candidate as newest, mislabels air_server
    // (no releases upstream) or stops a service.
    let (w, _) = World::installed("check");
    let src = newer_source();
    let (done, exec) = w.run(&["upgrade", "check"], &src, &mut Scripted::new(false));
    assert_eq!(done.code, 0, "{}", done.out);
    let line = |name: &str| {
        done.out
            .lines()
            .find(|l| l.starts_with(name))
            .unwrap_or_else(|| panic!("{name}: {}", done.out))
            .to_string()
    };
    assert!(
        line("fsp-mint").contains("installed 0.8.0")
            && line("fsp-mint").contains("newest 0.9.0")
            && line("fsp-mint").ends_with("upgrade available (0.9.0)"),
        "{}",
        done.out
    );
    assert!(line("ecash-wallet").ends_with("up to date"), "{}", done.out);
    assert!(
        line("air_server")
            .ends_with("no releases published upstream: the binary is yours (air.bin)"),
        "{}",
        done.out
    );
    assert!(done.out.contains("summary: 1 up to date, 1 upgrade available, 0 pinned, 0 unknown, 1 without upstream releases"), "{}", done.out);
    assert_eq!(stops(&exec.calls()), Vec::<String>::new());
}

#[test]
fn check_warns_about_a_pin_that_holds_a_program_back() {
    // Fails if a pin behind the newest release is not warned about by check.
    let (w, _) = World::installed("checkpin");
    let src = newer_source();
    let exec = w.exec();
    let done = cli(
        &w.dir,
        &pinned_text(&w, "0.8.0"),
        &["upgrade", "check"],
        &exec,
        &src,
        &mut Scripted::new(false),
    );
    assert!(done.out.contains("WARNING: these programs are pinned and are not at the newest release:\n  fsp-mint: pinned at 0.8.0 (newest 0.9.0)"), "{}", done.out);
    assert!(done.out.contains("1 pinned"), "{}", done.out);
}

#[test]
fn upgrade_warns_about_the_downtime_and_changes_nothing_without_a_yes() {
    // Fails if anything is downloaded, swapped or restarted before the operator says yes.
    let (w, _) = World::installed("noyes");
    let src = newer_source();
    let (done, exec) = w.run(&["upgrade"], &src, &mut Scripted::new(false));
    assert_eq!(done.code, 0, "{}", done.out);
    assert!(done.out.contains("is a BRIEF DOWNTIME"), "{}", done.out);
    assert!(
        done.out
            .contains("air_server: BRIEF DOWNTIME: air_server is stopped and the new one started"),
        "{}",
        done.out
    );
    assert!(
        done.out
            .contains("fsp-mint: BRIEF DOWNTIME: fsp-mint is stopped and the new one started"),
        "{}",
        done.out
    );
    assert!(
        done.out.contains("ecash-wallet: no downtime"),
        "{}",
        done.out
    );
    assert!(
        done.out.contains("not confirmed: nothing was changed"),
        "{}",
        done.out
    );
    assert_eq!(stops(&exec.calls()), Vec::<String>::new());
    assert_eq!(w.read("bin/fsp-mint"), "fsp-mint build 0.8.0");
    assert!(
        src.fetched.lock().unwrap().is_empty(),
        "{:?}",
        src.fetched.lock().unwrap()
    );
}

#[test]
fn an_answered_yes_upgrades_fsp_mint_with_one_restart_and_reports_the_downtime() {
    // Fails if air_server is restarted needlessly, the old binary is not kept,
    // the record is not updated, the release candidate is chosen, or the
    // downtime is not reported.
    let (w, _) = World::installed("yes");
    let src = newer_source();
    let mut ask = Scripted::new(true).saying(&[true]);
    let (done, exec) = w.run(&["upgrade"], &src, &mut ask);
    assert_eq!(done.code, 0, "{}", done.out);
    assert_eq!(ask.asked, vec!["Upgrade now?"]);
    assert_eq!(
        stops(&exec.calls()),
        vec!["systemctl restart aso-fsp.service"]
    );
    assert_eq!(w.read("bin/fsp-mint"), "fsp-mint build 0.9.0");
    assert_eq!(w.read("bin/fsp-mint.previous"), "fsp-mint build 0.8.0");
    assert!(
        w.read("installed/fsp-mint.json")
            .contains("\"version\": \"0.9.0\""),
        "{}",
        w.read("installed/fsp-mint.json")
    );
    assert_eq!(
        *src.fetched.lock().unwrap(),
        vec!["fsp-mint 0.9.0".to_string()]
    );
    assert!(
        done.out
            .contains("fsp-mint: 0.8.0 -> 0.9.0: BRIEF DOWNTIME"),
        "{}",
        done.out
    );
    assert!(
        done.out.contains(
            "fsp-mint: was briefly down while the new version started: back, healthy, after "
        ),
        "{}",
        done.out
    );
    assert!(done.out.contains("fsp-mint: now 0.9.0"), "{}", done.out);
    assert!(
        done.out
            .contains("air_server: already 1.0.1: nothing to do"),
        "{}",
        done.out
    );
    assert!(
        done.out
            .contains("ecash-wallet: already 0.8.0: nothing to do"),
        "{}",
        done.out
    );
}

#[test]
fn everything_up_to_date_stops_nothing() {
    // Fails if an upgrade with nothing to do restarts anything.
    let (w, src) = World::installed("uptodate");
    let (done, exec) = w.run(&["upgrade", "--yes"], &src, &mut Scripted::new(false));
    assert_eq!(done.code, 0, "{}", done.out);
    assert!(
        done.out
            .contains("everything up to date: nothing was stopped"),
        "{}",
        done.out
    );
    assert_eq!(stops(&exec.calls()), Vec::<String>::new());
}

#[test]
fn a_dry_run_downloads_nothing_and_stops_nothing() {
    // Fails if --dry-run fetches a release or touches a service.
    let (w, _) = World::installed("dry");
    let src = newer_source();
    let (done, exec) = w.run(&["upgrade", "--dry-run"], &src, &mut Scripted::new(false));
    assert_eq!(done.code, 0, "{}", done.out);
    assert!(done.out.contains("fsp-mint: the build would be downloaded from the release source and checked against its SHA-256 before anything is stopped"), "{}", done.out);
    assert!(
        done.out.contains("dry run: nothing was changed"),
        "{}",
        done.out
    );
    assert!(src.fetched.lock().unwrap().is_empty());
    assert_eq!(stops(&exec.calls()), Vec::<String>::new());
    assert_eq!(w.read("bin/fsp-mint"), "fsp-mint build 0.8.0");
}

#[test]
fn a_download_that_fails_its_checksum_is_refused_before_anything_is_stopped() {
    // Fails if a corrupt download is installed, or a service is stopped for it.
    let (w, _) = World::installed("badsum");
    let mut src = newer_source();
    src.builds.iter_mut().find(|b| b.1 == "0.9.0").unwrap().3 = Some("0".repeat(64));
    let (done, exec) = w.run(&["upgrade", "--yes"], &src, &mut Scripted::new(false));
    assert_eq!(done.code, 1, "{}", done.out);
    assert!(done.out.contains("failed its checksum"), "{}", done.out);
    assert_eq!(stops(&exec.calls()), Vec::<String>::new());
    assert_eq!(w.read("bin/fsp-mint"), "fsp-mint build 0.8.0");
}

#[test]
fn a_restart_that_fails_puts_the_old_binary_back() {
    // Fails if a failed start of the new build leaves the new binary in place.
    let (w, _) = World::installed("rollback");
    let src = newer_source();
    let inner = w.exec();
    let broken = fake(move |bin, args| {
        if bin == "systemctl" && args.first().map(String::as_str) == Some("restart") {
            ran(1, "", "Job for aso-fsp.service failed")
        } else {
            inner.run(bin, args, &[])
        }
    });
    let done = cli(
        &w.dir,
        &w.text(),
        &["upgrade", "fsp-mint", "--yes"],
        &broken,
        &src,
        &mut Scripted::new(false),
    );
    assert_eq!(done.code, 1, "{}", done.out);
    assert!(done.out.contains("FAIL: fsp-mint:"), "{}", done.out);
    assert!(
        done.out.contains("the old one is back in place"),
        "{}",
        done.out
    );
    assert_eq!(w.read("bin/fsp-mint"), "fsp-mint build 0.8.0");
    assert!(
        w.read("installed/fsp-mint.json")
            .contains("\"version\": \"0.8.0\"")
    );
}

#[test]
fn a_release_candidate_is_never_installed_even_when_it_is_the_only_newer_tag() {
    // Fails if an rc tag becomes the target.
    let (w, _) = World::installed("rc");
    let src = FakeSource::new()
        .with(aso::program::Program::Fsp, &["v0.8.0", "v0.10.0-rc1"])
        .build(
            aso::program::Program::Fsp,
            "0.10.0-rc1",
            "fsp-mint build rc",
        )
        .with(aso::program::Program::Wallet, &["v0.8.0"])
        .build(aso::program::Program::Wallet, "0.8.0", "x");
    let (done, exec) = w.run(&["upgrade", "--yes"], &src, &mut Scripted::new(false));
    assert!(
        done.out.contains("fsp-mint: already 0.8.0: nothing to do"),
        "{}",
        done.out
    );
    assert!(src.fetched.lock().unwrap().is_empty());
    assert_eq!(stops(&exec.calls()), Vec::<String>::new());
    let exec = w.exec();
    let done = cli(
        &w.dir,
        &pinned_text(&w, "0.10.0-rc1"),
        &["upgrade", "--yes"],
        &exec,
        &src,
        &mut Scripted::new(false),
    );
    assert_eq!(done.code, 2, "{done:?}");
    assert!(
        done.err.contains(
            "fsp.version is `0.10.0-rc1`: a release candidate or pre-release is never installed"
        ),
        "{}",
        done.err
    );
}

#[test]
fn a_pin_is_warned_about_and_kept_when_there_is_no_terminal() {
    // Fails if a pin is reset without being asked, or kept silently.
    let (w, _) = World::installed("pinkeep");
    let src = newer_source();
    let exec = w.exec();
    let done = cli(
        &w.dir,
        &pinned_text(&w, "0.8.0"),
        &["upgrade", "--yes"],
        &exec,
        &src,
        &mut Scripted::new(false),
    );
    assert_eq!(done.code, 0, "{}", done.out);
    assert!(
        done.out
            .contains("  fsp-mint: pinned at 0.8.0 (newest 0.9.0)"),
        "{}",
        done.out
    );
    assert!(
        done.out
            .contains("WARNING: fsp-mint stays pinned at 0.8.0: there is no terminal to ask"),
        "{}",
        done.out
    );
    assert_eq!(w.read("bin/fsp-mint"), "fsp-mint build 0.8.0");
    assert!(
        fs::read_to_string(w.dir.join("aso.toml"))
            .unwrap()
            .contains("version = \"0.8.0\"")
    );
}

#[test]
fn a_pin_is_asked_about_and_a_yes_resets_it_with_a_backup_of_aso_toml() {
    // Fails if the pin is not asked about by name, the answer is ignored, or the
    // old aso.toml is not kept.
    let (w, _) = World::installed("pinreset");
    let src = newer_source();
    let text = pinned_text(&w, "0.8.0");
    let mut ask = Scripted::new(true).saying(&[true, true]);
    let exec = w.exec();
    let done = cli(&w.dir, &text, &["upgrade"], &exec, &src, &mut ask);
    assert_eq!(done.code, 0, "{}", done.out);
    assert_eq!(
        ask.asked[0],
        "Reset the pin of fsp-mint (pinned at 0.8.0, newest 0.9.0) so the newest is installed?"
    );
    assert_eq!(ask.asked[1], "Upgrade now?");
    assert_eq!(w.read("bin/fsp-mint"), "fsp-mint build 0.9.0");
    assert!(
        !w.read("aso.toml").contains("version = \"0.8.0\""),
        "{}",
        w.read("aso.toml")
    );
    assert_eq!(w.read(&format!("aso.toml.{NOW}.bak")), text);

    let (w, _) = World::installed("pinkeep2");
    let mut ask = Scripted::new(true).saying(&[false, true]);
    let done = cli(
        &w.dir,
        &pinned_text(&w, "0.8.0"),
        &["upgrade"],
        &w.exec(),
        &src,
        &mut ask,
    );
    assert_eq!(done.code, 0, "{}", done.out);
    assert_eq!(w.read("bin/fsp-mint"), "fsp-mint build 0.8.0");
    assert!(!w.dir.join(format!("aso.toml.{NOW}.bak")).exists());
}

#[test]
fn keep_pins_and_reset_pins_answer_for_every_pin() {
    // Fails if --reset-pins asks anyway, or --keep-pins resets.
    let (w, _) = World::installed("pinflags");
    let src = newer_source();
    let mut ask = Scripted::new(true).saying(&[true]);
    let done = cli(
        &w.dir,
        &pinned_text(&w, "0.8.0"),
        &["upgrade", "--reset-pins"],
        &w.exec(),
        &src,
        &mut ask,
    );
    assert_eq!(done.code, 0, "{}", done.out);
    assert_eq!(ask.asked, vec!["Upgrade now?"]);
    assert_eq!(w.read("bin/fsp-mint"), "fsp-mint build 0.9.0");
    let (w, _) = World::installed("pinflags2");
    let mut ask = Scripted::new(true).saying(&[true]);
    cli(
        &w.dir,
        &pinned_text(&w, "0.8.0"),
        &["upgrade", "--keep-pins"],
        &w.exec(),
        &src,
        &mut ask,
    );
    assert_eq!(ask.asked, vec!["Upgrade now?"]);
    assert_eq!(w.read("bin/fsp-mint"), "fsp-mint build 0.8.0");
}

#[test]
fn an_upgrade_is_scheduled_shown_refused_twice_and_cancelled() {
    // Fails if the time is not printed as spi prints it, a second upgrade is
    // accepted, the job is not a one-shot timer in UTC, or cancel leaves a file.
    let (w, _) = World::installed("schedule");
    let src = newer_source();
    let (done, exec) = w.run(
        &["upgrade", "--in", "15h28m", "--yes"],
        &src,
        &mut Scripted::new(false),
    );
    assert_eq!(done.code, 0, "{}", done.out);
    assert!(
        done.out
            .lines()
            .any(|l| l.starts_with("Upgrade scheduled for ") && l.ends_with("(in 15h 28m)")),
        "{}",
        done.out
    );
    assert_eq!(stops(&exec.calls()), Vec::<String>::new());
    assert!(
        exec.calls()
            .contains(&"systemctl enable --now aso-upgrade.timer".to_string()),
        "{:#?}",
        exec.calls()
    );
    assert!(
        w.read("units/aso-upgrade.timer")
            .contains("OnCalendar=2026-10-15 09:14:40 UTC\n"),
        "{}",
        w.read("units/aso-upgrade.timer")
    );
    assert!(
        w.read("units/aso-upgrade.timer")
            .contains("Persistent=true")
    );
    assert!(
        w.read("units/aso-upgrade.service")
            .contains("upgrade run-scheduled")
    );
    assert_eq!(w.read("bin/fsp-mint"), "fsp-mint build 0.8.0");

    let (done, _) = w.run(&["upgrade", "status"], &src, &mut Scripted::new(false));
    assert!(done.out.contains("Upgrade scheduled for "), "{}", done.out);
    assert!(
        done.out
            .contains("programs: air_server, fsp-mint and ecash-wallet (all three)"),
        "{}",
        done.out
    );
    assert!(
        done.out.contains(
            "If the machine cannot start it within 1h of that time, it is skipped and removed."
        ),
        "{}",
        done.out
    );

    let (done, _) = w.run(
        &["upgrade", "--in", "1h", "--yes"],
        &src,
        &mut Scripted::new(false),
    );
    assert_eq!(done.code, 1);
    assert!(
        done.out.contains("an upgrade is already scheduled for "),
        "{}",
        done.out
    );
    assert!(
        done.out
            .contains("cancel it first with `aso upgrade cancel`"),
        "{}",
        done.out
    );

    let (done, exec) = w.run(&["upgrade", "cancel"], &src, &mut Scripted::new(false));
    assert!(
        done.out.starts_with("Cancelled the upgrade scheduled for "),
        "{}",
        done.out
    );
    assert!(
        exec.calls()
            .contains(&"systemctl disable --now aso-upgrade.timer".to_string())
    );
    assert!(
        !w.dir.join("units/aso-upgrade.timer").exists()
            && !w.dir.join("units/aso-upgrade.service").exists()
    );
    assert!(!w.dir.join("upgrade/pending.json").exists());
    let (done, _) = w.run(&["upgrade", "status"], &src, &mut Scripted::new(false));
    assert!(
        done.out.contains("No upgrade is scheduled."),
        "{}",
        done.out
    );
    let (done, _) = w.run(&["upgrade", "cancel"], &src, &mut Scripted::new(false));
    assert!(
        done.out
            .contains("No upgrade is scheduled: nothing to cancel."),
        "{}",
        done.out
    );
}

#[test]
fn at_and_in_need_a_good_time_and_cannot_be_given_together() {
    // Fails if a bad time is accepted or both flags are.
    let (w, src) = World::installed("badtime");
    for (args, said) in [
        (
            vec!["upgrade", "--at", "25:00", "--yes"],
            "`--at` takes a time like 03:00",
        ),
        (
            vec!["upgrade", "--in", "5", "--yes"],
            "`--in` takes a duration like",
        ),
        (
            vec!["upgrade", "--at", "03:00", "--in", "1h"],
            "--at and --in cannot be used together",
        ),
    ] {
        let (done, exec) = w.run(&args, &src, &mut Scripted::new(false));
        assert_ne!(done.code, 0);
        assert!(
            format!("{}{}", done.out, done.err).contains(said),
            "{args:?}: {}{}",
            done.out,
            done.err
        );
        assert!(!w.dir.join("upgrade/pending.json").exists());
        assert_eq!(stops(&exec.calls()), Vec::<String>::new());
    }
}

#[test]
fn the_scheduled_run_upgrades_on_time_skips_when_late_and_waits_when_early() {
    // Fails if the job upgrades when it is an hour late or more, keeps the
    // job after it ran, or acts before the time.
    let src = newer_source();
    let schedule = |name: &str| {
        let (w, _) = World::installed(name);
        let (done, _) = w.run(
            &["upgrade", "--in", "1m", "--reset-pins", "--yes"],
            &src,
            &mut Scripted::new(false),
        );
        assert_eq!(done.code, 0, "{}", done.out);
        w
    };
    let due = NOW + 60;

    let w = schedule("run-early");
    let exec = w.exec();
    let done = cli_at(
        NOW + 10,
        &w.dir,
        &w.text(),
        &["upgrade", "run-scheduled"],
        &exec,
        &src,
        &mut Scripted::new(false),
    );
    assert!(done.out.contains("not yet: left scheduled"), "{}", done.out);
    assert!(w.dir.join("upgrade/pending.json").exists());
    assert_eq!(w.read("bin/fsp-mint"), "fsp-mint build 0.8.0");

    let w = schedule("run-on-time");
    let exec = w.exec();
    let done = cli_at(
        due + 30,
        &w.dir,
        &w.text(),
        &["upgrade", "run-scheduled"],
        &exec,
        &src,
        &mut Scripted::new(false),
    );
    assert_eq!(done.code, 0, "{}", done.out);
    assert_eq!(w.read("bin/fsp-mint"), "fsp-mint build 0.9.0");
    assert_eq!(
        stops(&exec.calls()),
        vec!["systemctl restart aso-fsp.service"]
    );
    assert!(!w.dir.join("upgrade/pending.json").exists());
    assert!(!w.dir.join("units/aso-upgrade.timer").exists());
    assert!(
        w.read("logs/aso-upgrade.log")
            .contains("running the scheduled upgrade"),
        "{}",
        w.read("logs/aso-upgrade.log")
    );

    let w = schedule("run-late");
    let exec = w.exec();
    let done = cli_at(
        due + 7200,
        &w.dir,
        &w.text(),
        &["upgrade", "run-scheduled"],
        &exec,
        &src,
        &mut Scripted::new(false),
    );
    let skipped = format!(
        "SKIPPED: the scheduled upgrade was due at {due} and it is 7200 s late (more than the 3600 s allowed): nothing was changed"
    );
    assert!(done.out.contains(&skipped), "{}", done.out);
    assert!(w.read("logs/aso-upgrade.log").contains(&skipped));
    assert_eq!(w.read("bin/fsp-mint"), "fsp-mint build 0.8.0");
    assert_eq!(stops(&exec.calls()), Vec::<String>::new());
    assert!(!w.dir.join("upgrade/pending.json").exists());
}

#[test]
fn a_scheduled_upgrade_with_reset_pins_resets_them_when_it_runs_not_when_it_is_scheduled() {
    // Fails if scheduling edits aso.toml, or the run keeps the pin.
    let (w, _) = World::installed("schedpin");
    let src = newer_source();
    let text = pinned_text(&w, "0.8.0");
    let done = cli(
        &w.dir,
        &text,
        &["upgrade", "--in", "1m", "--reset-pins", "--yes"],
        &w.exec(),
        &src,
        &mut Scripted::new(false),
    );
    assert_eq!(done.code, 0, "{}", done.out);
    assert_eq!(w.read("aso.toml"), text);
    let done = cli_at(
        NOW + 61,
        &w.dir,
        &text,
        &["upgrade", "run-scheduled"],
        &w.exec(),
        &src,
        &mut Scripted::new(false),
    );
    assert_eq!(done.code, 0, "{}", done.out);
    assert!(!w.read("aso.toml").contains("version = \"0.8.0\""));
    assert_eq!(w.read(&format!("aso.toml.{}.bak", NOW + 61)), text);
    assert_eq!(w.read("bin/fsp-mint"), "fsp-mint build 0.9.0");
}
