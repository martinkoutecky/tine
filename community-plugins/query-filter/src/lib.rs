use tine_plugin_sdk::{Effect, Event};

const FILTER: &str = "status != \"done\"";

fn is_query_view(raw: &str) -> bool {
    raw.contains("{{query")
        && raw
            .lines()
            .any(|line| matches!(line.trim(), "tine.view:: table" | "tine.view:: board"))
}

fn add_filter(raw: &str) -> String {
    if raw
        .lines()
        .any(|line| line.trim() == format!("tine.filter:: {FILTER}"))
    {
        return raw.to_string();
    }
    if raw.is_empty() {
        format!("tine.filter:: {FILTER}")
    } else {
        format!("{raw}\ntine.filter:: {FILTER}")
    }
}

fn handle(event: &Event) -> Result<Vec<Effect>, String> {
    if event.kind != "command" || event.contribution_id.as_deref() != Some("hide-completed") {
        return Ok(Vec::new());
    }
    let block = event.focused_block.as_ref().ok_or_else(|| {
        "Edit the query table or board block before running this command.".to_string()
    })?;
    if !is_query_view(&block.raw) {
        return Err(
            "The focused block is not a query table or board; nothing was changed.".to_string(),
        );
    }
    let raw = add_filter(&block.raw);
    if raw == block.raw {
        return Ok(vec![tine_plugin_sdk::notice(
            "This view already hides completed rows.",
        )]);
    }
    Ok(vec![Effect::ReplaceBlockText {
        block_id: block.id.clone(),
        expected_raw: block.raw.clone(),
        raw,
    }])
}

tine_plugin_sdk::tine_plugin!(handle);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_once_without_rewriting_existing_content() {
        assert_eq!(
            add_filter("{{query (task TODO)}}\ntine.view:: table"),
            "{{query (task TODO)}}\ntine.view:: table\ntine.filter:: status != \"done\""
        );
        assert_eq!(add_filter(&add_filter("Query")), add_filter("Query"));
        assert!(is_query_view("{{query (task TODO)}}\ntine.view:: board"));
        assert!(!is_query_view("ordinary prose"));
        assert!(!is_query_view("{{query (task TODO)}}"));
    }
}
