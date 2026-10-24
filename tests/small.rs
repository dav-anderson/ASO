//! The small pieces: editing aso.toml, release tags and pins, schedule times,
//! and that the README documents every setting.

mod common;

use aso::edit::set_value;
use aso::release::{is_newer, newest, without_pin_line};
use aso::schedule::{parse_at, parse_duration, span_text};

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
fn release_tags_compare_as_numbers_and_ignore_what_is_not_a_release() {
    // Fails if 0.7.10 sorts below 0.7.9, or a release candidate or a word can be newest.
    let tags: Vec<String> = ["v0.9.0-rc1", "v0.7.10", "v0.7.9", "nightly", "0.8"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(newest(&tags), Some("0.7.10".to_string()));
    assert!(is_newer("0.10.0", "0.9.0"));
    assert!(!is_newer("0.9.0", "0.9.0"));
    assert!(!is_newer("0.9.0-rc1", "0.8.0"));
    assert_eq!(newest(&["v1.0.0-rc2".to_string()]), None);
}

#[test]
fn a_pin_line_is_removed_from_its_own_section_only() {
    // Fails if the version line of another section goes, or a comment is lost.
    let text = "# top\n[air]\nversion = \"1.0.1\"\n[fsp]\n# pinned for the audit\nversion = \"0.7.0\"\nbin = \"/x\"\n";
    assert_eq!(
        without_pin_line(text, "fsp").unwrap(),
        "# top\n[air]\nversion = \"1.0.1\"\n[fsp]\n# pinned for the audit\nbin = \"/x\"\n"
    );
    assert_eq!(without_pin_line("[fsp]\nbin = \"/x\"\n", "fsp"), None);
}

#[test]
fn schedule_times_and_durations_are_read_like_spi_reads_them() {
    // Fails if a time or duration that spi accepts is refused, or the other way.
    assert_eq!(parse_at("3:00"), Ok((3, 0)));
    assert_eq!(parse_at("23:59"), Ok((23, 59)));
    assert!(parse_at("24:00").is_err() && parse_at("03:5").is_err() && parse_at("0300").is_err());
    assert_eq!(parse_duration("1h30m"), Ok(5400));
    assert_eq!(parse_duration("15h28m"), Ok(55680));
    assert_eq!(parse_duration("1d"), Ok(86_400));
    assert!(
        parse_duration("59m59").is_err()
            && parse_duration("30s").is_err()
            && parse_duration("32d").is_err()
            && parse_duration("0m").is_err()
    );
    assert_eq!(span_text(55_680), "15h 28m");
    assert_eq!(span_text(90_000), "1d 1h");
    assert_eq!(span_text(30), "0m");
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
