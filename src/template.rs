//! Service files are text templates with `{{placeholder}}`s. The values that go
//! into one are checked first: a value with a space, a quote or a `%` would
//! change what the file means.

/// A value that may go into a unit, a property list and a shell word as it is.
pub fn plain(what: &str, value: &str) -> Result<(), String> {
    let ok = !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._+@:=,-".contains(c));
    if ok {
        Ok(())
    } else {
        Err(format!(
            "{what} is `{value}`: in a service file it may hold only letters, digits and / . _ + @ : = , - (no spaces, quotes, % or $). Use a path without them"
        ))
    }
}

fn xml_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

/// `template` with each `{{key}}` replaced by its value in `vars`. With `xml`
/// the values are escaped, except those of keys that end in `_xml`.
pub fn expand(
    what: &str,
    template: &str,
    vars: &[(&str, String)],
    xml: bool,
) -> Result<String, String> {
    let mut out = String::with_capacity(template.len() + 256);
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            return Err(format!("{what}: a `{{{{` is never closed with `}}}}`"));
        };
        let key = after[..end].trim();
        let Some((_, value)) = vars.iter().find(|(k, _)| *k == key) else {
            let known: Vec<&str> = vars.iter().map(|(k, _)| *k).collect();
            return Err(format!(
                "{what}: unknown placeholder {{{{{key}}}}} (known here: {})",
                known.join(", ")
            ));
        };
        if xml && !key.ends_with("_xml") {
            out.push_str(&xml_escape(value));
        } else {
            out.push_str(value);
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    Ok(out)
}
