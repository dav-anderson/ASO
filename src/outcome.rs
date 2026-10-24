//! The result of a command: the lines to print, and whether it succeeded.

#[derive(Debug, Default)]
pub struct Outcome {
    pub lines: Vec<String>,
    pub ok: bool,
}

impl Outcome {
    pub fn new() -> Self {
        Outcome {
            lines: Vec::new(),
            ok: true,
        }
    }

    pub fn say(&mut self, line: impl Into<String>) {
        self.lines.push(line.into());
    }

    pub fn fail(&mut self, line: impl Into<String>) {
        self.ok = false;
        self.lines.push(format!("FAIL: {}", line.into()));
    }
}
