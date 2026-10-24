//! Asking the operator. A trait, so that the commands that confirm or ask can be
//! tested with scripted answers; [`Terminal`] is the real one.
//!
//! With no terminal (a pipe, a scheduled run) nothing can be asked: `line`
//! answers `None` and `confirm` answers its default.

use std::io::{BufRead, IsTerminal, Write};

pub trait Ask {
    /// A line of text, or `None` when none was given (no terminal, or Enter).
    fn line(&mut self, prompt: &str) -> Option<String>;
    /// A yes or no. `default` is the answer when there is no terminal or the
    /// operator just presses Enter.
    fn confirm(&mut self, prompt: &str, default: bool) -> bool;
    /// Whether anyone can answer.
    fn interactive(&self) -> bool;
}

pub struct Terminal;

fn read_line(prompt: &str) -> Option<String> {
    print!("{prompt} ");
    std::io::stdout().flush().ok()?;
    let mut text = String::new();
    std::io::stdin().lock().read_line(&mut text).ok()?;
    let text = text.trim().to_string();
    (!text.is_empty()).then_some(text)
}

impl Ask for Terminal {
    fn line(&mut self, prompt: &str) -> Option<String> {
        if !self.interactive() {
            return None;
        }
        read_line(prompt)
    }

    fn confirm(&mut self, prompt: &str, default: bool) -> bool {
        if !self.interactive() {
            return default;
        }
        let hint = if default { "[Y/n]" } else { "[y/N]" };
        match read_line(&format!("{prompt} {hint}")) {
            Some(a) if a.eq_ignore_ascii_case("y") || a.eq_ignore_ascii_case("yes") => true,
            Some(a) if a.eq_ignore_ascii_case("n") || a.eq_ignore_ascii_case("no") => false,
            _ => default,
        }
    }

    fn interactive(&self) -> bool {
        std::io::stdin().is_terminal()
    }
}
