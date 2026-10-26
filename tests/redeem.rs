//! The automatic ESP redeem: export-esp, receive, redeem, collect, resume, and
//! that a second run hands out nothing twice. The programs are a model of the
//! contract between fsp-mint (item 13: `export-esp` through the running serve
//! gives each kept note once) and ecash-wallet 0.8.0.

mod common;

use std::fs;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use aso::exec::Exec;
use common::*;

/// The state of the model: notes fsp-mint has kept and not exported, notes the
/// wallet received, redemptions made, and what the wallet shows as unanswered.
#[derive(Default)]
struct Chain {
    kept: Mutex<Vec<String>>,
    in_wallet: Mutex<Vec<String>>,
    redemptions: Mutex<Vec<String>>,
    unanswered: Mutex<Vec<String>>,
    exports: AtomicUsize,
    receive_works: AtomicBool,
}

fn payout_text(w: &World) -> String {
    format!("{}[redeem]\npayout = \"btc:bc1qexample\"\n", w.text())
}

fn chain_exec(w: &World, chain: &Arc<Chain>) -> Fake {
    let inner = w.exec();
    let chain = Arc::clone(chain);
    fake(move |bin, args| {
        let has = |x: &str| args.iter().any(|a| a == x);
        let after = |x: &str| {
            args.iter()
                .position(|a| a == x)
                .and_then(|i| args.get(i + 1))
                .cloned()
        };
        if bin.ends_with("fsp-mint") && has("export-esp") {
            // Without --serve-only, fsp-mint 0.8.0 does the export itself when
            // no serve answers: as root, in the service account's data folder.
            if !has("--serve-only") {
                return ran(1, "", "export-esp without --serve-only");
            }
            let out = std::path::PathBuf::from(after("--out").unwrap());
            let notes: Vec<String> = std::mem::take(&mut *chain.kept.lock().unwrap());
            if !notes.is_empty() {
                let n = chain.exports.fetch_add(1, Ordering::SeqCst) + 1;
                let file = out.join(format!("esp-export-{n}.json"));
                fs::write(&file, notes.join(",")).unwrap();
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
                }
            }
            return ran(0, "", "");
        }
        if bin.ends_with("ecash-wallet") && has("receive") {
            if !chain.receive_works.load(Ordering::SeqCst) {
                return ran(1, "", "cannot read the hand-over file");
            }
            let file = after("receive").unwrap();
            chain
                .in_wallet
                .lock()
                .unwrap()
                .push(fs::read_to_string(file).unwrap());
            return ran(0, "received\n", "");
        }
        if bin.ends_with("ecash-wallet") && has("redeem") && has("--resume") {
            let id = after("--resume").unwrap();
            chain.unanswered.lock().unwrap().retain(|u| *u != id);
            return ran(0, "answered\n", "");
        }
        if bin.ends_with("ecash-wallet") && has("redeem") && has("--collect") {
            return ran(0, "no redemption is waiting for a payout\n", "");
        }
        if bin.ends_with("ecash-wallet") && has("redeem") {
            let mut held = chain.in_wallet.lock().unwrap();
            if held.is_empty() {
                return ran(1, "", "the wallet holds no free ESP for that service");
            }
            let id = format!("r{}", chain.redemptions.lock().unwrap().len() + 1);
            chain.redemptions.lock().unwrap().push(held.join("|"));
            held.clear();
            return ran(0, &format!("redemption {id} sent\n"), "");
        }
        if bin.ends_with("ecash-wallet") && args.last().map(String::as_str) == Some("balance") {
            let mut text = "psp 100\nesp 0\nfsp 0\n".to_string();
            for id in chain.unanswered.lock().unwrap().iter() {
                text.push_str(&format!("pending redemption {id} (not answered yet)\n"));
            }
            return ran(0, &text, "");
        }
        inner.run(bin, args, &[])
    })
}

fn setup(name: &str) -> (World, Arc<Chain>, FakeSource) {
    let (w, src) = World::installed(name);
    let chain = Arc::new(Chain::default());
    chain.receive_works.store(true, Ordering::SeqCst);
    (w, chain, src)
}

fn run(w: &World, chain: &Arc<Chain>, src: &FakeSource) -> (aso::cli::Done, Fake) {
    let exec = chain_exec(w, chain);
    let done = cli(
        &w.dir,
        &payout_text(w),
        &["redeem", "run"],
        &exec,
        src,
        &mut Scripted::new(false),
    );
    (done, exec)
}

#[test]
fn the_chain_runs_end_to_end_with_fsp_mint_serving() {
    // Fails if any link is left out or out of order, the service key is typed on
    // a command line, the hand-over file is kept, or the hand-over folder is not owner-only.
    let (w, chain, src) = setup("chain");
    chain
        .kept
        .lock()
        .unwrap()
        .extend(["a".to_string(), "b".to_string()]);
    let (done, exec) = run(&w, &chain, &src);
    assert_eq!(done.code, 0, "{}", done.out);
    let calls = exec.calls();
    let bin = w.dir.join("bin");
    let wallet = format!(
        "{d}/ecash-wallet --wallet {r}/data/wallet/wallet.json --config {r}/etc/ecash-wallet.toml",
        d = bin.display(),
        r = w.dir.display()
    );
    let handover = w.dir.join("data/redeem/handover");
    let export = at(
        &calls,
        &format!(
            "{}/fsp-mint export-esp --out {} --serve-only",
            bin.display(),
            handover.display()
        ),
    );
    let receive = at(
        &calls,
        &format!("{wallet} receive {}/esp-export-1.json", handover.display()),
    );
    let key = w.dir.join("data/fsp/main/service_key.hex");
    let redeem = at(
        &calls,
        &format!(
            "{wallet} redeem --service-key-file {} --memo btc:bc1qexample",
            key.display()
        ),
    );
    let collect = at(&calls, &format!("{wallet} redeem --collect"));
    assert!(
        export < receive && receive < redeem && redeem < collect,
        "{calls:#?}"
    );
    assert_eq!(*chain.redemptions.lock().unwrap(), vec!["a,b".to_string()]);
    assert_eq!(
        fs::read_dir(&handover).unwrap().count(),
        0,
        "the hand-over file must be deleted after receive"
    );
    for l in [
        "export-esp: done",
        "redeem: sent",
        "collect: no payout is waiting",
    ] {
        assert!(done.out.lines().any(|x| x == l), "{l}: {}", done.out);
    }
    assert!(done.out.contains("taken in and deleted"), "{}", done.out);
    assert!(
        calls.iter().all(|c| !c.contains(&"11".repeat(32))),
        "the key itself must never be on a command line"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&handover).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    assert!(w.read("logs/aso-redeem.log").contains("redeem: sent"));
}

#[test]
fn a_second_run_hands_out_and_redeems_nothing_twice() {
    // Fails if a run exports, receives or redeems what the first run already did.
    let (w, chain, src) = setup("twice");
    chain.kept.lock().unwrap().push("a".to_string());
    let (first, _) = run(&w, &chain, &src);
    assert_eq!(first.code, 0, "{}", first.out);
    let (second, exec) = run(&w, &chain, &src);
    assert_eq!(second.code, 0, "{}", second.out);
    assert!(
        second
            .out
            .lines()
            .any(|l| l == "redeem: nothing has matured yet"),
        "{}",
        second.out
    );
    assert!(
        !exec.calls().iter().any(|c| c.contains(" receive ")),
        "{:#?}",
        exec.calls()
    );
    assert_eq!(chain.exports.load(Ordering::SeqCst), 1);
    assert_eq!(chain.redemptions.lock().unwrap().len(), 1);
    assert_eq!(chain.in_wallet.lock().unwrap().len(), 0);
}

#[test]
fn a_hand_over_file_that_fails_to_receive_is_kept_and_tried_again_not_exported_again() {
    // Fails if a failed receive deletes the file (the notes are lost) or the
    // next run exports the same notes again.
    let (w, chain, src) = setup("retry");
    chain.kept.lock().unwrap().push("a".to_string());
    chain.receive_works.store(false, Ordering::SeqCst);
    let (first, _) = run(&w, &chain, &src);
    assert_eq!(first.code, 1, "{}", first.out);
    assert!(first.out.contains("FAIL: receive "), "{}", first.out);
    assert!(
        first
            .out
            .contains("(the file is kept and tried again next run)"),
        "{}",
        first.out
    );
    assert!(
        w.dir
            .join("data/redeem/handover/esp-export-1.json")
            .is_file()
    );
    chain.receive_works.store(true, Ordering::SeqCst);
    let (second, _) = run(&w, &chain, &src);
    assert_eq!(second.code, 0, "{}", second.out);
    assert_eq!(chain.exports.load(Ordering::SeqCst), 1);
    assert_eq!(*chain.redemptions.lock().unwrap(), vec!["a".to_string()]);
    assert!(
        !w.dir
            .join("data/redeem/handover/esp-export-1.json")
            .exists()
    );
}

#[test]
fn a_redemption_the_wallet_shows_as_unanswered_is_sent_again_before_the_new_one() {
    // Fails if a lost reply is never resumed, or is resumed after a new redemption.
    let (w, chain, src) = setup("resume");
    chain.unanswered.lock().unwrap().push("r9".to_string());
    let (done, exec) = run(&w, &chain, &src);
    assert_eq!(done.code, 0, "{}", done.out);
    let calls = exec.calls();
    let key = w.dir.join("data/fsp/main/service_key.hex");
    let resume = at(
        &calls,
        &format!(
            "{}/bin/ecash-wallet --wallet {r}/data/wallet/wallet.json --config {r}/etc/ecash-wallet.toml redeem --service-key-file {} --resume r9",
            w.dir.display(),
            key.display(),
            r = w.dir.display()
        ),
    );
    assert!(
        resume
            < at(
                &calls,
                &format!(
                    "{}/bin/ecash-wallet --wallet {r}/data/wallet/wallet.json --config {r}/etc/ecash-wallet.toml redeem --service-key-file {} --memo",
                    w.dir.display(),
                    key.display(),
                    r = w.dir.display()
                )
            ),
        "{calls:#?}"
    );
    assert!(
        done.out.contains("redeem --resume r9: sent again"),
        "{}",
        done.out
    );
}

#[test]
fn aso_never_does_the_export_itself_when_fsp_mint_is_not_serving() {
    // Fails if export-esp is run with no serve (it would write fsp-mint's data
    // folder as root).
    let (mut w, chain, src) = setup("noserve");
    w.fsp = nobody();
    chain.kept.lock().unwrap().push("a".to_string());
    let (done, exec) = run(&w, &chain, &src);
    assert_eq!(done.code, 1, "{}", done.out);
    assert!(
        done.out
            .contains("export-esp skipped: fsp-mint is not answering its health check"),
        "{}",
        done.out
    );
    assert!(
        !exec.calls().iter().any(|c| c.contains("export-esp")),
        "{:#?}",
        exec.calls()
    );
    assert_eq!(chain.kept.lock().unwrap().len(), 1);
}

#[test]
fn an_fsp_mint_older_than_0_8_0_or_one_aso_has_no_record_of_is_not_asked_to_export() {
    // Fails if aso asks an fsp-mint that has no `export-esp --serve-only` (it
    // would do the export itself, as root, when serve is not answering).
    let (w, chain, src) = setup("oldfsp");
    chain.kept.lock().unwrap().push("a".to_string());
    let binary = w.dir.join("bin/fsp-mint");
    aso::state::record_installed(&w.aso(), aso::program::Program::Fsp, "0.7.0", &binary).unwrap();
    let (done, exec) = run(&w, &chain, &src);
    assert_eq!(done.code, 1, "{}", done.out);
    assert!(
        done.out.contains(
            "export-esp skipped: the installed fsp-mint is 0.7.0, and aso needs 0.8.0 or later (`export-esp --serve-only`)"
        ),
        "{}",
        done.out
    );
    assert!(
        !exec.calls().iter().any(|c| c.contains("export-esp")),
        "{:#?}",
        exec.calls()
    );

    fs::write(&binary, "a binary aso did not install").unwrap();
    let (done, exec) = run(&w, &chain, &src);
    assert_eq!(done.code, 1, "{}", done.out);
    assert!(
        done.out.contains(&format!(
            "export-esp skipped: aso has no record of {} (fsp-mint prints no version)",
            binary.display()
        )),
        "{}",
        done.out
    );
    assert!(!exec.calls().iter().any(|c| c.contains("export-esp")));
    assert_eq!(chain.kept.lock().unwrap().len(), 1);
}

#[test]
fn with_no_payout_there_is_no_automatic_redeem() {
    // Fails if the chain runs while redeem.payout is unset.
    let (w, _, src) = setup("nopayout");
    let exec = w.exec();
    let done = cli(
        &w.dir,
        &w.text(),
        &["redeem", "run"],
        &exec,
        &src,
        &mut Scripted::new(false),
    );
    assert_eq!(done.code, 0, "{}", done.out);
    assert!(
        done.out
            .contains("the automatic redeem is off: redeem.payout is not set in aso.toml"),
        "{}",
        done.out
    );
    assert!(
        !exec
            .calls()
            .iter()
            .any(|c| c.contains("export-esp") || c.contains("redeem"))
    );
}
