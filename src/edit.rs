//! Editing `aso.toml` in place: setting a value while keeping the comments and
//! the order of the file. The way a file is replaced (a copy to
//! `aso.toml.<epoch>.bak` first, the newest `service.config_backups` kept, the
//! new text whole and at once) is `air_interface::files::backup_and_replace`.

fn section_of(line: &str) -> Option<String> {
    let t = line.trim();
    if t.starts_with('[') && !t.starts_with("[[") {
        Some(
            t.trim_start_matches('[')
                .split(']')
                .next()
                .unwrap_or("")
                .trim()
                .to_string(),
        )
    } else {
        None
    }
}

fn quoted(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

/// `text` with `key = "value"` set in `[section]`: the line is replaced if the
/// section has one, else added under the section's header, else the section is
/// added at the end.
pub fn set_value(text: &str, section: &str, key: &str, value: &str) -> String {
    let line = format!("{key} = {}\n", quoted(value));
    let mut current = String::new();
    let mut out = String::with_capacity(text.len() + line.len());
    let mut header_at: Option<usize> = None;
    let mut done = false;
    for raw in text.split_inclusive('\n') {
        if let Some(name) = section_of(raw) {
            current = name;
            out.push_str(raw);
            if !raw.ends_with('\n') {
                out.push('\n');
            }
            if current == section && header_at.is_none() {
                header_at = Some(out.len());
            }
            continue;
        }
        let t = raw.trim();
        if !done
            && current == section
            && t.strip_prefix(key)
                .is_some_and(|rest| rest.trim_start().starts_with('='))
        {
            out.push_str(&line);
            done = true;
            continue;
        }
        out.push_str(raw);
    }
    if done {
        return out;
    }
    match header_at {
        Some(at) => {
            out.insert_str(at, &line);
            out
        }
        None => {
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&format!("[{section}]\n{line}"));
            out
        }
    }
}
