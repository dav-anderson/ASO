//! `aso install`: the four steps, the smart rescan, and `aso status`.

mod common;

use std::fs;

use aso::exec::Exec;
use common::*;

const CLAIM: [&str; 2] = ["--claim-psp", "--skip-backup"];

#[test]
fn a_full_install_goes_a_b_c_in_order_and_ends_ready() {
    // Fails if a step runs out of order (fsp-mint before air_server is healthy,
    // the key after the service starts), a file is not written, or "ready" is
    // printed while the wallet is empty.
    let w = World::new("full");
    let src = w.source();
    let (done, exec) = w.install(&CLAIM, &src, &mut Scripted::new(false));
    assert_eq!(done.code, 0, "{}", done.out);
    let calls = exec.calls();
    let air = at(&calls, "systemctl enable --now aso-air.service");
    let key = at(
        &calls,
        &format!("{}/bin/fsp-mint service-key", w.dir.display()),
    );
    let fsp = at(&calls, "systemctl enable --now aso-fsp.service");
    let claim = at(&calls, &format!("{}/bin/ecash-wallet", w.dir.display()));
    assert!(air < key && key < fsp && fsp < claim, "{calls:#?}");
    assert!(
        done.out.starts_with("BACK UP THE DID KEY, OFFLINE:"),
        "{}",
        done.out
    );
    assert!(
        done.out
            .contains("ready: air_server and fsp-mint are healthy and the wallet is funded"),
        "{}",
        done.out
    );
    for f in [
        "bin/air_server",
        "bin/fsp-mint",
        "bin/ecash-wallet",
        "etc/fsp-mint.toml",
        "etc/fsp-mint.env",
        "etc/air-server.env",
        "etc/ecash-wallet.toml",
        "units/aso-air.service",
        "units/aso-fsp.service",
    ] {
        assert!(w.dir.join(f).is_file(), "{f} was not written");
    }
    let toml = fs::read_to_string(w.dir.join("etc/fsp-mint.toml")).unwrap();
    assert!(
        toml.contains(&format!("did = \"{DID}\""))
            && toml.contains(&format!("server = \"{AIR_NAME}\"")),
        "{toml}"
    );
    let unit = fs::read_to_string(w.dir.join("units/aso-fsp.service")).unwrap();
    assert!(
        unit.contains("After=network-online.target aso-air.service"),
        "{unit}"
    );
    assert!(unit.contains("Restart=on-failure"), "{unit}");
}

#[test]
fn the_did_key_backup_is_the_operators_job_and_is_said_before_the_service_starts() {
    // Fails if the notice is dropped, moved after the first start, or aso copies the key.
    let w = World::new("notice");
    let (done, _) = w.install(&CLAIM, &w.source(), &mut Scripted::new(false));
    let key = w.dir.join("data/fsp/main/service_key.hex");
    let first = done.out.lines().next().unwrap();
    assert_eq!(
        first,
        format!(
            "BACK UP THE DID KEY, OFFLINE: {} (made by `fsp-mint service-key`).",
            key.display()
        )
    );
    assert!(done.out.contains("aso makes no copy of it"), "{}", done.out);
    let copies: Vec<_> = walk(&w.dir)
        .into_iter()
        .filter(|p| p.ends_with("service_key.hex"))
        .collect();
    assert_eq!(copies.len(), 1, "{copies:?}");
}

fn walk(dir: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    for e in fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p.display().to_string());
        }
    }
    out
}

#[test]
fn running_install_again_redoes_nothing_that_is_done() {
    // Fails if a second run restarts air_server, makes the key again or funds the
    // wallet again.
    let w = World::new("again");
    let src = w.source();
    let (first, _) = w.install(&CLAIM, &src, &mut Scripted::new(false));
    assert_eq!(first.code, 0, "{}", first.out);
    let (second, exec) = w.install(&["--skip-backup"], &src, &mut Scripted::new(false));
    assert_eq!(second.code, 0, "{}", second.out);
    let calls = exec.calls();
    assert!(
        calls.iter().all(|c| !c.contains("enable --now")
            && !c.contains("service-key")
            && !c.contains("claim-psp")),
        "{calls:#?}"
    );
    assert!(
        second
            .out
            .contains("a. AIR service (air_server behind nginx): done already, left as it is"),
        "{}",
        second.out
    );
    assert!(
        second
            .out
            .contains("c. wallet funded (ecash-wallet): done already, left as it is"),
        "{}",
        second.out
    );
}

#[test]
fn a_missing_piece_is_named_and_only_it_is_built() {
    // Fails if the rescan rebuilds step a for a missing fsp-mint binary, or says nothing.
    let w = World::new("rescan");
    let src = w.source();
    w.install(&CLAIM, &src, &mut Scripted::new(false));
    fs::remove_file(w.dir.join("bin/fsp-mint")).unwrap();
    let (done, exec) = w.install(&["--skip-backup"], &src, &mut Scripted::new(false));
    assert_eq!(done.code, 0, "{}", done.out);
    assert!(
        done.out
            .contains("b. FSP (fsp-mint serve): missing: fsp-mint is installed"),
        "{}",
        done.out
    );
    let calls = exec.calls();
    assert!(
        !calls
            .iter()
            .any(|c| c == "systemctl enable --now aso-air.service"),
        "{calls:#?}"
    );
    assert!(
        calls
            .iter()
            .any(|c| c == "systemctl enable --now aso-fsp.service"),
        "{calls:#?}"
    );
    assert!(w.dir.join("bin/fsp-mint").is_file());
}

#[test]
fn without_the_esp_mint_settings_fsp_is_not_started_and_the_message_names_them() {
    // Fails if fsp-mint is started without its three required settings.
    let w = World::new("noesp");
    let text = w
        .text()
        .replace("[fsp.air]", "[fsp.unused]")
        .replace("esp_did", "a")
        .replace("esp_server", "b")
        .replace("esp_host", "c");
    let exec = w.exec();
    let done = cli(
        &w.dir,
        &text,
        &["install", "--skip-backup"],
        &exec,
        &w.source(),
        &mut Scripted::new(false),
    );
    assert_eq!(done.code, 1, "{}", done.out);
    assert!(done.out.contains("fsp-mint serve refuses to start without fsp.air.esp_did, fsp.air.esp_server, fsp.air.esp_host"), "{}", done.out);
    assert!(
        !exec
            .calls()
            .iter()
            .any(|c| c.contains("aso-fsp.service") && c.contains("enable"))
    );
}

#[test]
fn an_air_server_that_never_answers_stops_the_install_before_fsp_mint() {
    // Fails if fsp-mint is started after an air_server whose health is not ok.
    let mut w = World::new("airdown");
    w.air = nobody();
    let (done, exec) = w.install(&CLAIM, &w.source(), &mut Scripted::new(false));
    assert_eq!(done.code, 1, "{}", done.out);
    assert!(
        done.out
            .contains("air_server was started but its health is not ok after 1 s"),
        "{}",
        done.out
    );
    assert!(
        !exec.calls().iter().any(|c| c.contains("service-key")),
        "{:#?}",
        exec.calls()
    );
}

#[test]
fn nginx_is_checked_before_air_server_starts_and_a_bad_file_stops_the_install() {
    // Fails if the nginx file is not written with the public name and the
    // forward to air_server, `nginx -t` runs after the start, or its failure is ignored.
    let w = World::new("nginx");
    let text = with_air(
        &w.text(),
        "public_name = \"aso.example.org\"\npublic_cert = \"/etc/ssl/aso/fullchain.pem\"\npublic_key = \"/etc/ssl/aso/privkey.pem\"",
    );
    let exec = w.exec();
    let done = cli(
        &w.dir,
        &text,
        &["install", "--claim-psp", "--skip-backup"],
        &exec,
        &w.source(),
        &mut Scripted::new(false),
    );
    assert_eq!(done.code, 0, "{}", done.out);
    let calls = exec.calls();
    assert!(at(&calls, "nginx -t") < at(&calls, "systemctl enable --now aso-air.service"));
    let conf = fs::read_to_string(w.dir.join("etc/nginx-air.conf")).unwrap();
    assert!(
        conf.contains("server_name aso.example.org;")
            && conf.contains("proxy_pass http://127.0.0.1:5702;"),
        "{conf}"
    );
    assert!(
        done.out.contains("aso does not reload nginx"),
        "{}",
        done.out
    );
    assert!(
        done.out.contains("Give wallets this entry for [air] fsp: "),
        "{}",
        done.out
    );
    assert!(
        done.out
            .contains(&format!("{DID}@{AIR_NAME}@aso.example.org")),
        "{}",
        done.out
    );

    let w = World::new("nginxbad");
    let text = with_air(
        &w.text(),
        "public_name = \"aso.example.org\"\npublic_cert = \"/etc/ssl/aso/fullchain.pem\"\npublic_key = \"/etc/ssl/aso/privkey.pem\"",
    );
    let inner = w.exec();
    let bad = fake(move |bin, args| {
        if bin == "nginx" {
            ran(1, "", "nginx: [emerg] bad directive")
        } else {
            inner.run(bin, args, &[])
        }
    });
    let done = cli(
        &w.dir,
        &text,
        &["install", "--skip-backup"],
        &bad,
        &w.source(),
        &mut Scripted::new(false),
    );
    assert_eq!(done.code, 1, "{}", done.out);
    assert!(
        done.out
            .contains("`nginx -t` failed: nginx: [emerg] bad directive"),
        "{}",
        done.out
    );
    assert!(
        !bad.calls()
            .iter()
            .any(|c| c.contains("enable --now aso-air")),
        "{:#?}",
        bad.calls()
    );
}

#[test]
fn the_wallet_is_funded_by_buy_then_claim_or_by_a_handoff() {
    // Fails if the funding options build the wrong wallet command lines, or
    // buy-psp says the wallet is funded.
    let w = World::new("buy");
    let (done, exec) = w.install(
        &["--buy-psp", "02dd", "--skip-backup"],
        &w.source(),
        &mut Scripted::new(false),
    );
    let wallet = format!(
        "{d}/bin/ecash-wallet --wallet {d}/data/wallet/wallet.json --config {d}/etc/ecash-wallet.toml",
        d = w.dir.display()
    );
    assert!(
        exec.calls()
            .contains(&format!("{wallet} buy-psp --mint-key 02dd")),
        "{:#?}",
        exec.calls()
    );
    assert!(done.out.contains("a1b2c3d4payload"), "{}", done.out);
    assert!(
        done.out.contains(
            "Pay that payload once (in an OP_RETURN), wait for its confirmations, then run `aso install --claim-psp`"
        ),
        "{}",
        done.out
    );
    assert_eq!(done.code, 0, "install did what it can: {}", done.out);
    assert!(
        done.out
            .contains("NOT ready: still to do: wallet funded (ecash-wallet)"),
        "{}",
        done.out
    );

    let w = World::new("handoff");
    let (done, exec) = w.install(
        &[
            "--handoff",
            "/tmp/h.txt",
            "--did-key-file",
            "/tmp/k.hex",
            "--skip-backup",
        ],
        &w.source(),
        &mut Scripted::new(false),
    );
    let wallet = format!(
        "{d}/bin/ecash-wallet --wallet {d}/data/wallet/wallet.json --config {d}/etc/ecash-wallet.toml",
        d = w.dir.display()
    );
    assert!(
        exec.calls().contains(&format!(
            "{wallet} handoff import /tmp/h.txt --did-key-file /tmp/k.hex"
        )),
        "{:#?}",
        exec.calls()
    );
    assert_eq!(done.code, 0, "{}", done.out);
    assert!(
        done.out.contains("the wallet is funded (PSP 80)"),
        "{}",
        done.out
    );
}

#[test]
fn step_d_buys_fsp_at_another_service_or_is_skipped_and_remembered() {
    // Fails if step d runs the wrong commands, or an answered step d is asked again.
    let w = World::new("backup");
    let src = w.source();
    let (done, exec) = w.install(
        &[
            "--claim-psp",
            "--backup-service",
            "02ee",
            "--backup-psp",
            "20",
        ],
        &src,
        &mut Scripted::new(false),
    );
    assert_eq!(done.code, 0, "{}", done.out);
    let wallet = format!(
        "{d}/bin/ecash-wallet --wallet {d}/data/wallet/wallet.json --config {d}/etc/ecash-wallet.toml",
        d = w.dir.display()
    );
    let calls = exec.calls();
    assert!(
        at(
            &calls,
            &format!("{wallet} exchange --service 02ee --psp 20")
        ) < at(&calls, &format!("{wallet} fsp issue --service 02ee"))
    );
    assert!(
        done.out
            .contains("[done]     d. FSP funded at another service, for backups (recommended)"),
        "{}",
        done.out
    );
    let (again, exec) = w.install(&[], &src, &mut Scripted::new(false));
    assert!(
        !exec.calls().iter().any(|c| c.contains("exchange")),
        "{:#?}",
        exec.calls()
    );
    assert!(!again.out.contains("step d (recommended)"), "{}", again.out);
}

#[test]
fn a_fresh_install_without_step_d_says_how_to_do_or_skip_it() {
    // Fails if the install is silent about the optional step d.
    let w = World::new("dhint");
    let (done, _) = w.install(&["--claim-psp"], &w.source(), &mut Scripted::new(false));
    assert!(
        done.out
            .contains("step d (recommended): `aso install --backup-service DID --backup-psp N`"),
        "{}",
        done.out
    );
    assert_eq!(
        done.code, 0,
        "step d does not hold the service back: {}",
        done.out
    );
}

#[test]
fn the_payout_asked_by_install_is_written_to_aso_toml_with_a_backup() {
    // Fails if the answer is not saved, the old file is not kept, or other lines are lost.
    let w = World::new("payout");
    let mut ask = Scripted::new(true);
    ask.lines.push_back(Some("btc:bc1qexample".to_string()));
    let (done, _) = w.install(&CLAIM, &w.source(), &mut ask);
    assert_eq!(done.code, 0, "{}", done.out);
    assert!(
        ask.asked
            .iter()
            .any(|q| q.starts_with("Where should the ESP this service earns be paid out")),
        "{:?}",
        ask.asked
    );
    let saved = fs::read_to_string(w.dir.join("aso.toml")).unwrap();
    assert!(
        saved.contains("[redeem]\npayout = \"btc:bc1qexample\"\n"),
        "{saved}"
    );
    assert!(saved.contains("[fsp.server]"), "{saved}");
    let backup = fs::read_to_string(w.dir.join(format!("aso.toml.{NOW}.bak"))).unwrap();
    assert_eq!(backup, w.text());
    assert!(
        done.out
            .contains("run `aso reload` to start the automatic redeem"),
        "{}",
        done.out
    );
}

#[test]
fn a_payout_set_before_the_install_loads_the_redeem_job() {
    // Fails if the redeem job is written by an early step (air_server's, the
    // first to write the files) and then never loaded because a later step
    // found its files already written.
    let w = World::new("prepaid");
    let text = format!("{}[redeem]\npayout = \"btc:bc1qexample\"\n", w.text());
    let exec = w.exec();
    let mut all = vec!["install"];
    all.extend(CLAIM);
    let done = cli(
        &w.dir,
        &text,
        &all,
        &exec,
        &w.source(),
        &mut Scripted::new(false),
    );
    assert_eq!(done.code, 0, "{}", done.out);
    assert!(w.dir.join("units/aso-redeem.timer").is_file());
    let calls = exec.calls();
    assert_eq!(
        calls
            .iter()
            .filter(|c| *c == "systemctl enable --now aso-redeem.timer")
            .count(),
        1,
        "{calls:#?}"
    );
}

#[test]
fn with_no_terminal_the_payout_is_not_asked_and_the_redeem_stays_off() {
    // Fails if the install blocks or guesses a payout when nobody can answer.
    let w = World::new("unpaid");
    let mut ask = Scripted::new(false);
    let (done, exec) = w.install(&CLAIM, &w.source(), &mut ask);
    assert!(
        done.out
            .contains("automatic redeem: off until redeem.payout is set in aso.toml"),
        "{}",
        done.out
    );
    assert!(
        !fs::read_to_string(w.dir.join("aso.toml"))
            .unwrap()
            .contains("payout")
    );
    assert!(!exec.calls().iter().any(|c| c.contains("aso-redeem")));
}

#[test]
fn status_shows_each_step_each_process_and_readiness() {
    // Fails if a step, a process line or the readiness verdict is missing, or
    // `ready` is claimed with an empty wallet.
    let w = World::new("status");
    let src = w.source();
    w.install(&CLAIM, &src, &mut Scripted::new(false));
    let exec = w.exec();
    let done = cli(
        &w.dir,
        &w.text(),
        &["status"],
        &exec,
        &src,
        &mut Scripted::new(false),
    );
    assert_eq!(done.code, 0, "{}", done.out);
    for step in [
        "a. AIR service",
        "b. FSP",
        "c. wallet funded",
        "d. FSP funded",
    ] {
        assert!(
            done.out.contains(&format!("[done]     {step}")),
            "{step}: {}",
            done.out
        );
    }
    assert!(
        done.out.lines().any(|l| l.starts_with("air_server")
            && l.contains("running")
            && l.contains("health ok")
            && l.ends_with("version 1.0.1")),
        "{}",
        done.out
    );
    assert!(
        done.out
            .lines()
            .any(|l| l.starts_with("fsp-mint") && l.ends_with("version 0.8.0")),
        "{}",
        done.out
    );
    assert!(
        done.out
            .contains("automatic redeem: off (redeem.payout is not set in aso.toml)"),
        "{}",
        done.out
    );
    w.psp.store(0, std::sync::atomic::Ordering::SeqCst);
    let done = cli(
        &w.dir,
        &w.text(),
        &["status"],
        &w.exec(),
        &src,
        &mut Scripted::new(false),
    );
    assert_eq!(done.code, 1, "{}", done.out);
    assert!(
        done.out
            .contains("[not done] c. wallet funded (ecash-wallet)"),
        "{}",
        done.out
    );
    assert!(
        done.out
            .contains("             missing: the wallet is funded (PSP 0)"),
        "{}",
        done.out
    );
}

#[test]
fn install_writes_a_starter_aso_toml_when_there_is_none() {
    // Fails if install needs the file to exist, or the starter sets anything.
    let dir = scratch("starter");
    let exec = quiet();
    let vars = |_: &str| None;
    let env = aso::cli::Env {
        os: "linux",
        home: "/home/test".into(),
        vars: &vars,
        now: NOW,
        exe: "/usr/local/bin/aso".into(),
        source: Some(&FakeSource::new()),
    };
    let args: Vec<String> = ["--dir", &dir.display().to_string(), "install"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let done = aso::cli::run(&args, &env, &exec, &mut Scripted::new(false));
    let text = fs::read_to_string(dir.join("aso.toml")).unwrap();
    assert_eq!(text, aso::cli::STARTER);
    assert!(
        text.lines().all(|l| l.is_empty() || l.starts_with('#')),
        "{text}"
    );
    assert!(
        done.out.starts_with("wrote a starter ") || done.out.contains("wrote a starter "),
        "{}",
        done.out
    );
}

#[test]
fn an_unfunded_wallet_is_told_the_funding_steps_in_the_new_words_and_amounts_are_refused() {
    // Fails if the hint still names AMOUNT, says to pay "ONCE" in capitals, or
    // `--buy-psp` still takes amounts (the PSP mint decides what a payment is owed).
    let w = World::new("fund-hint");
    let (done, _) = w.install(&["--skip-backup"], &w.source(), &mut Scripted::new(false));
    assert!(
        done.out.contains(
            "The wallet holds no PSP. Fund it with one of: `aso install --buy-psp MINT_KEY` (prints a payload to pay once), then `aso install --claim-psp` once it has its confirmations; or `aso install --handoff FILE --did-key-file KEYFILE`"
        ),
        "{}",
        done.out
    );
    assert!(!done.out.contains("AMOUNT"), "{}", done.out);
    let (done, exec) = w.install(
        &["--buy-psp", "02dd", "64", "--skip-backup"],
        &w.source(),
        &mut Scripted::new(false),
    );
    assert_eq!(done.code, 2, "{done:?}");
    assert!(
        done.err.contains("`aso install` does not take `64`"),
        "{done:?}"
    );
    assert!(
        !exec.calls().iter().any(|c| c.contains("buy-psp")),
        "{:#?}",
        exec.calls()
    );
    let (done, _) = w.install(
        &["--buy-psp", "--skip-backup"],
        &w.source(),
        &mut Scripted::new(false),
    );
    assert_eq!(done.code, 2, "{done:?}");
    assert!(done.err.contains("--buy-psp needs MINT_KEY"), "{done:?}");
}
