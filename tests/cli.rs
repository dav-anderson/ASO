//! Every command, through the command line: help, start, stop, restart, reload,
//! audit, versions, config, and that nothing else can stop a service.

mod common;

use std::fs;

use common::*;

#[test]
fn help_lists_every_command() {
    // Fails if a command is built and left out of --help (or the other way).
    let done = cli(
        &scratch("help"),
        "",
        &["--help"],
        &quiet(),
        &FakeSource::new(),
        &mut Scripted::new(false),
    );
    assert_eq!(done.code, 0);
    for command in [
        "install",
        "start",
        "stop",
        "restart",
        "reload",
        "status",
        "audit",
        "versions",
        "upgrade",
        "upgrade check",
        "upgrade status",
        "upgrade cancel",
        "config",
        "redeem run",
        "logs rotate",
    ] {
        assert!(
            done.out
                .lines()
                .any(|l| l.trim_start().starts_with(&format!("{command} "))),
            "`{command}` is not in the help:\n{}",
            done.out
        );
    }
    assert!(
        done.out.contains("--at HH:MM | --in DURATION"),
        "{}",
        done.out
    );
}

#[test]
fn an_unknown_command_or_extra_word_is_a_usage_error_that_changes_nothing() {
    // Fails if a typo runs something, or exits 0.
    let exec = quiet();
    let src = FakeSource::new();
    let dir = scratch("usage");
    let done = cli(&dir, "", &["strat"], &exec, &src, &mut Scripted::new(false));
    assert_eq!(
        (done.code, done.err.contains("unknown command `strat`")),
        (2, true),
        "{done:?}"
    );
    let done = cli(
        &dir,
        "",
        &["start", "now"],
        &exec,
        &src,
        &mut Scripted::new(false),
    );
    assert_eq!(
        (
            done.code,
            done.err.contains("`aso start` does not take `now`")
        ),
        (2, true),
        "{done:?}"
    );
    assert_eq!(exec.calls(), Vec::<String>::new());
}

#[test]
fn a_bad_aso_toml_stops_every_command_before_it_runs_anything() {
    // Fails if a command runs with a file that has a problem in it.
    let exec = quiet();
    let done = cli(
        &scratch("bad"),
        "[air]\nfee_read = 3\n",
        &["reload"],
        &exec,
        &FakeSource::new(),
        &mut Scripted::new(false),
    );
    assert_eq!(done.code, 2, "{done:?}");
    assert!(
        done.err.contains("has problems; nothing was changed:"),
        "{}",
        done.err
    );
    assert!(done.err.contains("air.fee_read = 3"), "{}", done.err);
    assert_eq!(exec.calls(), Vec::<String>::new());
}

#[test]
fn config_prints_every_setting_aso_and_the_components() {
    // Fails if a setting in SETTINGS, FSP or WALLET is missing from `aso config`.
    let w = World::new("config");
    let (done, _) = w.run(&["config"], &FakeSource::new(), &mut Scripted::new(false));
    assert_eq!(done.code, 0);
    for st in aso::config::SETTINGS {
        assert!(
            done.out
                .lines()
                .any(|l| l.starts_with(&format!("{} = ", st.key))),
            "{}",
            st.key
        );
    }
    for (k, _, _) in aso::config::FSP {
        assert!(
            done.out
                .lines()
                .any(|l| l.starts_with(&format!("fsp.{k} = "))),
            "fsp.{k}"
        );
    }
    for (k, _, _) in aso::config::WALLET {
        assert!(
            done.out
                .lines()
                .any(|l| l.starts_with(&format!("wallet.{k} = "))),
            "wallet.{k}"
        );
    }
    assert!(
        done.out
            .contains("service.log_max_size_mb = 50  [default; reloads live]"),
        "{}",
        done.out
    );
}

#[test]
fn start_brings_up_air_server_then_fsp_mint_and_waits_for_each() {
    // Fails if fsp-mint is started first, or "started" is said without a health check.
    let (w, src) = World::installed("start");
    let (done, exec) = w.run(&["start"], &src, &mut Scripted::new(false));
    assert_eq!(done.code, 0, "{}", done.out);
    let calls = exec.calls();
    assert!(
        at(&calls, "systemctl enable --now aso-air.service")
            < at(&calls, "systemctl enable --now aso-fsp.service")
    );
    assert!(
        done.out
            .contains("air_server: started, health ok\nfsp-mint: started, health ok"),
        "{}",
        done.out
    );

    let mut down = World::new("startdown");
    down.fsp = nobody();
    let src = down.source();
    down.install(
        &["--claim-psp", "--skip-backup"],
        &src,
        &mut Scripted::new(false),
    );
    let (done, _) = down.run(&["start"], &src, &mut Scripted::new(false));
    assert_eq!(done.code, 1, "{}", done.out);
    assert!(
        done.out
            .contains("fsp-mint was started but its health is not ok after 1 s"),
        "{}",
        done.out
    );
}

#[test]
fn stop_asks_first_and_takes_fsp_mint_down_before_air_server() {
    // Fails if stop acts without a yes, or stops air_server first.
    let (w, src) = World::installed("stop");
    let (done, exec) = w.run(&["stop"], &src, &mut Scripted::new(false));
    assert!(
        done.out.contains("not confirmed: nothing was stopped"),
        "{}",
        done.out
    );
    assert_eq!(stops(&exec.calls()), Vec::<String>::new());
    let (done, exec) = w.run(&["stop", "--yes"], &src, &mut Scripted::new(false));
    assert_eq!(done.code, 0, "{}", done.out);
    assert_eq!(
        stops(&exec.calls()),
        vec![
            "systemctl disable --now aso-fsp.service",
            "systemctl disable --now aso-air.service"
        ]
    );
    let mut ask = Scripted::new(true).saying(&[true]);
    let (_, exec) = w.run(&["stop"], &src, &mut ask);
    assert_eq!(stops(&exec.calls()).len(), 2);
    assert_eq!(ask.asked, vec!["Stop fsp-mint and air_server?"]);
}

#[test]
fn restart_names_the_downtime_asks_and_restarts_only_what_was_named() {
    // Fails if restart acts without a yes, restarts both when one was named, or
    // leaves out why it is a downtime.
    let (w, src) = World::installed("restart");
    let (done, exec) = w.run(&["restart", "fsp-mint"], &src, &mut Scripted::new(false));
    assert!(
        done.out
            .contains("fsp-mint: BRIEF DOWNTIME: fsp-mint is stopped and the new one started"),
        "{}",
        done.out
    );
    assert!(
        done.out.contains("not confirmed: nothing was restarted"),
        "{}",
        done.out
    );
    assert_eq!(stops(&exec.calls()), Vec::<String>::new());
    let (done, exec) = w.run(
        &["restart", "fsp-mint", "--yes"],
        &src,
        &mut Scripted::new(false),
    );
    assert_eq!(done.code, 0, "{}", done.out);
    assert_eq!(
        stops(&exec.calls()),
        vec!["systemctl restart aso-fsp.service"]
    );
    assert!(
        done.out
            .contains("fsp-mint: restarted, health ok (back after "),
        "{}",
        done.out
    );
    let (done, exec) = w.run(&["restart", "--yes"], &src, &mut Scripted::new(false));
    assert_eq!(
        stops(&exec.calls()),
        vec![
            "systemctl restart aso-air.service",
            "systemctl restart aso-fsp.service"
        ],
        "{}",
        done.out
    );
    let (done, _) = w.run(
        &["restart", "ecash-wallet", "--yes"],
        &src,
        &mut Scripted::new(false),
    );
    assert_eq!(done.code, 1);
    assert!(
        done.out
            .contains("ecash-wallet is not a service: there is nothing to restart"),
        "{}",
        done.out
    );
}

#[test]
fn reload_rewrites_the_files_and_signals_fsp_mint_without_stopping_anything() {
    // Fails if reload does not write the changed setting, restarts, or does not
    // signal fsp-mint.
    let (w, src) = World::installed("reload");
    let text = format!("{}[fsp.issue]\nmax_items = 5\n", w.text());
    let exec = w.exec();
    let done = cli(
        &w.dir,
        &text,
        &["reload"],
        &exec,
        &src,
        &mut Scripted::new(false),
    );
    assert_eq!(done.code, 0, "{}", done.out);
    assert!(
        w.read("etc/fsp-mint.toml")
            .contains("[issue]\nmax_items = 5\n")
    );
    assert!(
        done.out
            .contains(&format!("wrote {}/etc/fsp-mint.toml", w.dir.display())),
        "{}",
        done.out
    );
    assert!(
        done.out
            .contains("fsp-mint: told to read its settings again (nothing was stopped)"),
        "{}",
        done.out
    );
    let calls = exec.calls();
    assert!(
        calls.contains(&"systemctl reload aso-fsp.service".to_string()),
        "{calls:#?}"
    );
    assert_eq!(stops(&calls), Vec::<String>::new());
    assert!(!done.out.contains("WAITING FOR A RESTART"), "{}", done.out);
}

#[test]
fn a_setting_that_needs_a_restart_is_named_until_the_restart() {
    // Fails if reload hides a restart-only change, or keeps naming it after the restart.
    let (w, src) = World::installed("waiting");
    let text = format!("{}[fsp.rotation]\nperiod_hours = 5\n", w.text());
    let reload = |text: &str| {
        cli(
            &w.dir,
            text,
            &["reload"],
            &w.exec(),
            &src,
            &mut Scripted::new(false),
        )
    };
    let first = reload(&text);
    assert!(first.out.contains("WAITING FOR A RESTART: fsp.rotation.period_hours was 8760 when the service started, aso.toml says 5: `aso restart` applies it"), "{}", first.out);
    let done = cli(
        &w.dir,
        &text,
        &["restart", "fsp-mint", "--yes"],
        &w.exec(),
        &src,
        &mut Scripted::new(false),
    );
    assert_eq!(done.code, 0, "{}", done.out);
    assert!(!reload(&text).out.contains("WAITING FOR A RESTART"));
}

#[test]
fn the_redeem_job_follows_redeem_payout_on_reload() {
    // Fails if setting the payout writes no job, the interval is not used, or
    // clearing the payout leaves the job behind.
    let (w, src) = World::installed("job");
    let on = format!(
        "{}[redeem]\npayout = \"btc:bc1qexample\"\ninterval_hours = 6\n",
        w.text()
    );
    let exec = w.exec();
    let done = cli(
        &w.dir,
        &on,
        &["reload"],
        &exec,
        &src,
        &mut Scripted::new(false),
    );
    assert_eq!(done.code, 0, "{}", done.out);
    assert!(
        w.read("units/aso-redeem.timer")
            .contains("OnUnitActiveSec=6h\n"),
        "{}",
        w.read("units/aso-redeem.timer")
    );
    assert!(
        w.read("units/aso-redeem.service").contains("--dir "),
        "{}",
        w.read("units/aso-redeem.service")
    );
    assert!(w.read("units/aso-redeem.service").contains("redeem run"));
    let calls = exec.calls();
    assert!(
        calls.contains(&"systemctl enable --now aso-redeem.timer".to_string()),
        "{calls:#?}"
    );
    assert!(
        done.out.contains("automatic redeem: on, every 6 h"),
        "{}",
        done.out
    );
    assert_eq!(stops(&calls), Vec::<String>::new());

    let exec = w.exec();
    let done = cli(
        &w.dir,
        &w.text(),
        &["reload"],
        &exec,
        &src,
        &mut Scripted::new(false),
    );
    assert!(
        done.out
            .contains("automatic redeem: off (redeem.payout is not set); its job was removed"),
        "{}",
        done.out
    );
    assert!(!w.dir.join("units/aso-redeem.timer").exists());
    assert!(
        exec.calls()
            .contains(&"systemctl disable --now aso-redeem.timer".to_string())
    );
}

#[test]
fn audit_wraps_the_mint_audit_and_the_balance_and_exits_non_zero_on_a_failed_audit() {
    // Fails if the balance is left out, the verdict is ignored, or audit stops anything.
    let (w, src) = World::installed("audit");
    let (done, exec) = w.run(&["audit"], &src, &mut Scripted::new(false));
    assert_eq!(done.code, 0, "{}", done.out);
    for line in [
        "== mint audit (fsp-mint audit) ==",
        "issued 10",
        "mint audit: consistent",
        "== wallet balance (ecash-wallet balance) ==",
        "psp 100",
    ] {
        assert!(done.out.lines().any(|l| l == line), "{line}: {}", done.out);
    }
    assert_eq!(stops(&exec.calls()), Vec::<String>::new());

    let inner = w.exec();
    let bad = fake(move |bin, args| {
        if args.iter().any(|a| a == "--json") {
            ran(
                0,
                r#"{"consistent":false,"warnings":["spent 5 > issued 4"]}"#,
                "",
            )
        } else {
            use aso::exec::Exec;
            inner.run(bin, args, &[])
        }
    });
    let done = cli(
        &w.dir,
        &w.text(),
        &["audit"],
        &bad,
        &src,
        &mut Scripted::new(false),
    );
    assert_eq!(done.code, 1, "{}", done.out);
    assert!(
        done.out
            .contains("FAIL: the mint audit is not consistent: spent 5 > issued 4"),
        "{}",
        done.out
    );
    assert!(
        done.out.contains("psp 100"),
        "the balance is still shown: {}",
        done.out
    );
}

#[test]
fn versions_says_where_each_version_came_from_or_why_it_is_unknown() {
    // Fails if a version is guessed, the way it was read is not said, or an
    // unreadable one is not "unknown" with a reason.
    let (w, src) = World::installed("versions");
    let (done, _) = w.run(&["versions"], &src, &mut Scripted::new(false));
    assert_eq!(done.code, 0);
    let line = |name: &str| {
        done.out
            .lines()
            .find(|l| l.split_whitespace().next() == Some(name))
            .unwrap_or_else(|| panic!("{name}: {}", done.out))
            .to_string()
    };
    assert!(
        line("fsp-mint").contains("0.8.0")
            && line("fsp-mint")
                .contains("recorded by aso for this exact binary (fsp-mint prints no version)")
    );
    assert!(
        line("air_server").contains("1.0.1")
            && line("air_server")
                .contains("recorded by aso for this exact binary (air_server prints no version)")
    );
    assert!(line("ecash-wallet").contains("0.7.0"));
    assert!(line("ecash-wallet/sp-wire").contains("0.5.1"));
    assert!(
        line("fsp-mint/sp-wire").contains("unknown")
            && line("fsp-mint/sp-wire").contains("fsp-mint prints no version")
    );
    fs::remove_file(w.dir.join("installed/fsp-mint.json")).unwrap();
    let (done, _) = w.run(&["versions"], &src, &mut Scripted::new(false));
    let l = done
        .out
        .lines()
        .find(|l| l.split_whitespace().next() == Some("fsp-mint"))
        .unwrap();
    assert!(
        l.contains("unknown") && l.contains("aso has no record of installing it"),
        "{l}"
    );
}

#[test]
fn only_stop_restart_upgrade_and_install_can_stop_a_service() {
    // Fails if status, audit, versions, config, reload, upgrade check/status,
    // redeem run, start or logs rotate ever issue a stop, disable or restart.
    let (w, src) = World::installed("nostop");
    let mut all = Vec::new();
    for args in [
        vec!["status"],
        vec!["audit"],
        vec!["versions"],
        vec!["config"],
        vec!["reload"],
        vec!["start"],
        vec!["upgrade", "check"],
        vec!["upgrade", "status"],
        vec!["redeem", "run"],
    ] {
        let (_, exec) = w.run(&args, &src, &mut Scripted::new(false));
        all.extend(exec.calls());
    }
    assert_eq!(stops(&all), Vec::<String>::new(), "{all:#?}");
    assert!(all.len() > 10);
}
