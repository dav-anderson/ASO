//! The caps on every log aso writes or sends a child's output to, through the
//! command line and the generated units. (The rotation itself, `Log::open` and
//! the pruning are air-minterface's tests.)

mod common;

use common::*;
use std::fs;

fn big(path: &std::path::Path, bytes: usize) {
    fs::write(path, vec![b'x'; bytes]).unwrap();
}

#[test]
fn the_logs_command_rotates_with_the_caps_from_the_environment() {
    // Fails if `aso logs rotate` ignores ASO_SERVICE_LOG_* or needs an aso.toml.
    let dir = scratch("cmd");
    let log = dir.join("fsp-mint.log");
    big(&log, 2 * 1024 * 1024);
    let vars = |n: &str| match n {
        "ASO_SERVICE_LOG_MAX_SIZE_MB" => Some("1".to_string()),
        "ASO_SERVICE_LOG_KEEP" => Some("2".to_string()),
        _ => None,
    };
    let env = aso::cli::Env {
        os: "linux",
        home: "/home/test".into(),
        vars: &vars,
        now: 7_000,
        exe: "/usr/local/bin/aso".into(),
        source: None,
    };
    let args: Vec<String> = ["logs", "rotate", &log.display().to_string()]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let done = aso::cli::run(&args, &env, &quiet(), &mut Scripted::new(false));
    assert_eq!(done.code, 0, "{done:?}");
    assert!(
        done.out.contains("fsp-mint.log is over 1 MB: moved to"),
        "{}",
        done.out
    );
    assert!(dir.join("fsp-mint.log.7000.bak").exists());
}

#[test]
fn the_service_units_rotate_the_log_before_the_program_opens_it() {
    // Fails if a unit stops running `logs rotate` before the program, or runs it after.
    let dir = scratch("unit");
    let aso = load(&dir, &config_text(&dir, "127.0.0.1:5701", "127.0.0.1:3200"));
    let files = aso::service::service_files(
        &aso,
        std::path::Path::new("/usr/local/bin/aso"),
        aso::program::Program::Fsp,
    )
    .unwrap();
    let unit = &files[1].1;
    let log = dir.join("logs/fsp-mint.log");
    let line = format!(
        "ExecStart=/bin/sh -c '/usr/local/bin/aso logs rotate {l}; exec {b} serve >>{l} 2>&1'",
        l = log.display(),
        b = dir.join("bin/fsp-mint").display()
    );
    assert!(unit.contains(&line), "{unit}");
    let env = &files[0].1;
    assert!(
        env.contains("ASO_SERVICE_LOG_MAX_SIZE_MB=50\nASO_SERVICE_LOG_KEEP=4\n"),
        "{env}"
    );
}

#[test]
fn the_logs_command_with_running_empties_the_log_in_place() {
    // Fails if `aso logs rotate --running` ignores the flag and moves the file.
    let dir = scratch("cmd-running");
    let log = dir.join("fsp-mint.log");
    big(&log, 2 * 1024 * 1024);
    let vars = |n: &str| match n {
        "ASO_SERVICE_LOG_MAX_SIZE_MB" => Some("1".to_string()),
        _ => None,
    };
    let env = aso::cli::Env {
        os: "linux",
        home: "/home/test".into(),
        vars: &vars,
        now: 8_000,
        exe: "/usr/local/bin/aso".into(),
        source: None,
    };
    let args: Vec<String> = ["logs", "rotate", "--running", &log.display().to_string()]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let done = aso::cli::run(&args, &env, &quiet(), &mut Scripted::new(false));
    assert_eq!(done.code, 0, "{done:?}");
    assert!(
        done.out.contains(&format!(
            "fsp-mint.log is over 1 MB: copied to {}/fsp-mint.log.8000.bak and emptied",
            dir.display()
        )),
        "{}",
        done.out
    );
    assert_eq!(fs::metadata(&log).unwrap().len(), 0);
}

#[test]
fn the_logs_job_caps_both_service_logs_while_they_run_as_the_service_account() {
    // Fails if the job stops running on a timer, runs as root, reads its caps
    // from somewhere other than the units' env file, or leaves a log out.
    let dir = scratch("job");
    let text = config_text(&dir, "127.0.0.1:5701", "127.0.0.1:3200");
    let exe = std::path::Path::new("/usr/local/bin/aso");
    let (fsp_log, air_log) = (
        dir.join("logs/fsp-mint.log").display().to_string(),
        dir.join("logs/air_server.log").display().to_string(),
    );
    let env_file = dir.join("etc/fsp-mint.env").display().to_string();

    let linux = aso::service::logs_files(&load_os(&dir, &text, "linux"), exe).unwrap();
    assert_eq!(linux[0].0, dir.join("units/aso-logs.service"));
    let unit = &linux[0].1;
    for want in [
        "User=aso\nGroup=aso\n".to_string(),
        format!("EnvironmentFile={env_file}\n"),
        format!("ExecStart=/usr/local/bin/aso logs rotate --running {fsp_log} {air_log}\n"),
        format!("ReadWritePaths={}\n", dir.join("logs").display()),
    ] {
        assert!(unit.contains(&want), "{want} not in {unit}");
    }
    assert!(
        linux[1].1.contains("OnUnitActiveSec=5min\n") && linux[1].1.contains("OnBootSec=5min\n"),
        "{}",
        linux[1].1
    );

    let mac = aso::service::logs_files(&load_os(&dir, &text, "macos"), exe).unwrap();
    assert_eq!(mac[0].0, dir.join("units/org.servicepoints.aso-logs.plist"));
    let plist = &mac[0].1;
    for want in [
        "<key>StartInterval</key>\n    <integer>300</integer>".to_string(),
        format!(
            "set -a; . '{env_file}'; set +a; exec '/usr/local/bin/aso' logs rotate --running {fsp_log} {air_log}"
        ),
    ] {
        assert!(plist.contains(&want), "{want} not in {plist}");
    }
}

#[test]
fn the_install_loads_the_logs_job() {
    // Fails if `aso install` writes the logs job and never loads it.
    let w = World::new("logs-job");
    let src = w.source();
    let (done, exec) = w.install(
        &["--claim-psp", "--skip-backup"],
        &src,
        &mut Scripted::new(false),
    );
    assert_eq!(done.code, 0, "{}", done.out);
    let calls = exec.calls();
    assert!(
        calls.contains(&"systemctl enable --now aso-logs.timer".to_string()),
        "{calls:#?}"
    );
    assert!(
        w.read("units/aso-logs.timer")
            .contains("OnUnitActiveSec=5min\n")
    );
}
