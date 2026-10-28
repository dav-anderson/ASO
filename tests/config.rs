//! aso.toml: the one file, where every value came from, what passes through,
//! and what is refused.

mod common;

use aso::generate;
use aso::state::State;
use common::*;

fn problems(text: &str) -> Vec<String> {
    try_load(&scratch("cfg"), Some(text), &[]).unwrap_err()
}

#[test]
fn defaults_are_the_components_own() {
    // Fails if a row of the FSP or WALLET table in config.rs drifts from the
    // component's default, or the log caps stop being 50 MB and 4 files, or the
    // release keep stops being 2.
    let aso = load(&scratch("defaults"), "");
    assert_eq!(aso.text("fsp.server.bind"), "127.0.0.1:3200");
    assert_eq!(aso.text("fsp.server.network"), "main");
    assert_eq!(aso.whole("fsp.rotation.activation_hours"), 8760);
    assert_eq!(aso.whole("fsp.issue.max_items"), 1000);
    assert_eq!(aso.whole("fsp.air.held_reads"), 16);
    assert_eq!(aso.whole("wallet.air.attempts"), 3);
    assert!(aso.entry("wallet.psp.min_purchase").is_none());
    assert_eq!(aso.whole("service.log_max_size_mb"), 50);
    assert_eq!(aso.whole("service.log_keep"), 4);
    assert_eq!(aso.whole("service.config_backups"), 5);
    assert_eq!(aso.whole("release.keep"), 2);
    assert_eq!(aso.text("air.health_addr"), "127.0.0.1:5701");
}

#[test]
fn config_names_where_each_value_came_from() {
    // Fails if `aso config` drops a setting, or names the wrong source for one
    // set in the file, one set in the environment, or one left at its default.
    let dir = scratch("sources");
    let aso = try_load(
        &dir,
        Some("[service]\nlog_keep = 9\n[fsp.server]\nbind = \"127.0.0.1:3300\"\n"),
        &[("ASO_SERVICE_LOG_MAX_SIZE_MB", "7")],
    )
    .unwrap();
    let lines = aso.describe();
    assert!(lines.contains(&"service.log_keep = 9  [aso.toml; reloads live]".to_string()));
    assert!(lines.contains(
        &"service.log_max_size_mb = 7  [env ASO_SERVICE_LOG_MAX_SIZE_MB; reloads live]".to_string()
    ));
    assert!(
        lines
            .contains(&"fsp.server.bind = 127.0.0.1:3300  [aso.toml; needs a restart]".to_string())
    );
    assert!(lines.contains(&"redeem.interval_hours = 24  [default; reloads live]".to_string()));
    assert!(lines.contains(&"fsp.issue.max_items = 1000  [default; reloads live]".to_string()));
}

#[test]
fn a_key_aso_does_not_name_reaches_fsp_mint_as_written() {
    // Fails if the generator drops, renames or reorders a setting aso has no
    // row for, or sets it from a default aso made up.
    let text = "[fsp.future]\nnew_key = \"x\"\n[fsp.rotation]\nactivation_hours = 100\n";
    let aso = load(&scratch("pass"), text);
    let toml = generate::fsp_toml(&aso, &State::default()).unwrap();
    assert!(toml.contains("[future]\nnew_key = \"x\"\n"), "{toml}");
    assert!(
        toml.contains("[rotation]\nactivation_hours = 100\n"),
        "{toml}"
    );
    assert!(
        !toml.contains("max_items"),
        "a default aso did not set must not be written: {toml}"
    );
    let described = aso.describe();
    assert!(
        described.contains(
            &"fsp.future.new_key = x  [aso.toml; passed through: the component says if it reloads]"
                .to_string()
        )
    );
}

#[test]
fn the_wallet_gets_its_own_keys_in_its_own_file() {
    // Fails if [wallet.*] settings land in fsp-mint's file or the wallet's is empty.
    let aso = load(
        &scratch("wallet"),
        "[wallet.air]\nattempts = 5\n[wallet.later]\nkey = \"v\"\n",
    );
    let wallet = generate::wallet_toml(&aso).unwrap();
    assert!(wallet.contains("[air]\nattempts = 5\n"), "{wallet}");
    assert!(wallet.contains("[later]\nkey = \"v\"\n"), "{wallet}");
    assert!(
        !generate::fsp_toml(&aso, &State::default())
            .unwrap()
            .contains("attempts")
    );
}

#[test]
fn the_generated_fsp_file_gets_the_did_and_the_air_server_from_aso() {
    // Fails if service.did, air.server or air.host are not injected, or an
    // operator's own value is overwritten.
    let aso = load(&scratch("inject"), "[fsp.air]\nhost = \"10.0.0.7\"\n");
    let state = State {
        air_name: AIR_NAME.into(),
        service_did: DID.into(),
        ..State::default()
    };
    let toml = generate::fsp_toml(&aso, &state).unwrap();
    assert!(toml.contains(&format!("did = \"{DID}\"")), "{toml}");
    assert!(toml.contains(&format!("server = \"{AIR_NAME}\"")), "{toml}");
    assert!(toml.contains("host = \"10.0.0.7\""), "{toml}");
    assert!(!toml.contains("127.0.0.1"), "{toml}");
}

#[test]
fn a_fee_other_than_zero_is_refused_naming_the_upstream_item() {
    // Fails if a non-zero fee loads, or the refusal stops naming upstream item 1.
    let p = problems("[air]\nfee_read = 5\n");
    assert_eq!(p.len(), 1);
    assert!(p[0].contains("air.fee_read = 5"), "{}", p[0]);
    assert!(p[0].contains("upstream item 1"), "{}", p[0]);
    let p = problems("[air]\nfee_write = 1\n");
    assert!(p[0].contains("air.fee_write = 1"), "{}", p[0]);
    assert!(
        load(&scratch("fee0"), "[air]\nfee_read = 0\nfee_write = 0\n").whole("air.fee_read") == 0
    );
}

#[test]
fn a_fee_set_in_the_environment_is_refused_too() {
    // Fails if the check runs on the file only.
    let e = try_load(&scratch("feeenv"), Some(""), &[("ASO_AIR_FEE_WRITE", "3")]).unwrap_err();
    assert!(e[0].contains("air.fee_write = 3"), "{}", e[0]);
}

#[test]
fn a_pin_on_a_release_candidate_is_refused_when_the_file_loads() {
    // Fails if an rc or a pre-release pin loads for fsp-mint or ecash-wallet.
    for (section, version) in [("fsp", "0.11.0-rc1"), ("wallet", "0.9.0rc1")] {
        let p = problems(&format!("[{section}]\nversion = \"{version}\"\n"));
        assert_eq!(p.len(), 1, "{p:?}");
        assert!(
            p[0].contains(&format!("{section}.version is `{version}`")),
            "{}",
            p[0]
        );
        assert!(p[0].contains("never installed"), "{}", p[0]);
    }
    assert_eq!(
        load(&scratch("pin"), "[fsp]\nversion = \"0.10.0\"\n").text("fsp.version"),
        "0.10.0"
    );
}

#[test]
fn a_pin_older_than_what_aso_needs_is_refused_and_air_version_is_only_a_note() {
    // Fails if aso can be pinned to an fsp-mint older than 0.10.0 or an
    // ecash-wallet older than 0.9.0 (the first releases on wire version 5, with
    // the shared keyset ID and the key check), or if air.version (the operator's
    // words for a binary with no releases) is checked like a pin.
    for (section, version, needs) in [
        (
            "fsp",
            "0.9.9",
            "aso needs fsp-mint 0.10.0 or later (it needs wire version 5 and the keys it publishes)",
        ),
        (
            "fsp",
            "0.8.0",
            "aso needs fsp-mint 0.10.0 or later (it needs wire version 5 and the keys it publishes)",
        ),
        (
            "wallet",
            "0.8.9",
            "aso needs ecash-wallet 0.9.0 or later (it needs wire version 5 and the key check)",
        ),
        (
            "wallet",
            "0.8.0",
            "aso needs ecash-wallet 0.9.0 or later (it needs wire version 5 and the key check)",
        ),
    ] {
        let p = problems(&format!("[{section}]\nversion = \"{version}\"\n"));
        assert_eq!(p.len(), 1, "{p:?}");
        assert!(
            p[0].contains(&format!("{section}.version is `{version}`: {needs}")),
            "{}",
            p[0]
        );
    }
    assert_eq!(
        load(
            &scratch("airnote"),
            "[air]\nversion = \"1.0.2-beta, built from main\"\n"
        )
        .text("air.version"),
        "1.0.2-beta, built from main"
    );
}

#[test]
fn every_problem_is_listed_and_a_bad_file_loads_nothing() {
    // Fails if loading stops at the first problem, or a file with problems loads.
    let p = problems("[service]\nlog_keep = 53\n[air]\nbogus = 1\n[nowhere]\nx = 1\n");
    assert_eq!(p.len(), 3, "{p:?}");
    let p = problems("[air]\nfee_read = 2\n[fsp.server]\nbind = \"0.0.0.0:1\"\n");
    assert_eq!(p.len(), 2, "{p:?}");
}

#[test]
fn the_fsp_bind_must_be_loopback() {
    // Fails if the mint can be bound to a public address through aso.
    let p = problems("[fsp.server]\nbind = \"0.0.0.0:3200\"\n");
    assert!(p[0].contains("not a loopback address"), "{}", p[0]);
}

#[test]
fn settings_that_need_a_restart_are_told_from_the_ones_that_do_not() {
    // Fails if restart_changes names a live setting, or misses a changed one.
    let dir = scratch("r1");
    let before = load(&dir, "[fsp.server]\nbind = \"127.0.0.1:3300\"\n");
    let applied = before.restart_values();
    let after = load(
        &dir,
        "[fsp.server]\nbind = \"127.0.0.1:3400\"\n[service]\nlog_keep = 9\n[redeem]\ninterval_hours = 6\n",
    );
    let changes = after.restart_changes(&applied);
    assert_eq!(
        changes,
        vec![(
            "fsp.server.bind".to_string(),
            "127.0.0.1:3300".to_string(),
            "127.0.0.1:3400".to_string()
        )]
    );
}

#[test]
fn zero_keeps_everything_for_every_keep_setting_in_the_file_and_the_environment() {
    // Fails if 0 is refused for any of the four (the old ranges started at 1), or
    // the environment variable and the file disagree about it.
    for (key, var) in [
        ("service.log_keep", "ASO_SERVICE_LOG_KEEP"),
        ("service.log_max_size_mb", "ASO_SERVICE_LOG_MAX_SIZE_MB"),
        ("service.config_backups", "ASO_SERVICE_CONFIG_BACKUPS"),
        ("release.keep", "ASO_RELEASE_KEEP"),
    ] {
        let (section, name) = key.split_once('.').unwrap();
        let aso = load(&scratch("zero-file"), &format!("[{section}]\n{name} = 0\n"));
        assert_eq!(aso.whole(key), 0, "{key} from the file");
        let aso = try_load(&scratch("zero-env"), None, &[(var, "0")]).unwrap();
        assert_eq!(aso.whole(key), 0, "{key} from {var}");
        assert!(
            aso.describe()
                .contains(&format!("{key} = 0  [env {var}; reloads live]")),
            "{key}"
        );
    }
}

#[test]
fn the_upper_bounds_of_the_keep_settings_stand_and_a_negative_number_is_refused() {
    // Fails if an upper bound moves (52, 10240, 100, 20) or 0 opens the range to
    // a negative number.
    for (file, message) in [
        (
            "[service]\nlog_keep = 53\n",
            "service.log_keep = 53 must be 0 to 52",
        ),
        (
            "[service]\nlog_max_size_mb = 10241\n",
            "service.log_max_size_mb = 10241 must be 0 to 10240",
        ),
        (
            "[service]\nconfig_backups = 101\n",
            "service.config_backups = 101 must be 0 to 100",
        ),
        (
            "[release]\nkeep = 21\n",
            "release.keep = 21 must be 0 to 20",
        ),
        (
            "[service]\nlog_keep = -1\n",
            "service.log_keep = -1 must not be negative",
        ),
    ] {
        assert_eq!(problems(file), vec![message.to_string()], "{file}");
    }
    let p = try_load(
        &scratch("bound-env"),
        None,
        &[("ASO_SERVICE_LOG_KEEP", "53")],
    )
    .unwrap_err();
    assert_eq!(
        p,
        vec![
            "service.log_keep = 53 must be 0 to 52 (from the environment variable ASO_SERVICE_LOG_KEEP)"
                .to_string()
        ]
    );
    // The edges that stay.
    let aso = load(
        &scratch("edges"),
        "[service]\nlog_keep = 52\nlog_max_size_mb = 10240\nconfig_backups = 100\n[release]\nkeep = 20\n",
    );
    assert_eq!(aso.whole("service.log_keep"), 52);
    assert_eq!(aso.whole("service.log_max_size_mb"), 10_240);
    assert_eq!(aso.whole("service.config_backups"), 100);
    assert_eq!(aso.whole("release.keep"), 20);
}

#[test]
fn the_wallet_min_purchase_setting_is_refused_as_removed() {
    // Fails if the key passes through to ecash-wallet (which would then stop at
    // its own start) or aso accepts it without a word. Nothing enforces a minimum.
    assert_eq!(
        problems("[wallet.psp]\nmin_purchase = 100\n"),
        vec!["wallet.psp.min_purchase was removed in aso 0.3.0: buy-psp takes no amounts, so there is no purchase size to check. We recommend a PSP purchase of at least $5 (50 PSP): the PSP mint's fees come out of what a payment buys, so a smaller payment can buy little or nothing. Nothing enforces it. Remove it".to_string()]
    );
    // Another wallet setting still passes through.
    let aso = load(&scratch("wallet-pass"), "[wallet.psp]\nfuture = 1\n");
    assert_eq!(aso.whole("wallet.psp.future"), 1);
}

#[test]
fn the_wallet_unpaid_key_days_default_to_90_and_reach_the_wallet() {
    // Fails if wallet.psp.unpaid_key_days drifts from ecash-wallet's default (90),
    // is not live, or a value set in aso.toml (0 keeps an unpaid purchase for
    // ever) does not reach the wallet's own file.
    let aso = load(&scratch("unpaid-default"), "");
    assert_eq!(aso.whole("wallet.psp.unpaid_key_days"), 90);
    assert!(
        aso.describe()
            .contains(&"wallet.psp.unpaid_key_days = 90  [default; reloads live]".to_string())
    );
    let aso = load(
        &scratch("unpaid-set"),
        "[wallet.psp]\nunpaid_key_days = 0\n",
    );
    assert_eq!(aso.whole("wallet.psp.unpaid_key_days"), 0);
    let wallet = generate::wallet_toml(&aso).unwrap();
    assert!(wallet.contains("[psp]\nunpaid_key_days = 0\n"), "{wallet}");
}

#[test]
fn the_pins_at_the_new_minimums_load_and_the_ones_just_below_do_not() {
    // Fails if the minimums are not exactly fsp-mint 0.10.0 and ecash-wallet 0.9.0
    // (an off-by-one in the comparison would load 0.9.9 or refuse 0.10.0).
    for (section, ok, below) in [("fsp", "0.10.0", "0.9.99"), ("wallet", "0.9.0", "0.8.99")] {
        assert_eq!(
            load(
                &scratch("min-ok"),
                &format!("[{section}]\nversion = \"{ok}\"\n")
            )
            .text(&format!("{section}.version")),
            ok
        );
        let p = problems(&format!("[{section}]\nversion = \"{below}\"\n"));
        assert_eq!(p.len(), 1, "{p:?}");
        assert!(p[0].contains("or later"), "{}", p[0]);
    }
    let (min, _) = aso::program::Program::Fsp.min_version().unwrap();
    assert_eq!(min, "0.10.0");
    let (min, _) = aso::program::Program::Wallet.min_version().unwrap();
    assert_eq!(min, "0.9.0");
}

#[test]
fn the_wallet_key_check_settings_default_off_and_1000_and_reach_the_wallet() {
    // Fails if wallet.fsp.blacklist_on_key_mismatch or wallet.fsp.blacklist_max
    // drift from ecash-wallet 0.9.0's defaults (false, 1000), are not live, are
    // missing from `aso config show`, or a value set in aso.toml (a TOML boolean
    // and a number) does not reach the wallet's own file as written.
    let aso = load(&scratch("blacklist-default"), "");
    let shown = aso.describe();
    assert!(
        shown.contains(
            &"wallet.fsp.blacklist_on_key_mismatch = false  [default; reloads live]".to_string()
        ),
        "{shown:?}"
    );
    assert!(
        shown.contains(&"wallet.fsp.blacklist_max = 1000  [default; reloads live]".to_string()),
        "{shown:?}"
    );
    let wallet = generate::wallet_toml(&aso).unwrap();
    assert!(
        !wallet.contains("blacklist"),
        "a default is not written: {wallet}"
    );
    let aso = load(
        &scratch("blacklist-set"),
        "[wallet.fsp]\nblacklist_on_key_mismatch = true\nblacklist_max = 5\n",
    );
    let wallet = generate::wallet_toml(&aso).unwrap();
    assert!(
        wallet.contains("[fsp]\nblacklist_max = 5\nblacklist_on_key_mismatch = true\n"),
        "{wallet}"
    );
}
