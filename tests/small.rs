//! The small pieces: editing aso.toml, and that the README documents every
//! setting. (Release tags, pins and schedule times are air-interface's tests.)

mod common;

use aso::edit::set_value;

#[test]
fn set_value_replaces_a_line_keeps_comments_and_adds_what_is_missing() {
    // Fails if the edit loses a comment, touches another section's key of the
    // same name, or puts the line in the wrong section.
    assert_eq!(
        set_value(
            "[redeem]\n# why\npayout = \"old\"\ninterval_hours = 6\n",
            "redeem",
            "payout",
            "btc:x"
        ),
        "[redeem]\n# why\npayout = \"btc:x\"\ninterval_hours = 6\n"
    );
    assert_eq!(
        set_value(
            "[redeem]\ninterval_hours = 6\n[air]\nbin = \"x\"\n",
            "redeem",
            "payout",
            "btc:x"
        ),
        "[redeem]\npayout = \"btc:x\"\ninterval_hours = 6\n[air]\nbin = \"x\"\n"
    );
    assert_eq!(
        set_value("[air]\nbin = \"x\"\n", "redeem", "payout", "btc:x"),
        "[air]\nbin = \"x\"\n\n[redeem]\npayout = \"btc:x\"\n"
    );
    assert_eq!(
        set_value("[air]\npayout = \"keep\"\n", "redeem", "payout", "btc:x"),
        "[air]\npayout = \"keep\"\n\n[redeem]\npayout = \"btc:x\"\n"
    );
    assert_eq!(
        set_value("", "redeem", "payout", "a\"b\\c"),
        "[redeem]\npayout = \"a\\\"b\\\\c\"\n"
    );
}

#[test]
fn the_readme_documents_every_setting_and_how_it_reloads() {
    // Fails if a setting is added to the tables and left out of the README, or
    // a row stops saying whether it reloads live.
    let readme = include_str!("../README.md");
    let row = |key: &str| {
        readme
            .lines()
            .find(|l| l.starts_with(&format!("| `{key}` |")))
            .unwrap_or_else(|| panic!("README has no row for `{key}`"))
            .to_string()
    };
    for st in aso::config::SETTINGS {
        let r = row(st.key);
        assert!(
            r.contains(&format!("| {} |", st.reload.words())),
            "{}: {r}",
            st.key
        );
    }
    for (k, default, live) in aso::config::FSP {
        let r = row(&format!("fsp.{k}"));
        assert!(
            r.contains(if *live {
                "| reloads live |"
            } else {
                "| needs a restart |"
            }),
            "fsp.{k}: {r}"
        );
        assert!(
            r.contains(&format!("`{default}`")) || default.is_empty(),
            "fsp.{k}: {r}"
        );
    }
    for (k, _, _) in aso::config::WALLET {
        assert!(row(&format!("wallet.{k}")).contains("| reloads live |"));
    }
}
