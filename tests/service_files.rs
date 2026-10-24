//! The service definitions: systemd units and launchd property lists, restart
//! on failure, the start order, and the nginx file.

mod common;

use std::path::Path;

use aso::program::Program;
use aso::service;
use common::*;

const EXE: &str = "/usr/local/bin/aso";

fn files(os: &str, program: Program) -> (std::path::PathBuf, Vec<(std::path::PathBuf, String)>) {
    let dir = scratch("svc");
    let aso = load_os(
        &dir,
        &config_text(&dir, "127.0.0.1:5701", "127.0.0.1:3200"),
        os,
    );
    let f = service::service_files(&aso, Path::new(EXE), program).unwrap();
    (dir, f)
}

#[test]
fn the_systemd_units_restart_on_failure_and_fsp_mint_starts_after_air_server() {
    // Fails if either unit stops restarting itself, fsp-mint stops waiting for
    // air_server, or the stop times differ from 30 s (air_server) and 120 s (fsp-mint).
    let (_, air) = files("linux", Program::Air);
    let (_, fsp) = files("linux", Program::Fsp);
    let (air, fsp) = (&air[1].1, &fsp[1].1);
    for unit in [air, fsp] {
        assert!(
            unit.contains("Restart=on-failure\nRestartSec=5\n"),
            "{unit}"
        );
        assert!(unit.contains("StartLimitIntervalSec=0\n"), "{unit}");
        assert!(unit.contains("WantedBy=multi-user.target\n"), "{unit}");
        assert!(unit.contains("User=aso\nGroup=aso\n"), "{unit}");
    }
    assert!(
        air.contains("After=network-online.target\n") && !air.contains("aso-air.service"),
        "{air}"
    );
    assert!(fsp.contains("After=network-online.target aso-air.service\nWants=network-online.target aso-air.service\n"), "{fsp}");
    assert!(air.contains("TimeoutStopSec=30\n") && fsp.contains("TimeoutStopSec=120\n"));
    assert!(fsp.contains("ExecReload=/bin/kill -HUP $MAINPID"), "{fsp}");
    assert!(!air.contains("ExecReload"), "air_server has no reload");
}

#[test]
fn air_server_gets_its_settings_as_environment_variables_with_loopback_sockets() {
    // Fails if air_server is told to listen on all interfaces, or the storage is lost.
    let (dir, f) = files("linux", Program::Air);
    let env = &f[0].1;
    assert!(
        env.contains(&format!(
            "AIR_STORAGE={}/data/air/STORAGE.db\n",
            dir.display()
        )),
        "{env}"
    );
    assert!(
        env.contains("AIR_WS_ADDR=127.0.0.1:5702\nAIR_HEALTH_ADDR=127.0.0.1:5701\n"),
        "{env}"
    );
    assert!(!env.contains("AIR_TLS"), "{env}");
}

#[test]
fn the_launchd_jobs_are_kept_alive_and_named_for_the_service() {
    // Fails if a macOS job is not restarted after a crash, or is named otherwise.
    let (dir, f) = files("macos", Program::Fsp);
    assert_eq!(f[1].0, dir.join("units/org.servicepoints.aso-fsp.plist"));
    let plist = &f[1].1;
    assert!(
        plist.contains("<key>Label</key>\n    <string>org.servicepoints.aso-fsp</string>"),
        "{plist}"
    );
    assert!(
        plist.contains(
            "<key>KeepAlive</key>\n    <dict>\n        <key>SuccessfulExit</key>\n        <false/>"
        ),
        "{plist}"
    );
    assert!(
        plist.contains("<key>ExitTimeOut</key>\n    <integer>120</integer>"),
        "{plist}"
    );
    assert!(
        plist.contains("<key>UserName</key>\n    <string>operator</string>"),
        "{plist}"
    );
    assert!(
        plist.contains(&format!(
            "'{EXE}' logs rotate '{}/logs/fsp-mint.log'",
            dir.display()
        )),
        "{plist}"
    );
}

#[test]
fn the_service_manager_commands_differ_by_os_and_start_air_server_first() {
    // Fails if macOS gets systemctl, a reload becomes a restart, or stop does not disable.
    let dir = scratch("cmds");
    let text = config_text(&dir, "127.0.0.1:5701", "127.0.0.1:3200");
    let linux = load_os(&dir, &text, "linux");
    let mac = load_os(&dir, &text, "macos");
    use aso::ctl::{Action, commands};
    let line = |c: &aso::ctl::Cmd| format!("{} {}", c.0, c.1.join(" "));
    let l = |a| {
        commands(&linux, Program::Fsp, a)
            .iter()
            .map(line)
            .collect::<Vec<_>>()
    };
    let m = |a| {
        commands(&mac, Program::Fsp, a)
            .iter()
            .map(line)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        l(Action::Start),
        vec!["systemctl enable --now aso-fsp.service"]
    );
    assert_eq!(
        l(Action::Stop),
        vec!["systemctl disable --now aso-fsp.service"]
    );
    assert_eq!(
        l(Action::Restart),
        vec!["systemctl restart aso-fsp.service"]
    );
    assert_eq!(l(Action::Reload), vec!["systemctl reload aso-fsp.service"]);
    assert_eq!(
        m(Action::Start),
        vec![
            format!(
                "launchctl bootstrap system {}/units/org.servicepoints.aso-fsp.plist",
                dir.display()
            ),
            "launchctl kickstart system/org.servicepoints.aso-fsp".to_string()
        ]
    );
    assert_eq!(
        m(Action::Stop),
        vec!["launchctl bootout system/org.servicepoints.aso-fsp"]
    );
    assert_eq!(
        m(Action::Restart),
        vec!["launchctl kickstart -k system/org.servicepoints.aso-fsp"]
    );
    assert_eq!(
        m(Action::Reload),
        vec!["launchctl kill HUP system/org.servicepoints.aso-fsp"]
    );
}

#[test]
fn the_nginx_file_forwards_to_air_server_on_the_public_name() {
    // Fails if the public name, the certificate or the forward are not in the file,
    // and that no file is written when there is no public name.
    let dir = scratch("nginx");
    let text = with_air(
        &config_text(&dir, "127.0.0.1:5701", "127.0.0.1:3200"),
        "public_name = \"air.example.org\"\npublic_cert = \"/etc/ssl/a.pem\"\npublic_key = \"/etc/ssl/a.key\"\nws_addr = \"127.0.0.1:5800\"",
    );
    let aso = load(&dir, &text);
    let (path, conf) = service::nginx_file(&aso).unwrap().unwrap();
    assert_eq!(path, dir.join("etc/nginx-air.conf"));
    for want in [
        "server_name air.example.org;",
        "ssl_certificate /etc/ssl/a.pem;",
        "ssl_certificate_key /etc/ssl/a.key;",
        "proxy_pass http://127.0.0.1:5800;",
        "proxy_set_header Upgrade $http_upgrade;",
    ] {
        assert!(conf.contains(want), "{want}: {conf}");
    }
    let plain = load(&dir, &config_text(&dir, "127.0.0.1:5701", "127.0.0.1:3200"));
    assert_eq!(service::nginx_file(&plain).unwrap(), None);
}

#[test]
fn a_path_a_service_file_cannot_hold_safely_is_refused() {
    // Fails if a path with a space or a quote is written into a unit.
    let dir = scratch("quote");
    // aso.toml refuses the path when it loads, before any unit is written.
    let text = format!(
        "[service]\nlog_dir = \"{}/my logs\"\nunit_dir = \"{}/u\"\n",
        dir.display(),
        dir.display()
    );
    let err = try_load(&dir, Some(&text), &[]).unwrap_err().join("\n");
    assert!(err.contains("my logs"), "{err}");
}
