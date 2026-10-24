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
    // component's default, or the log caps stop being 50 MB and 4 files.
    let aso = load(&scratch("defaults"), "");
    assert_eq!(aso.text("fsp.server.bind"), "127.0.0.1:3200");
    assert_eq!(aso.text("fsp.server.network"), "main");
    assert_eq!(aso.whole("fsp.rotation.activation_hours"), 8760);
    assert_eq!(aso.whole("fsp.issue.max_items"), 1000);
    assert_eq!(aso.whole("fsp.air.held_reads"), 16);
    assert_eq!(aso.whole("wallet.air.attempts"), 3);
    assert_eq!(aso.whole("wallet.psp.min_purchase"), 50);
    assert_eq!(aso.whole("service.log_max_size_mb"), 50);
    assert_eq!(aso.whole("service.log_keep"), 4);
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
    for (section, version) in [("fsp", "0.9.0-rc1"), ("wallet", "0.8.0rc1")] {
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
        load(&scratch("pin"), "[fsp]\nversion = \"0.8.0\"\n").text("fsp.version"),
        "0.8.0"
    );
}

#[test]
fn a_pin_older_than_what_aso_needs_is_refused_and_air_version_is_only_a_note() {
    // Fails if aso can be pinned to an fsp-mint without `export-esp
    // --serve-only` or an ecash-wallet without `redeem --service-key-file`, or if
    // air.version (the operator's words for a binary with no releases) is
    // checked like a pin.
    for (section, version, needs) in [
        (
            "fsp",
            "0.7.0",
            "aso needs fsp-mint 0.8.0 or later (it uses `export-esp --serve-only`)",
        ),
        (
            "wallet",
            "0.6.1",
            "aso needs ecash-wallet 0.7.0 or later (it uses `redeem --service-key-file`)",
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
    let p = problems("[service]\nlog_keep = 0\n[air]\nbogus = 1\n[nowhere]\nx = 1\n");
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
