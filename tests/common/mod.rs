//! Helpers for the tests: a stub HTTP server for the health listeners, a fake
//! for the programs aso runs, a release source that serves from memory, scripted
//! answers for the questions aso asks, and a config text for a scratch folder.

// Each test file uses a different part of these helpers.
#![allow(dead_code)]

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use air_minterface::Managed;
use air_minterface::files::sha256_file;
use air_minterface::release::{Download, Source};
use aso::ask::Ask;
use aso::cli::{Done, Env};
use aso::config::{Aso, Inputs};
use aso::exec::{Exec, Ran};
use aso::install::Ctx;
use aso::program::Program;

/// A folder of its own for every call.
pub fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("aso-test-{}-{n}-{name}", std::process::id()));
    std::fs::remove_dir_all(&d).ok();
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Serve on a free loopback port until the process ends: every request gets
/// `status` and `body`. Returns `127.0.0.1:port`.
pub fn health_stub(status: u16, body: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buf = [0u8; 2048];
            let _read = stream.read(&mut buf);
            let reply = format!(
                "HTTP/1.0 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(reply.as_bytes()).ok();
        }
    });
    addr
}

/// fsp-mint's `/health` answering ok.
pub fn fsp_up() -> String {
    health_stub(200, r#"{"status":"ok"}"#)
}

/// air_server's health listener answering its bare `ok`.
pub fn air_up() -> String {
    health_stub(200, "ok")
}

/// An address nothing listens on.
pub fn nobody() -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let a = l.local_addr().unwrap().to_string();
    drop(l);
    a
}

pub const ESP_DID: &str = "02cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
pub const AIR_NAME: &str = "03bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
pub const DID: &str = "02cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

/// An aso.toml for the scratch folder `dir`: the units go to `dir/units`, the
/// health listeners to the given addresses, air_server is the file
/// `dir/in/air_server`, and the three settings fsp-mint needs are set.
pub fn config_text(dir: &Path, air: &str, fsp: &str) -> String {
    let d = dir.display();
    format!(
        "[service]\nunit_dir = \"{d}/units\"\nstart_wait_secs = 1\n\n[air]\nhealth_addr = \"{air}\"\nbin = \"{d}/in/air_server\"\nversion = \"1.0.1\"\n\n[fsp.server]\nbind = \"{fsp}\"\n\n[fsp.air]\nesp_did = \"{ESP_DID}\"\nesp_server = \"{AIR_NAME}\"\nesp_host = \"203.0.113.9\"\n"
    )
}

/// The file air_server is installed from.
pub fn put_air_binary(dir: &Path) {
    std::fs::create_dir_all(dir.join("in")).unwrap();
    std::fs::write(dir.join("in").join("air_server"), "air_server build 1.0.1").unwrap();
}

pub fn try_load(dir: &Path, text: Option<&str>, env: &[(&str, &str)]) -> Result<Aso, Vec<String>> {
    let env: Vec<(String, String)> = env
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let lookup = move |name: &str| env.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone());
    Aso::load(&Inputs {
        dir: dir.to_path_buf(),
        file: text,
        env: &lookup,
        home: "/home/test".into(),
        os: "linux",
    })
}

pub fn load(dir: &Path, text: &str) -> Aso {
    try_load(dir, Some(text), &[]).unwrap()
}

type Answer = Box<dyn Fn(&str, &[String]) -> Result<Ran, String> + Send + Sync>;

/// The programs aso runs, answered by a closure. Every call is kept as one line:
/// `bin arg arg`, with the extra environment as `KEY=value` words after ` env `.
pub struct Fake {
    answer: Answer,
    pub calls: Mutex<Vec<String>>,
}

impl Exec for Fake {
    fn run(&self, bin: &str, args: &[String], env: &[(String, String)]) -> Result<Ran, String> {
        let mut line = format!("{bin} {}", args.join(" "));
        if !env.is_empty() {
            let words: Vec<String> = env.iter().map(|(k, v)| format!("{k}={v}")).collect();
            line.push_str(&format!(" env {}", words.join(" ")));
        }
        self.calls.lock().unwrap().push(line);
        (self.answer)(bin, args)
    }
}

impl Fake {
    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

pub fn fake<F>(answer: F) -> Fake
where
    F: Fn(&str, &[String]) -> Result<Ran, String> + Send + Sync + 'static,
{
    Fake {
        answer: Box::new(answer),
        calls: Mutex::new(Vec::new()),
    }
}

pub fn ran(code: i32, stdout: &str, stderr: &str) -> Result<Ran, String> {
    Ok(Ran {
        code: Some(code),
        stdout: stdout.to_string(),
        stderr: stderr.to_string(),
    })
}

/// A machine where every program exits 0 and prints nothing.
pub fn quiet() -> Fake {
    fake(|_, _| ran(0, "", ""))
}

/// The commands that would stop or restart air_server or fsp-mint (the timers
/// of the upgrade and the redeem are not services).
pub fn stops(calls: &[String]) -> Vec<String> {
    calls
        .iter()
        .filter(|c| {
            let verb = c.starts_with("systemctl stop")
                || c.starts_with("systemctl restart")
                || c.starts_with("systemctl disable")
                || c.starts_with("launchctl bootout")
                || c.starts_with("launchctl kickstart -k");
            verb && (c.contains("aso-air") || c.contains("aso-fsp"))
        })
        .cloned()
        .collect()
}

/// A release source that serves from memory.
pub struct FakeSource {
    pub tags: Vec<(Program, Result<Vec<String>, String>)>,
    /// (program, version, the file's text, a SHA-256 to publish instead).
    pub builds: Vec<(Program, String, String, Option<String>)>,
    pub fetched: Mutex<Vec<String>>,
}

impl FakeSource {
    pub fn new() -> FakeSource {
        FakeSource {
            tags: Vec::new(),
            builds: Vec::new(),
            fetched: Mutex::new(Vec::new()),
        }
    }

    pub fn with(mut self, program: Program, tags: &[&str]) -> FakeSource {
        self.tags
            .push((program, Ok(tags.iter().map(|t| t.to_string()).collect())));
        self
    }

    pub fn build(mut self, program: Program, version: &str, text: &str) -> FakeSource {
        self.builds
            .push((program, version.to_string(), text.to_string(), None));
        self
    }
}

impl Source<Program> for FakeSource {
    fn tags(&self, program: Program) -> Result<Vec<String>, String> {
        match self.tags.iter().find(|(p, _)| *p == program) {
            Some((_, r)) => r.clone(),
            None => Err(format!("{} has no releases here", program.name())),
        }
    }

    fn fetch(&self, program: Program, version: &str, dir: &Path) -> Result<Download, String> {
        self.fetched
            .lock()
            .unwrap()
            .push(format!("{} {version}", program.name()));
        let Some((_, _, text, sha)) = self
            .builds
            .iter()
            .find(|(p, v, _, _)| *p == program && v == version)
        else {
            return Err(format!("no build of {} {version}", program.name()));
        };
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let file = dir.join(program.name());
        std::fs::write(&file, text).map_err(|e| e.to_string())?;
        let sha256 = match sha {
            Some(s) => s.clone(),
            None => sha256_file(&file)?,
        };
        Ok(Download { file, sha256 })
    }
}

/// Answers to what aso asks, in order, and what it asked.
pub struct Scripted {
    pub lines: VecDeque<Option<String>>,
    pub yes: VecDeque<bool>,
    pub asked: Vec<String>,
    pub terminal: bool,
}

impl Scripted {
    pub fn new(terminal: bool) -> Scripted {
        Scripted {
            lines: VecDeque::new(),
            yes: VecDeque::new(),
            asked: Vec::new(),
            terminal,
        }
    }

    pub fn saying(mut self, answers: &[bool]) -> Scripted {
        self.yes = answers.iter().copied().collect();
        self
    }
}

impl Ask for Scripted {
    fn line(&mut self, prompt: &str) -> Option<String> {
        self.asked.push(prompt.to_string());
        self.lines.pop_front().flatten()
    }

    fn confirm(&mut self, prompt: &str, default: bool) -> bool {
        self.asked.push(prompt.to_string());
        self.yes.pop_front().unwrap_or(default)
    }

    fn interactive(&self) -> bool {
        self.terminal
    }
}

pub const NOW: i64 = 1_792_000_000;

pub fn ctx<'a>(aso: &'a Aso, exec: &'a dyn Exec, source: &'a dyn Source<Program>) -> Ctx<'a> {
    Ctx {
        aso,
        exec,
        source,
        exe: PathBuf::from("/usr/local/bin/aso"),
        config: aso.dir.join("aso.toml"),
        now: NOW,
    }
}

/// Run a command line as `aso` would, against a folder with `text` as its
/// aso.toml.
pub fn cli(
    dir: &Path,
    text: &str,
    args: &[&str],
    exec: &dyn Exec,
    source: &dyn Source<Program>,
    ask: &mut dyn Ask,
) -> Done {
    cli_at(NOW, dir, text, args, exec, source, ask)
}

/// [`cli`] at another time.
pub fn cli_at(
    now: i64,
    dir: &Path,
    text: &str,
    args: &[&str],
    exec: &dyn Exec,
    source: &dyn Source<Program>,
    ask: &mut dyn Ask,
) -> Done {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("aso.toml"), text).unwrap();
    let vars = |_: &str| None;
    let env = Env {
        os: "linux",
        home: "/home/test".into(),
        vars: &vars,
        now,
        exe: PathBuf::from("/usr/local/bin/aso"),
        source: Some(source),
    };
    let mut all = vec!["--dir".to_string(), dir.display().to_string()];
    all.extend(args.iter().map(|a| a.to_string()));
    aso::cli::run(&all, &env, exec, ask)
}

/// aso.toml with `extra` added at the start of its `[air]` section.
pub fn with_air(text: &str, extra: &str) -> String {
    text.replacen("[air]\n", &format!("[air]\n{extra}\n"), 1)
}

/// A machine for the install tests: scratch folder, stub health listeners that
/// answer ok, and programs that behave like the real ones on the points aso
/// reads (see `answer`).
pub struct World {
    pub dir: PathBuf,
    pub air: String,
    pub fsp: String,
    /// The PSP the wallet holds.
    pub psp: std::sync::Arc<AtomicU64>,
}

impl World {
    pub fn new(name: &str) -> World {
        let dir = scratch(name);
        put_air_binary(&dir);
        World {
            dir,
            air: air_up(),
            fsp: fsp_up(),
            psp: std::sync::Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn text(&self) -> String {
        config_text(&self.dir, &self.air, &self.fsp)
    }

    pub fn aso(&self) -> Aso {
        load(&self.dir, &self.text())
    }

    pub fn source(&self) -> FakeSource {
        FakeSource::new()
            .with(Program::Fsp, &["v0.8.0", "v0.10.0"])
            .build(Program::Fsp, "0.10.0", "fsp-mint build 0.10.0")
            .with(Program::Wallet, &["v0.8.0", "v0.9.0"])
            .build(Program::Wallet, "0.9.0", "ecash-wallet build 0.9.0")
    }

    /// The programs: `id` knows the account; `fsp-mint service-key` makes the key
    /// file and prints the DID; starting air_server writes its name to its log;
    /// the wallet answers `--version`, `balance` (from `psp`), and funds itself
    /// on `claim-psp` (100 PSP) and `handoff import` (80 PSP).
    pub fn exec(&self) -> Fake {
        let dir = self.dir.clone();
        let psp = std::sync::Arc::clone(&self.psp);
        fake(move |bin, args| {
            let name = Path::new(bin)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let has = |w: &str| args.iter().any(|a| a == w);
            match name.as_str() {
                "fsp-mint" if has("service-key") => {
                    let key = dir.join("data/fsp/main/service_key.hex");
                    std::fs::create_dir_all(key.parent().unwrap()).unwrap();
                    std::fs::write(&key, "11".repeat(32)).unwrap();
                    ran(
                        0,
                        &format!("made {}\nservice.did = {DID}\n", key.display()),
                        "",
                    )
                }
                "fsp-mint" if has("audit") && has("--json") => {
                    ran(0, r#"{"consistent":true,"warnings":[]}"#, "")
                }
                "fsp-mint" if has("audit") => ran(0, "issued 10\nspent 4\n", ""),
                "ecash-wallet" if args == ["--version"] => ran(
                    0,
                    "ecash-wallet 0.9.0\nsp-wire 0.7.0\nmint-core 0.3.0\nair 1.0.1\n",
                    "",
                ),
                "ecash-wallet" if args.last().map(String::as_str) == Some("balance") => ran(
                    0,
                    &format!("psp {}\nesp 0\nfsp 0\n", psp.load(Ordering::SeqCst)),
                    "",
                ),
                "ecash-wallet" if has("claim-psp") => {
                    psp.store(100, Ordering::SeqCst);
                    ran(0, "claimed 100 PSP\n", "")
                }
                "ecash-wallet" if has("handoff") && has("import") => {
                    psp.store(80, Ordering::SeqCst);
                    ran(0, "imported\n", "")
                }
                "ecash-wallet" if has("buy-psp") => ran(0, "a1b2c3d4payload\n", ""),
                "systemctl" if args == ["enable", "--now", "aso-air.service"] => {
                    let log = dir.join("logs/air_server.log");
                    std::fs::create_dir_all(log.parent().unwrap()).unwrap();
                    std::fs::write(&log, format!("air_server 1.0.1\nAIR server: {AIR_NAME}\n"))
                        .unwrap();
                    ran(0, "", "")
                }
                _ => ran(0, "", ""),
            }
        })
    }

    /// Run `aso install ARGS` against this world.
    pub fn install(&self, args: &[&str], source: &FakeSource, ask: &mut Scripted) -> (Done, Fake) {
        let exec = self.exec();
        let mut all = vec!["install"];
        all.extend(args);
        let done = cli(&self.dir, &self.text(), &all, &exec, source, ask);
        (done, exec)
    }
}

/// The index of the first call that starts with `prefix`, or panic with the calls.
pub fn at(calls: &[String], prefix: &str) -> usize {
    calls
        .iter()
        .position(|c| c.starts_with(prefix))
        .unwrap_or_else(|| panic!("no call starts with `{prefix}` in {calls:#?}"))
}

impl World {
    /// Install everything (wallet funded, step d skipped) and return the source
    /// that served it.
    pub fn installed(name: &str) -> (World, FakeSource) {
        let w = World::new(name);
        let src = w.source();
        let (done, _) = w.install(
            &["--claim-psp", "--skip-backup"],
            &src,
            &mut Scripted::new(false),
        );
        assert_eq!(done.code, 0, "{}", done.out);
        (w, src)
    }

    /// Run `aso ARGS` against this world with a fresh fake.
    pub fn run(&self, args: &[&str], source: &FakeSource, ask: &mut Scripted) -> (Done, Fake) {
        let exec = self.exec();
        let done = cli(&self.dir, &self.text(), args, &exec, source, ask);
        (done, exec)
    }

    pub fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.dir.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
    }
}

/// A source with fsp-mint 0.10.0 and 0.11.0 (and a release candidate that must
/// never be chosen) and the wallet at 0.9.0.
pub fn newer_source() -> FakeSource {
    FakeSource::new()
        .with(Program::Fsp, &["v0.10.0", "v0.11.0", "v0.12.0-rc1"])
        .build(Program::Fsp, "0.10.0", "fsp-mint build 0.10.0")
        .build(Program::Fsp, "0.11.0", "fsp-mint build 0.11.0")
        .build(Program::Fsp, "0.12.0-rc1", "fsp-mint build 0.12.0-rc1")
        .with(Program::Wallet, &["v0.9.0"])
        .build(Program::Wallet, "0.9.0", "ecash-wallet build 0.9.0")
}

/// [`load`] for another operating system (`linux` or `macos`).
pub fn load_os(dir: &Path, text: &str, os: &str) -> Aso {
    let lookup = |name: &str| (name == "SUDO_USER").then(|| "operator".to_string());
    Aso::load(&Inputs {
        dir: dir.to_path_buf(),
        file: Some(text),
        env: &lookup,
        home: "/Users/operator".into(),
        os,
    })
    .unwrap()
}
