//! The `aso` program: builds the real environment and prints what `cli::run`
//! returns.

use aso::ask::Terminal;
use aso::cli::{self, Env};
use aso::exec::System;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let vars = |name: &str| std::env::var(name).ok();
    let home = vars("HOME").unwrap_or_else(|| "/root".to_string());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(0))
        .unwrap_or(0);
    let exe = std::env::current_exe().unwrap_or_else(|_| "aso".into());
    let env = Env {
        os: std::env::consts::OS,
        home,
        vars: &vars,
        now,
        exe,
        source: None,
    };
    let done = cli::run(&args, &env, &System, &mut Terminal);
    print!("{}", done.out);
    eprint!("{}", done.err);
    std::process::exit(done.code);
}
