use tine_plugin_sdk::{BlockFormat, Effect, Event};

fn markdown_heading(raw: &str, level: u8) -> String {
    let mut lines = raw.split('\n').map(str::to_string).collect::<Vec<_>>();
    let body = lines[0]
        .strip_prefix("###### ").or_else(|| lines[0].strip_prefix("##### "))
        .or_else(|| lines[0].strip_prefix("#### ")).or_else(|| lines[0].strip_prefix("### "))
        .or_else(|| lines[0].strip_prefix("## ")).or_else(|| lines[0].strip_prefix("# "))
        .unwrap_or(&lines[0]);
    lines[0] = if level == 0 { body.to_string() } else { format!("{} {body}", "#".repeat(level as usize)) };
    lines.join("\n")
}

fn is_heading_property(line: &str) -> bool {
    let line = line.trim_start().to_ascii_lowercase();
    line.strip_prefix(":heading:")
        .is_some_and(|value| value.is_empty() || value.chars().next().is_some_and(char::is_whitespace))
}

fn org_heading(raw: &str, level: u8) -> String {
    let mut lines = raw.split('\n').map(str::to_string).collect::<Vec<_>>();
    let drawer_start = if lines.get(1).is_some_and(|line| line.trim().eq_ignore_ascii_case(":PROPERTIES:")) {
        Some(1)
    } else if lines.get(1).is_some_and(|line| line.is_empty())
        && lines.get(2).is_some_and(|line| line.trim().eq_ignore_ascii_case(":PROPERTIES:")) {
        Some(2)
    } else {
        None
    };
    if let Some(start) = drawer_start {
        if let Some(relative_end) = lines[start + 1..].iter().position(|line| line.trim().eq_ignore_ascii_case(":END:")) {
            let end = start + 1 + relative_end;
            let mut properties = lines[start + 1..end].iter()
                .filter(|line| !is_heading_property(line)).cloned().collect::<Vec<_>>();
            if level > 0 { properties.push(format!(":heading: {level}")); }
            if properties.is_empty() {
                lines.drain(start..=end);
            } else {
                lines.splice((start + 1)..end, properties);
            }
            return lines.join("\n");
        }
    }
    if level > 0 {
        if !lines.is_empty() && !lines.last().is_some_and(|line| line.is_empty()) { lines.push(String::new()); }
        lines.extend([":PROPERTIES:".to_string(), format!(":heading: {level}"), ":END:".to_string()]);
    }
    lines.join("\n")
}

fn handle(event: &Event) -> Result<Vec<Effect>, String> {
    if event.kind != "command" { return Ok(Vec::new()); }
    let level = event.contribution_id.as_deref().unwrap_or("").strip_prefix("heading-")
        .and_then(|value| value.parse::<u8>().ok()).filter(|value| *value <= 6)
        .ok_or_else(|| "Unknown heading-level command.".to_string())?;
    let block = event.focused_block.as_ref().ok_or_else(|| "Edit a block before choosing a heading level.".to_string())?;
    let raw = if block.format == Some(BlockFormat::Org) { org_heading(&block.raw, level) } else { markdown_heading(&block.raw, level) };
    if raw == block.raw { return Ok(Vec::new()); }
    Ok(vec![Effect::ReplaceBlockText { block_id: block.id.clone(), expected_raw: block.raw.clone(), raw }])
}

tine_plugin_sdk::tine_plugin!(handle);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rewrites_markdown() {
        assert_eq!(markdown_heading("## Title\nbody", 4), "#### Title\nbody");
        assert_eq!(markdown_heading("###### Title", 0), "Title");
        assert_eq!(markdown_heading("Title\n", 2), "## Title\n");
    }
    #[test]
    fn preserves_org_properties() {
        let raw = "Title\n:PROPERTIES:\n:id: abc\n:heading: 2\n:END:";
        assert_eq!(org_heading(raw, 5), "Title\n:PROPERTIES:\n:id: abc\n:heading: 5\n:END:");
        assert_eq!(org_heading(raw, 0), "Title\n:PROPERTIES:\n:id: abc\n:END:");
        assert_eq!(org_heading("Title\n\n:PROPERTIES:\n:heading: 1\n:END:\n", 0), "Title\n\n");
        assert_eq!(org_heading("Title\n:PROPERTIES:\n:heading:note: keep\n:heading: 1\n:END:", 0), "Title\n:PROPERTIES:\n:heading:note: keep\n:END:");
        let literal = "Title\nbody\n:PROPERTIES:\n:heading: 1\n:END:";
        assert_eq!(org_heading(literal, 0), literal);
    }
}
